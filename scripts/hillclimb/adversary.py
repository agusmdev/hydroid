#!/usr/bin/env python3
"""Black-box adversary: asks Claude for new cases that hydroid gets wrong, checks each one against
hydroid *and* the runtime oracle (check.py), sends the verdicts back, and lets the model revise.
Only cases the runtime proves hydroid wrong on (verdict BREAKS) are kept; UNVERIFIED ones are
kept for a reviewer; the rest are deleted.

The adversary never sees hydroid's source: it gets the README, the catalog, the existing cases,
and what check.py says about its own proposals. It attacks behavior, not the implementation.

Proposals land in tests/adversarial/candidates/<name>/ (app.py + RATIONALE.md + CHECK.json);
promote.py moves the proven ones into the suite. The headless counterpart of the adversary step
in the /hydroid-self-improve skill (which runs adversaries as Claude Code subagents instead).

Needs the Anthropic SDK (`uv run --with anthropic scripts/hillclimb/adversary.py`) and
credentials (ANTHROPIC_API_KEY, or a profile from `ant auth login`).

Usage: adversary.py [--count 5] [--rounds 3] [--theme "ORM sessions"]
"""

import argparse
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

import anthropic

ROOT = Path(__file__).resolve().parents[2]
CASES = ROOT / "tests/adversarial/cases"
CANDIDATES = ROOT / "tests/adversarial/candidates"
CATALOG = ROOT / "crates/hydroid_core/catalog.toml"
README = ROOT / "README.md"
CHECK = Path(__file__).resolve().parent / "check.py"
MODEL = "claude-opus-5-5"

SYSTEM = """You are an adversarial reviewer for hydroid, a static analyzer (Rust, built on ty's \
type inference) that finds blocking calls reachable on the asyncio event loop in FastAPI code.

You write small, self-contained FastAPI test projects (one app.py each) that hydroid gets wrong.
A good case is realistic: code a FastAPI team could plausibly ship, not an obfuscation contest.
Prefer patterns that are common in real codebases (ORMs, HTTP clients, caches, settings loaders,
dependency injection, validation, file handling) over exotic metaprogramming.

Expectation syntax, as a trailing comment on the line where the diagnostic must appear: the call
site in the innermost function that runs on the loop (in an `async def`, the line that calls into
the sync code that blocks; in a sync callback the loop calls directly, like a startup handler, the
blocking call itself):
  # expect: requests.api.get      one diagnostic whose blocking function has this defining
                                  qualified name (the module where it is defined, as in the
                                  catalog: requests.get is requests.api.get)
  # expect: *                     one diagnostic, any blocking function (when the qualified name
                                  is uncertain)
Lines without an expectation must produce no diagnostic: also include near-miss negatives (async
look-alikes, offloaded work, cheap sync code) next to the positives.

Only use these third-party libraries (they are installed in the test environment): fastapi,
starlette, pydantic, requests, httpx, sqlalchemy, sqlmodel, redis, pymysql, anyio, tenacity,
bcrypt, python-multipart; plus the standard library.

Every case is checked by running it. A runtime oracle imports app.py, finds the FastAPI app,
runs its lifespan, calls every route and websocket once with sample inputs built from the OpenAPI
schema (strings "1", integers 1, ...), and records every catalog function called on the event loop
thread. On the loop, blocking calls do not do their real work (they return a stand-in value that
accepts any use), so later lines still run; off the loop, network and process starts fail at once.
So:
- app.py must import cleanly and define the app at module level; no real servers are reachable.
- The lines you label must actually run when each route is called once with those inputs (do not
  hide them behind conditions the sample inputs do not meet, or behind exceptions).
- A false-positive claim (hydroid reports a line you leave unlabeled) is proven only if that line
  runs on the loop and nothing blocks there.

Every case must be a correct claim about runtime behavior: before marking a line, check that the
blocking call really executes synchronously on the event loop thread (sync FastAPI endpoints and
sync dependencies run in a threadpool; Starlette iterates sync iterators of StreamingResponse in
a threadpool; calling a generator function runs none of its body)."""

SCHEMA = {
    "type": "object",
    "properties": {
        "cases": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "snake_case directory name"},
                    "rationale": {
                        "type": "string",
                        "description": "why hydroid likely misses it and why the expectation is right",
                    },
                    "app_py": {"type": "string"},
                },
                "required": ["name", "rationale", "app_py"],
                "additionalProperties": False,
            },
        }
    },
    "required": ["cases"],
    "additionalProperties": False,
}


def existing_cases() -> str:
    parts = []
    for path in sorted(CASES.glob("*/app.py")):
        parts.append(f"### {path.parent.name}\n```python\n{path.read_text()}```")
    return "\n\n".join(parts)


def pending_candidates() -> str:
    if not CANDIDATES.exists():
        return "(none)"
    names = sorted(p.name for p in CANDIDATES.iterdir() if p.is_dir())
    return ", ".join(names) or "(none)"


def safe_name(raw: str, taken: set[str]) -> str:
    name = re.sub(r"[^a-z0-9_]", "_", raw.lower()) or "case"
    while name in taken:
        name += "_2"
    return name


