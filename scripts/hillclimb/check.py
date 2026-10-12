#!/usr/bin/env python3
"""Checks adversarial cases three ways, line by line: the `# expect:` labels, hydroid's report,
and the runtime oracle (oracle_runtime.py: the app really runs; what blocked on the loop?).

Verdict per case:
  BREAKS      hydroid is wrong and the runtime proves it: every line where hydroid disagrees with
              the labels is either a label the oracle saw block (a miss) or a line the oracle saw
              run on the loop without blocking (a false positive). Ready to promote.
  PASSES      hydroid already agrees with every label: nothing to fix.
  INVALID     the labels are wrong: the oracle saw a blocking call on a line with no label.
  UNVERIFIED  hydroid disagrees, but the oracle could not prove a disputed line (the line never
              ran on the loop, or it ran and nothing blocked under a label). Fix the case so the
              oracle reaches it, or have a reviewer judge it by hand.
  ERROR       hydroid or the oracle failed.

Binary: $HYDROID_BIN (default $CARGO_TARGET_DIR/release/hydroid, target/ when unset).
Usage: check.py CASE_DIR... | --candidates | --suite   [--json] [-v]
"""

import argparse
import json
import os
import subprocess
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from score import BINARY, CASES, ROOT, VENV, expectations  # noqa: E402

CANDIDATES = ROOT / "tests/adversarial/candidates"
ORACLE = Path(__file__).resolve().parent / "oracle_runtime.py"
PYTHON = VENV / "bin/python"
Key = tuple[str, int]


def run_hydroid(case: Path) -> dict[Key, list[str]] | str:
    proc = subprocess.run(
        [str(BINARY), str(case), "--python", str(VENV), "--format", "json", "--no-cache"],
        capture_output=True, text=True,
    )
    if proc.returncode not in (0, 1):
        return f"hydroid exited {proc.returncode}: {proc.stderr.strip()[-2000:]}"
    got: dict[Key, list[str]] = {}
    for d in json.loads(proc.stdout)["diagnostics"]:
        got.setdefault((d["location"]["path"], d["location"]["line"]), []).append(d["sink"]["qualname"])
    return got


def run_oracle(case: Path, timeout: float) -> dict:
    try:
        proc = subprocess.run(
            [str(PYTHON), "-W", "ignore", str(ORACLE), str(case)],
            capture_output=True, text=True, timeout=timeout,
        )
        out, err, timed_out = proc.stdout, proc.stderr, False
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"").decode() if isinstance(e.stdout, bytes) else (e.stdout or "")
        err, timed_out = "", True
    blocks: dict[Key, list[str]] = {}
    errors, loop_lines, done = [], {}, False
    for line in out.splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue  # the case printed something
        if row.get("kind") == "block":
            blocks.setdefault((row["path"], row["line"]), []).append(row["name"])
        elif row.get("kind") == "error":
            errors.append(f"{row['where']}: {row['error']}")
        elif row.get("kind") == "done":
            loop_lines, done = row["loop_lines"], True
    if timed_out:
        errors.append(f"oracle timed out after {timeout:.0f}s (results so far kept; loop coverage lost)")
    elif not done:
        errors.append(f"oracle crashed: {err.strip()[-2000:]}")
    return {"blocks": blocks, "errors": errors,
            "loop_lines": {(p, n) for p, ns in loop_lines.items() for n in ns}}


def agree(want: list[str], got: list[str]) -> tuple[list[str], list[str]]:
    """(missing expectations, unexpected diagnostics) on one line, as score.py matches them."""
    remaining = Counter(got)
    missing = []
    for name in sorted(want, key=lambda n: n == "*"):
        match = next((g for g in remaining if remaining[g] and (name == "*" or g == name)), None)
        if match:
            remaining[match] -= 1
        else:
            missing.append(name)
    return missing, [g for g, n in remaining.items() for _ in range(n)]


