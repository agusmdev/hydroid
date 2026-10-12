#!/usr/bin/env python3
"""Moves candidates (tests/adversarial/candidates/<name>) into the adversarial suite.

Re-checks each one first (check.py): only BREAKS is promoted, or UNVERIFIED with --reviewed
"<why the labels are right>" (written to RATIONALE.md). The split is fixed by the name, never
chosen: sha1(name) % 3 == 0 goes to `holdout` (never shown to the fixer), the rest to `open`
(scored with train; gate.py moves it to `train` once hydroid passes it).

Usage: promote.py NAME... [--reviewed "note"]   |   promote.py --all-proven
"""

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CASES = ROOT / "tests/adversarial/cases"
CANDIDATES = ROOT / "tests/adversarial/candidates"
SPLIT = ROOT / "tests/adversarial/split.json"
CHECK = Path(__file__).resolve().parent / "check.py"


def split_of(name: str) -> str:
    return "holdout" if int(hashlib.sha1(name.encode()).hexdigest(), 16) % 3 == 0 else "open"


def check(dirs: list[Path]) -> dict[str, dict]:
    proc = subprocess.run([sys.executable, str(CHECK), "--json", *map(str, dirs)],
                          capture_output=True, text=True, check=True)
    return {r["case"]: r for r in map(json.loads, proc.stdout.splitlines())}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("names", nargs="*")
    parser.add_argument("--all-proven", action="store_true", help="every candidate whose verdict is BREAKS")
    parser.add_argument("--reviewed", help="promote UNVERIFIED cases too; the reviewer's note")
    args = parser.parse_args()
    dirs = [CANDIDATES / n for n in args.names]
    if args.all_proven:
        dirs += sorted(p for p in CANDIDATES.iterdir() if p.is_dir()) if CANDIDATES.exists() else []
    if not dirs:
        parser.error("no candidates")
    for d in dirs:
        if not (d / "app.py").exists():
            sys.exit(f"no candidate at {d}")
    results = check(dirs)
    split = json.loads(SPLIT.read_text())
    promoted = 0
    for d in dirs:
        verdict = results[d.name]["verdict"]
        ok = verdict == "BREAKS" or (verdict == "UNVERIFIED" and args.reviewed and not args.all_proven)
        if not ok:
            print(f"skip    {d.name}: {verdict}")
            continue
        if (CASES / d.name).exists():
            print(f"skip    {d.name}: a suite case has this name")
            continue
        if verdict == "UNVERIFIED":
            with open(d / "RATIONALE.md", "a") as f:
                f.write(f"\nReviewed (runtime could not prove every line): {args.reviewed}\n")
        (d / "CHECK.json").unlink(missing_ok=True)
        shutil.move(str(d), CASES / d.name)
        split[d.name] = split_of(d.name)
        promoted += 1
        print(f"promote {d.name} -> {split[d.name]}")
    SPLIT.write_text(json.dumps(dict(sorted(split.items())), indent=2) + "\n")
    print(f"{promoted} promoted", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