def check(dirs: list[Path]) -> dict[str, dict]:
    proc = subprocess.run([sys.executable, str(CHECK), "--json", *map(str, dirs)],
                          capture_output=True, text=True)
    out = {}
    for line in proc.stdout.splitlines():
        row = json.loads(line)
        out[row["case"]] = row
    if proc.returncode:
        sys.exit(f"check.py failed: {proc.stderr.strip()}")
    return out


def feedback(result: dict) -> str:
    lines = [f"{result['case']}: {result['verdict']}"]
    for ln in result["lines"]:
        lines.append(f"  {ln['path']}:{ln['line']} label={' '.join(ln['label']) or '-'} "
                     f"hydroid={' '.join(ln['hydroid']) or '-'} runtime={' '.join(ln['runtime']) or '-'} "
                     f"| hydroid: {ln['hydroid_status']} | runtime: {ln['runtime_status']}")
    lines += [f"  oracle: {e[:300]}" for e in result["errors"]]
    return "\n".join(lines)


def ask(client: anthropic.Anthropic, messages: list) -> list[dict]:
    response = client.beta.messages.create(
        model=MODEL,
        max_tokens=16000,
        betas=["server-side-fallback-2026-07-01"],
        fallbacks="default",
        thinking={"type": "adaptive"},
        output_config={"effort": "high", "format": {"type": "json_schema", "schema": SCHEMA}},
        system=SYSTEM,
        messages=messages,
    )
    if response.stop_reason == "refusal":
        sys.exit(f"refused: {response.stop_details}")
    if response.stop_reason == "max_tokens":
        sys.exit("output truncated (max_tokens); retry with a smaller --count")
    u = response.usage
    print(f"model {response.model}  input {u.input_tokens}  output {u.output_tokens}", file=sys.stderr)
    # Append-only history: the full content (thinking blocks included) goes back unchanged.
    messages.append({"role": "assistant", "content": response.content})
    text = next(b.text for b in response.content if b.type == "text")
    return json.loads(text)["cases"]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--count", type=int, default=5)
    parser.add_argument("--rounds", type=int, default=3, help="propose + up to N-1 revisions")
    parser.add_argument("--theme", help="area to attack (e.g. 'SQLAlchemy sessions', 'middleware')")
    args = parser.parse_args()

    theme = f"\nFocus on this area: {args.theme}." if args.theme else ""
    prompt = f"""<readme>
{README.read_text()}
</readme>

<catalog>
{CATALOG.read_text()}
</catalog>

<existing_cases>
{existing_cases()}
</existing_cases>

<pending_candidates>
{pending_candidates()}
</pending_candidates>

Write {args.count} new cases that hydroid most likely gets wrong today. Each must exercise a
mechanism the existing cases and pending candidates do not already cover (a different language
feature, library API or FastAPI behavior), not a renamed variant of one.{theme}"""

    client = anthropic.Anthropic()
    messages: list = [{"role": "user", "content": prompt}]
    CANDIDATES.mkdir(parents=True, exist_ok=True)
    taken = {p.name for p in CASES.iterdir()} | {p.name for p in CANDIDATES.iterdir()}
    open_cases: dict[str, Path] = {}  # name the model uses -> candidate dir
    results: dict[str, dict] = {}
    for round_no in range(args.rounds):
        proposals = ask(client, messages)
        for case in proposals:
            out = open_cases.get(case["name"])
            if out is None:
                out = CANDIDATES / safe_name(case["name"], taken)
                taken.add(out.name)
                open_cases[case["name"]] = out
                out.mkdir(parents=True, exist_ok=True)
            (out / "app.py").write_text(case["app_py"].rstrip() + "\n")
            (out / "RATIONALE.md").write_text(case["rationale"].strip() + "\n")
        checked = check([open_cases[c["name"]] for c in proposals])
        for model_name, out in open_cases.items():
            if out.name in checked:
                results[model_name] = checked[out.name]
                (out / "CHECK.json").write_text(json.dumps(checked[out.name], indent=2) + "\n")
        for name, r in results.items():
            print(f"round {round_no + 1}: {r['verdict']:10} {open_cases[name].name}")
        todo = {n: r for n, r in results.items() if r["verdict"] != "BREAKS"}
        if not todo or round_no == args.rounds - 1:
            break
        report = "\n\n".join(feedback(r) for r in todo.values())
        messages.append({"role": "user", "content": f"""check.py results for your cases that are not
proven yet (BREAKS = proven; PASSES = hydroid already gets it right; INVALID = a label is wrong;
UNVERIFIED = the oracle could not reach or prove a disputed line):

{report}

Return revised versions of these cases only, under the same names. For PASSES, attack the
mechanism differently or drop the case (return it with a new idea under the same name). For
INVALID, fix the labels: the runtime is the ground truth. For UNVERIFIED, make the disputed lines
run when each route is called once with sample inputs."""})

    kept = 0
    for name, r in results.items():
        out = open_cases[name]
        if r["verdict"] in ("BREAKS", "UNVERIFIED"):
            kept += 1
            print(f"kept {r['verdict']:10} {out.relative_to(ROOT)}")
        else:
            shutil.rmtree(out)
    print(f"{kept} candidates kept; promote proven ones with scripts/hillclimb/promote.py", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
