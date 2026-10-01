#!/usr/bin/env python3
"""Asks Claude for new adversarial cases: Python code that blocks the event loop in ways hydroid
does not report yet (false negatives), or safe code it might report (false positives).

The proposals land in tests/adversarial/candidates/<name>/app.py, never straight in the suite:
a human (or a review round of the hillclimb) checks each expectation is right, then moves the
case into tests/adversarial/cases/ and adds it to split.json.

Needs the Anthropic SDK (`uv run --with anthropic scripts/hillclimb/adversary.py`) and
credentials (ANTHROPIC_API_KEY, or a profile from `ant auth login`).

Usage: scripts/hillclimb/adversary.py [--count 5] [--results .claude/hillclimb/recall/vN/results.jsonl]
"""

import argparse
import json
import re
import sys
from pathlib import Path

import anthropic

ROOT = Path(__file__).resolve().parents[2]
CASES = ROOT / "tests/adversarial/cases"
CANDIDATES = ROOT / "tests/adversarial/candidates"
CATALOG = ROOT / "crates/hydroid_core/catalog.toml"
README = ROOT / "README.md"
MODEL = "claude-opus-5-5"

SYSTEM = """You are an adversarial reviewer for hydroid, a static analyzer (Rust, built on ty's \
type inference) that finds blocking calls reachable on the asyncio event loop in FastAPI code.

You write small, self-contained FastAPI test projects (one app.py each) that hydroid gets wrong.
A good case is realistic: code a FastAPI team could plausibly ship, not an obfuscation contest.
Prefer patterns that are common in real codebases (ORMs, HTTP clients, caches, settings loaders,
dependency injection, validation, file handling) over exotic metaprogramming.

Expectation syntax, as a trailing comment on the line where the diagnostic must appear (the call
site inside code that runs on the loop):
  # expect: requests.api.get      one diagnostic whose blocking function has this defining
                                  qualified name (the module where it is defined, as in the
                                  catalog: requests.get is requests.api.get)
  # expect: *                     one diagnostic, any blocking function (when the qualified name
                                  is uncertain)
Lines without an expectation must produce no diagnostic: also include near-miss negatives (async
look-alikes, offloaded work, cheap sync code) next to the positives.

Only use these third-party libraries (they are installed in the test environment): fastapi,
starlette, pydantic, requests, httpx, sqlalchemy, sqlmodel, redis, pymysql, anyio, tenacity,
bcrypt; plus the standard library.

Every case must be a correct claim about runtime behavior: before marking a line, check that the
blocking call really executes synchronously on the event loop thread (sync FastAPI endpoints and
sync dependencies run in a threadpool; Starlette iterates sync iterators of StreamingResponse in
a threadpool; calling a generator function runs none of its body)."""


def existing_cases() -> str:
    parts = []
    for path in sorted(CASES.glob("*/app.py")):
        parts.append(f"### {path.parent.name}\n```python\n{path.read_text()}```")
    return "\n\n".join(parts)


def current_misses(results: Path | None) -> str:
    if not results or not results.exists():
        return "(no results file given)"
    lines = []
    for row in map(json.loads, results.read_text().splitlines()):
        if row.get("split") != "train":  # never show the held-out cases' failures
            continue
        for kind in ("misses", "false_positives"):
            lines.extend(f"{row['case']}: {kind}: {item}" for item in row.get(kind, []))
    return "\n".join(lines) or "(none: every training case passes)"


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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--count", type=int, default=5)
    parser.add_argument("--results", type=Path, help="results.jsonl of the latest scored round")
    args = parser.parse_args()

    prompt = f"""<readme>
{README.read_text()}
</readme>

<catalog>
{CATALOG.read_text()}
</catalog>

<existing_cases>
{existing_cases()}
</existing_cases>

<current_failures>
{current_misses(args.results)}
</current_failures>

Write {args.count} new cases that hydroid most likely gets wrong today. Each must exercise a
mechanism the existing cases do not already cover (a different language feature, library API or
FastAPI behavior), not a renamed variant of one."""

    client = anthropic.Anthropic()
    response = client.beta.messages.create(
        model=MODEL,
        max_tokens=16000,
        betas=["server-side-fallback-2026-07-01"],
        fallbacks="default",
        thinking={"type": "adaptive"},
        output_config={"effort": "high", "format": {"type": "json_schema", "schema": SCHEMA}},
        system=SYSTEM,
        messages=[{"role": "user", "content": prompt}],
    )
    if response.stop_reason == "refusal":
        print(f"refused: {response.stop_details}", file=sys.stderr)
        return 1
    if response.stop_reason == "max_tokens":
        print("output truncated (max_tokens); retry with a smaller --count", file=sys.stderr)
        return 1
    text = next(b.text for b in response.content if b.type == "text")
    cases = json.loads(text)["cases"]
    taken = {p.name for p in CASES.iterdir()} | (
        {p.name for p in CANDIDATES.iterdir()} if CANDIDATES.exists() else set()
    )
    for case in cases:
        name = re.sub(r"[^a-z0-9_]", "_", case["name"].lower()) or "case"
        while name in taken:
            name += "_2"
        taken.add(name)
        out = CANDIDATES / name
        out.mkdir(parents=True)
        (out / "app.py").write_text(case["app_py"].rstrip() + "\n")
        (out / "RATIONALE.md").write_text(case["rationale"].strip() + "\n")
        print(f"{out.relative_to(ROOT)}: {case['rationale'].splitlines()[0][:100]}")
    u = response.usage
    print(f"model {response.model}  input {u.input_tokens}  output {u.output_tokens}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
