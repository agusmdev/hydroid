---
name: hydroid-self-improve
description: Run hydroid's adversarial self-improvement loop - black-box adversary agents write FastAPI cases that break hydroid, a runtime oracle proves each one, a fixer agent changes hydroid until the gate passes, and the round is recorded. Use when asked to improve, harden, hillclimb, red-team or self-improve hydroid / the event loop checker, or to find and fix its false negatives or false positives.
---

# hydroid self-improvement loop

One **round** = attack → verify → triage → promote → fix → gate → record. Run rounds until the
stop rule (end of this file) or the round budget the user gave. Default budget: 1 round.

The yardstick never moves during a fix: the oracle, scorer, cases and split are the *harness*.
The adversary may only add cases; the fixer may only change hydroid. `gate.py` enforces this.

## Tools (all in `scripts/hillclimb/`)

| Script | Does |
|---|---|
| `check.py CASE... \| --candidates \| --suite` | Per line: labels vs hydroid vs runtime. Verdict `BREAKS` / `PASSES` / `INVALID` / `UNVERIFIED` / `ERROR`. `--json` for machines. |
| `oracle_runtime.py CASE` | Runtime ground truth (check.py calls it). Runs the app with TestClient, records catalog calls on the loop thread, at the location hydroid uses. |
| `adversary.py [--count N] [--rounds R] [--theme T]` | Headless adversary over the Anthropic API (same contract as the subagents below; for CI or no Claude Code). |
| `promote.py NAME... [--reviewed NOTE] \| --all-proven` | Candidate → suite. Split by `sha1(name) % 3`: `holdout` or `open`. |
| `score.py --split train\|holdout [--quiet]` | Recall / false positives. Train includes `open`. Holdout: always `--quiet`. |
| `gate.py --base REF [--fast] [--polar] [--record CHANGE]` | Harness lock → build → clippy → cargo test → score (+ holdout must not fall) → record + ratchet `open`→`train`. |
| `oracle.py PROJECT REPORT.json` | Old naive syntactic second opinion, for real-world corpora (`scripts/corpus.sh`). |

Splits in `tests/adversarial/split.json`: `train` (cargo test requires it), `open` (promoted, not
fixed yet), `holdout` (scored in aggregate only; the fixer never reads these cases).

## Step 0 - setup (once per machine)

```sh
cargo build --release -p hydroid                      # target/release/hydroid
(cd tests/fixtures && uv sync --frozen)               # fixture venv the oracle runs cases in
python3 scripts/hillclimb/check.py --suite | tail -1  # expect: PASSES for every suite case
python3 scripts/hillclimb/score.py --split train | head -1
```

If `check.py --suite` shows `INVALID` for a suite case, stop and report it: either the oracle or
the label is wrong, and the yardstick must be fixed (by the user, as a harness change) first.

Work on a branch, never on `main`. Note the start commit: `BASE=$(git rev-parse HEAD)`.

## Step 1 - attack (parallel adversary subagents)

Launch 3-5 adversary subagents in one message (Agent tool, `general-purpose`), each with a
different theme. Pick themes no recent round used (see `.claude/hillclimb/_state.json` rounds and
existing case names in `tests/adversarial/cases/`). Theme ideas: response serialization and
models; dependency injection (classes, `Annotated`, sub-dependencies, `dependency_overrides`);
middleware and exception handlers; lifespan and app state; background tasks and websockets;
ORMs (SQLAlchemy/SQLModel sessions, events, lazy loading, hybrid properties); HTTP clients and
SDK wrappers; caching and settings (`lru_cache`, `cached_property`, pydantic-settings style);
Python features (descriptors, metaclasses, `__init_subclass__`, context managers, generators,
dataclasses, protocols, `functools`); false positives (code that looks blocking but is offloaded,
awaited, or only runs in a thread).

Give each adversary the prompt in [adversary-prompt.md](adversary-prompt.md), with `{THEME}` and
`{COUNT}` (default 3) filled in. Each returns the candidate names it left and their verdicts.

## Step 2 - verify

```sh
python3 scripts/hillclimb/check.py --candidates
```

