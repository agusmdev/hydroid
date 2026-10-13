#!/usr/bin/env python3
"""The gate a fix must pass before it is kept. In order, stopping at the first failure:

1. harness lock   the change does not touch the harness: the scorer, the oracle, the cases, the
                  split, the fixture environment, the Rust test that reads them, or the skill
2. literals       no string literal added to crates/**/*.rs (outside comments and test code)
                  names library code (`logging.Handler`, `fastapi.`): library knowledge goes in
                  catalog.toml as data. Other added literals are printed for review (literals.py)
3. build          cargo build --release -p hydroid
4. lint           cargo clippy (as CI runs it); skipped with --fast
5. tests          cargo test --workspace (includes every `train` adversarial case)
6. score          train + open: 0 false positives, no errors;
                  holdout (aggregate only): 0 false positives, recall not below the baseline
                  (with --require-holdout-gain: above it)
7. polar          optional (--polar): Polar's wall time, against the goal in _state.json

The holdout baseline is keyed by the set of holdout case names (`holdout_set`, a hash of the sorted
names; `holdout_sets` in _state.json lists them): the last recorded round with the same set, or,
when the set changed since (cases promoted), the base ref's binary scored on the current set.

With --record "what changed", a passing gate also appends the round to
.claude/hillclimb/_state.json, keeps per-case results in .claude/hillclimb/recall/vN/, and moves
`open` cases hydroid now passes to `train` (the ratchet: from then on cargo test requires them).

Usage: gate.py [--base REF] [--lock-base REF] [--fast] [--polar] [--require-holdout-gain]
               [--record "change"] [--lever "..."]
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STATE = ROOT / ".claude/hillclimb/_state.json"
RECALL = ROOT / ".claude/hillclimb/recall"
SPLIT = ROOT / "tests/adversarial/split.json"
CASES = ROOT / "tests/adversarial/cases"
BASE_BUILDS = ROOT / "target/gate-base"
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import literals  # noqa: E402
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


def holdout_names() -> list[str]:
    split = json.loads(SPLIT.read_text())
    return sorted(p.name for p in CASES.iterdir() if p.is_dir() and split.get(p.name) == "holdout")


def set_hash(names: list[str]) -> str:
    return hashlib.sha1("\n".join(names).encode()).hexdigest()[:12]


def recorded_holdout(state: dict, key: str) -> tuple[int, int, int] | None:
    """(hits, total, round) of the last recorded round scored on this holdout set."""
    for r in reversed(state.get("rounds", [])):
        v = r.get("holdout_recall")
        if r.get("holdout_set") == key and isinstance(v, str) and "/" in v:
            h, t = v.split("/")
            return int(h), int(t), r.get("round")
    return None


def base_binary(base: str) -> Path | None:
    """hydroid built at `base` (cached by commit in target/gate-base/). Shares target/ with the
    head build, so only the workspace crates compile."""
    sha = run(["git", "rev-parse", "--verify", f"{base}^{{commit}}"], capture_output=True).stdout.strip()
    if not sha:
        return None
    out = BASE_BUILDS / sha / "hydroid"
    if out.exists():
        return out
    tree = BASE_BUILDS / sha / "src"
    if tree.exists():
        run(["git", "worktree", "remove", "--force", str(tree)], capture_output=True)
        shutil.rmtree(tree, ignore_errors=True)
    run(["git", "worktree", "prune"], capture_output=True)
    if run(["git", "worktree", "add", "--detach", "-q", str(tree), sha]).returncode:
        return None
    try:
        env = os.environ | {"CARGO_TARGET_DIR": str(ROOT / "target")}
        if subprocess.run(["cargo", "build", "--release", "--locked", "-q", "-p", "hydroid"],
                          cwd=tree, env=env).returncode:
            return None
        shutil.copy2(ROOT / "target/release/hydroid", out)
    finally:
        run(["git", "worktree", "remove", "--force", str(tree)], capture_output=True)
    return out


def score_holdout(binary: Path | None = None) -> subprocess.CompletedProcess:
    env = os.environ | ({"HYDROID_BIN": str(binary)} if binary else {})
    return run([sys.executable, str(HERE / "score.py"), "--split", "holdout", "--quiet"],
               capture_output=True, env=env)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="HEAD", help="commit the fix started from (default: uncommitted changes only)")
    parser.add_argument("--lock-base", metavar="REF",
                        help="harness lock against this ref instead of --base (only for a session that "
                             "changed the harness on purpose; a fixer never passes it)")
    parser.add_argument("--fast", action="store_true", help="skip clippy")
    parser.add_argument("--require-holdout-gain", action="store_true",
                        help="fail unless holdout recall goes up (default: fail only if it goes down)")
    parser.add_argument("--polar", action="store_true", help="also time Polar (needs scripts/corpus.sh polar)")
    parser.add_argument("--record", metavar="CHANGE", help="on success, record the round with this description")
    parser.add_argument("--lever", default="adversarial recall", help="what the round worked on")
    args = parser.parse_args()

    lock_base = args.lock_base or args.base
    touched = sorted(f for f in changed_files(lock_base) if f.startswith(PROTECTED))
    if touched:
        return fail("harness lock", "the fix changed harness files:\n  " + "\n  ".join(touched)
                    + "\nA fix changes hydroid (crates/, catalog), never the yardstick.")

    print(f"$ literals.py {args.base}  (string literals added to crates/**/*.rs)")
    no_library_names, text = literals.report(args.base)
    print(text)
    if not no_library_names:
        return fail("literals", "the change adds Rust string literals naming library code (above)")

    # The holdout baseline needs the base ref's binary when the holdout set changed since the last
    # recorded round: built first, since it shares target/release/hydroid with the head build.
    state = json.loads(STATE.read_text())
    names = holdout_names()
    key = set_hash(names)
    baseline = recorded_holdout(state, key) if names else None
    base_bin = None
    if names and baseline is None:
        print(f"holdout set {key} has no recorded round: scoring it with the binary at {args.base}")
        base_bin = base_binary(args.base)
        if base_bin is None:
            return fail("build", f"could not build hydroid at {args.base} for the holdout baseline")

    if run(["cargo", "build", "--release", "-q", "-p", "hydroid"]).returncode:
        return fail("build", "cargo build failed")
    if not args.fast and run(["cargo", "clippy", "--workspace", "--all-targets", "--locked", "-q", "--",
                              "-D", "warnings"]).returncode:
        return fail("lint", "clippy reports warnings")
    if run(["cargo", "test", "--workspace", "--locked", "-q"]).returncode:
        return fail("tests", "cargo test failed (a train case regressed, or a unit test broke)")

    versions = [int(m.group(1)) for p in RECALL.glob("v*") if (m := re.fullmatch(r"v(\d+)", p.name))]
    out_dir = RECALL / f"v{max(versions, default=-1) + 1}"
    train = run([sys.executable, str(HERE / "score.py"), "--split", "train", "--out", str(out_dir)],
                capture_output=True)
    print(train.stdout, end="")
    holdout = score_holdout()
    print(holdout.stdout, end="")
    baseline_note = None
    if base_bin is not None:
        b = parse_summary(score_holdout(base_bin).stdout).get("holdout")
        if b is None:
            return fail("score", f"no holdout summary from the binary at {args.base}")
        baseline = (b["hits"], b["total"], None)
        baseline_note = f"{b['hits']}/{b['total']} with the binary at {args.base}"
    elif baseline is not None:
        baseline_note = f"{baseline[0]}/{baseline[1]} recorded in round {baseline[2]}"
    if baseline_note:
        print(f"holdout baseline (set {key}, {len(names)} cases): {baseline_note}")
    scores = parse_summary(train.stdout) | parse_summary(holdout.stdout)
    t, h = scores.get("train"), scores.get("holdout")
    if t is None:
        return fail("score", f"no train summary:\n{train.stderr}")
    if t["fp"] or t["errors"]:
        return fail("score", f"train: {t['fp']} false positives, {t['errors']} errors (must be 0)")
    if h:
        if h["fp"]:
            return fail("score", f"holdout: {h['fp']} false positives (must be 0)")
        if baseline is not None:
            # Compare as fractions: the same names can carry more labels after a label review.
            now, before = h["hits"] * max(baseline[1], 1), baseline[0] * max(h["total"], 1)
            if now < before:
                return fail("score", f"holdout recall fell: {h['hits']}/{h['total']} < {baseline_note} "
                            "(the fix overfits the training cases)")
            if args.require_holdout_gain and now <= before:
                return fail("score", f"holdout recall did not go up: {h['hits']}/{h['total']} vs "
                            f"{baseline_note} (--require-holdout-gain)")

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
        "holdout_set": key if h else None,
        "holdout_baseline": baseline_note,
        "results": str(out_dir.relative_to(ROOT)),
        "ratcheted_to_train": ratchet,
    }
    if lock_base != args.base:
        entry["harness_lock_base"] = lock_base
    if polar is not None:
        entry["polar_ms_median"] = polar
    if h:
        state.setdefault("holdout_sets", {})[key] = names
    state.setdefault("rounds", []).append(entry)
    STATE.write_text(json.dumps(state, indent=2) + "\n")
    print(f"recorded round {entry['round']} in {STATE.relative_to(ROOT)}; "
          f"{len(ratchet)} open case(s) moved to train")
    return 0


if __name__ == "__main__":
    sys.exit(main())
