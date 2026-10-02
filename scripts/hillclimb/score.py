#!/usr/bin/env python3
"""Scores hydroid against the adversarial suite (tests/adversarial/cases).

Each case is a small project with inline expectations, same syntax as tests/fixtures:
`# expect: requests.api.get` (several names for several diagnostics; `*` matches any sink).
Every diagnostic must be expected: an unexpected one is a false positive.

Writes one JSON line per case to <out>/results.jsonl and prints a summary per split.
Binary: $HYDROID_BIN (default target/release/hydroid).
Usage: scripts/hillclimb/score.py [--out DIR] [--split train|holdout|all] [--quiet]
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CASES = ROOT / "tests/adversarial/cases"
SPLIT = ROOT / "tests/adversarial/split.json"
VENV = ROOT / "tests/fixtures/.venv"
BINARY = Path(os.environ.get("HYDROID_BIN", ROOT / "target/release/hydroid"))
EXPECT = re.compile(r"# expect: (.*)$")


def expectations(case: Path) -> dict[tuple[str, int], list[str]]:
    out: dict[tuple[str, int], list[str]] = {}
    for path in sorted(case.rglob("*.py")):
        rel = path.relative_to(case).as_posix()
        for i, line in enumerate(path.read_text().splitlines(), 1):
            m = EXPECT.search(line)
            if m:
                out[(rel, i)] = m.group(1).split()
    return out


def run_case(case: Path) -> dict:
    started = time.perf_counter()
    proc = subprocess.run(
        [str(BINARY), str(case), "--python", str(VENV), "--format", "json", "--no-cache"],
        capture_output=True,
        text=True,
    )
    elapsed = time.perf_counter() - started
    if proc.returncode not in (0, 1):
        return {"error": proc.stderr.strip(), "seconds": elapsed}
    report = json.loads(proc.stdout)
    got: dict[tuple[str, int], list[str]] = {}
    for d in report["diagnostics"]:
        key = (d["location"]["path"], d["location"]["line"])
        got.setdefault(key, []).append(d["sink"]["qualname"])
    want = expectations(case)
    hits, misses, fps = [], [], []
    for key in sorted(set(want) | set(got)):
        remaining = Counter(got.get(key, []))
        for name in sorted(want.get(key, []), key=lambda n: n == "*"):  # exact names first
            match = next((g for g in remaining if remaining[g] and (name == "*" or g == name)), None)
            where = f"{key[0]}:{key[1]}"
            if match:
                remaining[match] -= 1
                hits.append(f"{where} {name}")
            else:
                misses.append(f"{where} {name} (got {got.get(key, [])})")
        fps.extend(f"{key[0]}:{key[1]} {g}" for g, n in remaining.items() for _ in range(n))
    return {"hits": hits, "misses": misses, "false_positives": fps, "seconds": round(elapsed, 3)}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path)
    parser.add_argument("--split", default="all", choices=["train", "holdout", "all"])
    parser.add_argument("--quiet", action="store_true", help="aggregate only (use for the holdout)")
    args = parser.parse_args()
    split = json.loads(SPLIT.read_text())
    cases = sorted(p for p in CASES.iterdir() if p.is_dir())
    rows = []
    for case in cases:
        which = split.get(case.name, "train")
        if args.split != "all" and which != args.split:
            continue
        rows.append({"case": case.name, "split": which, **run_case(case)})
    if args.out:
        args.out.mkdir(parents=True, exist_ok=True)
        with open(args.out / "results.jsonl", "w") as f:
            for row in rows:
                f.write(json.dumps(row) + "\n")
    for name in ("train", "holdout"):
        part = [r for r in rows if r["split"] == name]
        if not part:
            continue
        h = sum(len(r.get("hits", [])) for r in part)
        m = sum(len(r.get("misses", [])) for r in part)
        fp = sum(len(r.get("false_positives", [])) for r in part)
        errors = sum("error" in r for r in part)
        clean = sum(not r.get("misses") and not r.get("false_positives") and "error" not in r for r in part)
        recall = h / (h + m) if h + m else 1.0
        print(f"{name:8} recall {recall:6.1%} ({h}/{h + m})  false_positives {fp}  "
              f"cases_clean {clean}/{len(part)}  errors {errors}")
        if name == "train" and not args.quiet:
            for r in part:
                for kind in ("error", "misses", "false_positives"):
                    items = r.get(kind)
                    if items:
                        for item in [items] if isinstance(items, str) else items:
                            print(f"  {r['case']}: {kind[:-1] if kind.endswith('s') else kind}: {item}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
