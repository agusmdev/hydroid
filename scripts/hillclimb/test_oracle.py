#!/usr/bin/env python3
"""Tests for the runtime oracle (oracle_runtime.py). Each case in scripts/hillclimb/tests/oracle/
labels exactly what must block on the loop (`# expect: NAME`, the outermost catalog function the
oracle records): an unlabeled recorded call is a false report, a label not recorded is a miss.
Modules in tests/oracle_lib/ are importable as library code (outside every case dir).

Usage: test_oracle.py [CASE...] [--oracle PATH]   (--oracle: test another oracle, e.g. an old one)
"""

import argparse
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check import ORACLE, run_oracle  # noqa: E402
from score import expectations  # noqa: E402

import check  # noqa: E402

TESTS = Path(__file__).resolve().parent / "tests/oracle"
LIBRARY = Path(__file__).resolve().parent / "tests/oracle_lib"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("cases", nargs="*", type=Path)
    parser.add_argument("--oracle", type=Path, default=ORACLE)
    args = parser.parse_args()
    check.ORACLE = args.oracle.resolve()
    os.environ["PYTHONPATH"] = os.pathsep.join(filter(None, [str(LIBRARY), os.environ.get("PYTHONPATH")]))
    cases = [c.resolve() for c in args.cases] or sorted(p for p in TESTS.iterdir() if p.is_dir())
    failed = 0
    for case in cases:
        want = {k: sorted(v) for k, v in expectations(case).items()}
        result = run_oracle(case, 60)
        got = {k: sorted(v) for k, v in result["blocks"].items()}
        problems = [f"{p}:{n} recorded {got.get((p, n), [])}, label {want.get((p, n), [])}"
                    for p, n in sorted(set(got) | set(want)) if got.get((p, n)) != want.get((p, n))]
        problems += [f"oracle error: {e[:300]}" for e in result["errors"]]
        failed += bool(problems)
        print(f"{'FAIL' if problems else 'ok  '}  {case.name}")
        for line in problems:
            print(f"      {line}")
    print(f"{len(cases) - failed} passed, {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
