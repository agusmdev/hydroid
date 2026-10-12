#!/usr/bin/env python3
"""The gate a fix must pass before it is kept. In order, stopping at the first failure:

1. harness lock   the change does not touch the harness: the scorer, the oracle, the cases, the
                  split, the fixture environment, the Rust test that reads them, or the skill
2. build          cargo build --release -p hydroid
3. lint           cargo clippy (as CI runs it); skipped with --fast
4. tests          cargo test --workspace (includes every `train` adversarial case)
5. score          train + open: 0 false positives, no errors;
                  holdout (aggregate only): 0 false positives, recall not below the last round
6. polar          optional (--polar): Polar's wall time, against the goal in _state.json

With --record "what changed", a passing gate also appends the round to
.claude/hillclimb/_state.json, keeps per-case results in .claude/hillclimb/recall/vN/, and moves
`open` cases hydroid now passes to `train` (the ratchet: from then on cargo test requires them).

Usage: gate.py [--base REF] [--fast] [--polar] [--record "change"] [--lever "..."]
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STATE = ROOT / ".claude/hillclimb/_state.json"
RECALL = ROOT / ".claude/hillclimb/recall"
SPLIT = ROOT / "tests/adversarial/split.json"
HERE = Path(__file__).resolve().parent
PROTECTED = (
    "scripts/hillclimb/",
    "tests/adversarial/",
    "tests/fixtures/pyproject.toml",
    "tests/fixtures/uv.lock",
    "crates/hydroid/tests/fixtures.rs",
    "crates/hydroid/tests/common/",
    ".claude/skills/hydroid-self-improve/",
    ".claude/hillclimb/",
)
SUMMARY = re.compile(r"^(train|holdout)\s+recall\s+([\d.]+)% \((\d+)/(\d+)\)\s+false_positives (\d+)\s+"
                     r"cases_clean (\d+)/(\d+)\s+errors (\d+)")


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    print(f"$ {' '.join(cmd)}", flush=True)
    return subprocess.run(cmd, cwd=ROOT, text=True, **kw)


def fail(step: str, why: str) -> int:
    print(f"\nGATE FAILED at {step}: {why}")
    return 1


def changed_files(base: str) -> set[str]:
    diff = run(["git", "diff", "--name-only", base], capture_output=True).stdout.split()
    untracked = run(["git", "ls-files", "--others", "--exclude-standard"], capture_output=True).stdout.split()
    return set(diff) | set(untracked)


def parse_summary(text: str) -> dict[str, dict]:
    out = {}
    for line in text.splitlines():
        m = SUMMARY.match(line.strip())
        if m:
            name, _, hits, total, fp, clean, cases, errors = m.groups()
            out[name] = {"hits": int(hits), "total": int(total), "fp": int(fp),
                         "clean": int(clean), "cases": int(cases), "errors": int(errors)}
    return out


def last_holdout(state: dict) -> tuple[int, int] | None:
    for r in reversed(state.get("rounds", [])):
        v = r.get("holdout_recall")
        if isinstance(v, str) and "/" in v:
            h, t = v.split("/")
            return int(h), int(t)
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="HEAD", help="commit the fix started from (default: uncommitted changes only)")
    parser.add_argument("--fast", action="store_true", help="skip clippy")
    parser.add_argument("--polar", action="store_true", help="also time Polar (needs scripts/corpus.sh polar)")
    parser.add_argument("--record", metavar="CHANGE", help="on success, record the round with this description")
    parser.add_argument("--lever", default="adversarial recall", help="what the round worked on")
    args = parser.parse_args()

    touched = sorted(f for f in changed_files(args.base) if f.startswith(PROTECTED))
    if touched:
        return fail("harness lock", "the fix changed harness files:\n  " + "\n  ".join(touched)
                    + "\nA fix changes hydroid (crates/, catalog), never the yardstick.")

    if run(["cargo", "build", "--release", "-q", "-p", "hydroid"]).returncode:
        return fail("build", "cargo build failed")
    if not args.fast and run(["cargo", "clippy", "--workspace", "--all-targets", "--locked", "-q", "--",
                              "-D", "warnings"]).returncode:
        return fail("lint", "clippy reports warnings")
    if run(["cargo", "test", "--workspace", "--locked", "-q"]).returncode:
        return fail("tests", "cargo test failed (a train case regressed, or a unit test broke)")

    state = json.loads(STATE.read_text())
    versions = [int(m.group(1)) for p in RECALL.glob("v*") if (m := re.fullmatch(r"v(\d+)", p.name))]
    out_dir = RECALL / f"v{max(versions, default=-1) + 1}"
    train = run([sys.executable, str(HERE / "score.py"), "--split", "train", "--out", str(out_dir)],
                capture_output=True)
    print(train.stdout, end="")
    holdout = run([sys.executable, str(HERE / "score.py"), "--split", "holdout", "--quiet"], capture_output=True)
    print(holdout.stdout, end="")
    scores = parse_summary(train.stdout) | parse_summary(holdout.stdout)
    t, h = scores.get("train"), scores.get("holdout")
    if t is None:
        return fail("score", f"no train summary:\n{train.stderr}")
    if t["fp"] or t["errors"]:
        return fail("score", f"train: {t['fp']} false positives, {t['errors']} errors (must be 0)")
    if h:
        if h["fp"]:
            return fail("score", f"holdout: {h['fp']} false positives (must be 0)")
        prev = last_holdout(state)
        if prev and prev[1] and h["total"] and h["hits"] / h["total"] < prev[0] / prev[1]:
            return fail("score", f"holdout recall fell: {h['hits']}/{h['total']} < {prev[0]}/{prev[1]} "
                        "(the fix overfits the training cases)")

    polar = None
    if args.polar:
        polar_dir = ROOT / "target/corpus/polar/server"
        if not polar_dir.exists():
            return fail("polar", "no Polar checkout: run scripts/corpus.sh target/corpus polar first")
        p = run(["bash", str(HERE / "polar_time.sh"), str(ROOT / "target/release/hydroid"), "3"], capture_output=True)
        print(p.stdout, end="")
        m = re.search(r"median (\d+) ms", p.stdout)
        polar = int(m.group(1)) if m else None

    print("\nGATE PASSED")
    if not args.record:
        return 0

    rows = [json.loads(line) for line in (out_dir / "results.jsonl").read_text().splitlines()]
    split = json.loads(SPLIT.read_text())
    ratchet = sorted(r["case"] for r in rows if r["split"] == "open" and not r.get("misses")
                     and not r.get("false_positives") and "error" not in r)
    for name in ratchet:
        split[name] = "train"
    SPLIT.write_text(json.dumps(dict(sorted(split.items())), indent=2) + "\n")
    entry = {
        "round": max((r.get("round", 0) for r in state.get("rounds", [])), default=-1) + 1,
        "lever": args.lever,
        "change": args.record,
        "train_recall": f"{t['hits']}/{t['total']}",
        "train_fp": t["fp"],
        "holdout_recall": f"{h['hits']}/{h['total']}" if h else None,
        "holdout_fp": h["fp"] if h else None,
        "results": str(out_dir.relative_to(ROOT)),
        "ratcheted_to_train": ratchet,
    }
    if polar is not None:
        entry["polar_ms_median"] = polar
    state.setdefault("rounds", []).append(entry)
    STATE.write_text(json.dumps(state, indent=2) + "\n")
    print(f"recorded round {entry['round']} in {STATE.relative_to(ROOT)}; "
          f"{len(ratchet)} open case(s) moved to train")
    return 0


if __name__ == "__main__":
    sys.exit(main())