- `BREAKS`: proven by the runtime. Goes to triage.
- `UNVERIFIED`: send to one reviewer subagent with [reviewer-prompt.md](reviewer-prompt.md). It
  either makes the oracle reach the disputed lines (then it becomes `BREAKS`), or gives a
  verdict with a reason. Promote with `--reviewed` only when the reviewer is confident.
- `PASSES` / `INVALID` / `ERROR`: delete the candidate (`rm -r tests/adversarial/candidates/<name>`).

Never edit a label to match hydroid. The runtime decides; hydroid's output is what is on trial.

## Step 3 - triage

Read each proven candidate's `RATIONALE.md` and the `check.py` lines. Group candidates by root
mechanism (one hydroid gap). Per round keep at most ~5 cases, one or two per mechanism: one fix
per mechanism is the unit of work. Delete exact duplicates of a mechanism already in the suite
(`grep` the suite for the same API or pattern).

Prefer, in order: false positives users would hit (precision is trust) > common real-world false
negatives (ORMs, HTTP clients, response models, dependencies) > exotic language features.

## Step 4 - promote and commit the yardstick

```sh
python3 scripts/hillclimb/promote.py <name>...      # or --all-proven
git add tests/adversarial && git commit -m "test(adversarial): <mechanisms>"
BASE=$(git rev-parse HEAD)                          # the fixer starts here
```

Check the output: some cases land in `holdout`. That is intended; do not move them.

## Step 5 - fix (one fixer subagent per mechanism, sequentially)

Launch a fixer subagent (Agent tool, `general-purpose`) with
[fixer-prompt.md](fixer-prompt.md), `{BASE}` and the `open` case names of one mechanism filled in.
List the holdout case names in its prompt as forbidden to read. Run fixers one after another, not
in parallel: they edit the same analysis code. Each fixer ends when
`python3 scripts/hillclimb/gate.py --base {BASE}` passes, or it reports that it is stuck.

After each fixer returns, review its diff yourself (`git diff $BASE`): reject special-casing of
test names, file names or line numbers, catalog entries that only exist to match one case, and
any change that silences a whole category. A general mechanism is the goal.

## Step 6 - gate and record

```sh
python3 scripts/hillclimb/gate.py --base $BASE --record "<what changed, one sentence>" --lever "<mechanism>"
```

Add `--polar` when the change touches analysis performance and the Polar corpus exists
(`scripts/corpus.sh target/corpus polar`); report its median against the goal in `_state.json`.

On pass: commit the fix together with the updated `split.json`, `_state.json` and
`.claude/hillclimb/recall/vN/` (`fix(analysis): ...`, conventional commits: the PR title check
requires them). Ask the user before pushing or opening a PR.

On fail: give the fixer the gate output once more. If it still fails, revert the fix
(`git checkout $BASE -- crates/`), leave the cases `open`, and record the mechanism as open in the
round summary.

## Report to the user

Per round, a short table: candidates written / proven / promoted (open vs holdout), mechanisms
fixed, train recall and false positives before → after, holdout recall before → after, and what
is still open. Mention any oracle limitation a reviewer hit (it is a harness improvement to make
later, by a human, outside the round).

## Stop rule

Stop when two rounds in a row produce fewer than 2 `BREAKS` from at least 10 candidates (the
adversary is starving), when the holdout recall reaches 100% with 0 false positives on at least
20 holdout cases, or at the round budget.

## Rules (all agents)

- The adversary never reads `crates/` (hydroid's source): black-box only.
- The fixer never reads holdout cases and never edits the harness: `scripts/hillclimb/`,
  `tests/adversarial/`, `tests/fixtures/pyproject.toml`, `tests/fixtures/uv.lock`,
  `crates/hydroid/tests/fixtures.rs`, `crates/hydroid/tests/common/`, `.claude/hillclimb/`, this
  skill. `gate.py` rejects any diff that touches them.
- Harness improvements (oracle coverage, new libraries in the fixture env) are separate,
  human-approved changes, committed before a round starts, never during a fix.
- The runtime oracle runs case code: cases run in a temporary copy, with network connections and
  process starts disabled. Do not weaken that.