def check(case: Path, timeout: float) -> dict:
    want = {k: [n for n in v if n != "via" and "." in n or n == "*"] for k, v in expectations(case).items()}
    got = run_hydroid(case)
    if isinstance(got, str):
        return {"case": case.name, "verdict": "ERROR", "errors": [got], "lines": []}
    oracle = run_oracle(case, timeout)
    blocks, ran = oracle["blocks"], oracle["loop_lines"]
    lines, invalid, unproven, disputed = [], False, False, False
    for key in sorted(set(want) | set(got) | set(blocks)):
        w, g, b = want.get(key, []), got.get(key, []), blocks.get(key, [])
        if w and b:
            runtime = "confirmed"
        elif w:
            runtime = "doubt: ran on the loop, nothing blocked" if key in ran else "unreached"
        elif b:
            runtime = "LABEL MISSING: blocked on the loop"
            invalid = True
        else:
            runtime = "safe: ran on the loop, nothing blocked" if key in ran else "unreached"
        missing, extra = agree(w, g)
        hydroid = "ok"
        if missing or extra:
            disputed = True
            hydroid = " ".join([f"miss {m}" for m in missing] + [f"false-positive {e}" for e in extra])
            proven = (not missing or runtime == "confirmed") and (not extra or runtime.startswith("safe"))
            unproven |= not proven
        note = ""
        if b and w and "*" not in w and not set(w) & set(b):
            note = "runtime name differs (hydroid reports the outermost catalog function)"
        lines.append({"path": key[0], "line": key[1], "label": w, "hydroid": g, "runtime": b,
                      "runtime_status": runtime, "hydroid_status": hydroid, "note": note})
    if invalid:
        verdict = "INVALID"
    elif not disputed:
        verdict = "PASSES"
    elif unproven:
        verdict = "UNVERIFIED"
    else:
        verdict = "BREAKS"
    return {"case": case.name, "path": str(case.relative_to(ROOT)) if case.is_relative_to(ROOT) else str(case),
            "verdict": verdict, "errors": oracle["errors"], "lines": lines}


def show(result: dict, verbose: bool) -> None:
    print(f"{result['verdict']:10} {result['case']}")
    for ln in result["lines"]:
        interesting = ln["hydroid_status"] != "ok" or not ln["runtime_status"].startswith(("confirmed", "safe"))
        if not (verbose or interesting):
            continue
        print(f"    {ln['path']}:{ln['line']}  label={' '.join(ln['label']) or '-'}  "
              f"hydroid={' '.join(ln['hydroid']) or '-'}  runtime={' '.join(ln['runtime']) or '-'}")
        print(f"        hydroid: {ln['hydroid_status']}   runtime: {ln['runtime_status']}"
              + (f"   ({ln['note']})" if ln["note"] else ""))
    if verbose or result["verdict"] in ("ERROR", "UNVERIFIED"):
        for e in result["errors"]:
            print(f"    oracle: {e[:300]}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("cases", nargs="*", type=Path)
    parser.add_argument("--candidates", action="store_true", help="every case in tests/adversarial/candidates")
    parser.add_argument("--suite", action="store_true", help="every case in tests/adversarial/cases")
    parser.add_argument("--json", action="store_true", help="one JSON object per case")
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args()
    cases = [c.resolve() for c in args.cases]
    if args.candidates and CANDIDATES.exists():
        cases += sorted(p for p in CANDIDATES.iterdir() if p.is_dir())
    if args.suite:
        cases += sorted(p for p in CASES.iterdir() if p.is_dir())
    if not cases:
        parser.error("no cases")
    if not BINARY.exists():
        sys.exit(f"no hydroid binary at {BINARY}: `cargo build --release` or set HYDROID_BIN")
    if not PYTHON.exists():
        sys.exit(f"no fixture environment at {VENV}: `uv sync --frozen` in tests/fixtures")
    tally = Counter()
    for case in cases:
        result = check(case, args.timeout)
        tally[result["verdict"]] += 1
        if args.json:
            print(json.dumps(result))
        else:
            show(result, args.verbose)
    if not args.json:
        print("  ".join(f"{k} {v}" for k, v in sorted(tally.items())), file=sys.stderr)
    return 0


if __name__ == "__main__":
    os.environ.setdefault("PYTHONDONTWRITEBYTECODE", "1")
    sys.exit(main())
