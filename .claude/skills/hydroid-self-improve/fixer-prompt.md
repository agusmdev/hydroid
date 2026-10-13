You improve hydroid, a Rust static analyzer (built on ty's semantic model) that reports blocking
calls reachable on the asyncio event loop in FastAPI code. Read `CLAUDE.md` if present,
`README.md`, `docs/GUIDE.md`, and the code under `crates/` as needed.

Mechanism to fix: **{MECHANISM}** (a gap in hydroid's analysis, not a library). Failing cases
(split `open`): {CASES}.
False-positive leads (proven at runtime, never in the suite): {LEADS}. For each, write your own
regression fixture under `tests/fixtures/cases/` from the lead and make it pass.
See them fail: `cargo build --release -p hydroid && python3 scripts/hillclimb/score.py --split train`
lists each open case's misses and false positives. One case in full:
`target/release/hydroid tests/adversarial/cases/<name> --python tests/fixtures/.venv --no-cache`.
Each case's `RATIONALE.md` says what runs on the loop and why.

## Rules

- Fix the general mechanism, not the cases. No special cases for case names, file names, line
  numbers or variable names. A catalog entry (`crates/hydroid_core/catalog.toml`) is fine when the
  gap is a missing blocking API, and must name the real defining function.
- **No library-specific rules in Rust.** Rust code holds no library knowledge: no qualified names
  (`"logging.Handler"`, `"starlette.responses.Response"`, `"fastapi."` prefixes), no method or
  keyword names that mean something only for one library (`"emit"`, `"response_model"`), no
  variable-name conventions (`request`, `app`). Library knowledge goes only in
  `crates/hydroid_core/catalog.toml`, as data. The gate fails on added Rust literals that look
  like qualified names, and prints every other added literal: the reviewer rejects library names
  in that list too.
- First try to fix the gap in the analysis of library source: hydroid indexes library code through
  ty, so what a library does can usually be derived from its source rather than described. Only
  when that is not possible, describe it in the catalog. A new catalog concept (a new table or
  field) must be general: usable by at least 2 unrelated libraries. Name the 2 examples in your
  report (they go in the commit message).
- You may change `crates/` (except `crates/hydroid/tests/fixtures.rs` and
  `crates/hydroid/tests/common/`), the catalog, docs, and add regression fixtures under
  `tests/fixtures/cases/` (a fixture per new behavior is welcome).
- You must NOT read these held-out cases: {HOLDOUT}. You must NOT edit `scripts/hillclimb/`,
  `tests/adversarial/`, `tests/fixtures/pyproject.toml`, `tests/fixtures/uv.lock`,
  `.claude/hillclimb/`, `.claude/skills/`. The gate rejects such diffs.
- No false positives anywhere: precision matters more than recall. Performance matters: hydroid
  runs on large codebases (Polar: ~1,900 files); avoid whole-program passes that scale badly. Watch
  the analysis phase, not only extraction: the warm (cached) Polar run must stay under 500 ms.
- Match the surrounding code style and comment density.

## Done when

```sh
python3 scripts/hillclimb/gate.py --base {BASE} --fast
```

prints `GATE PASSED` (the main session runs the full gate with clippy and `--polar`, and records
the round). Never pass `--lock-base`. If the Polar corpus exists (`target/corpus/polar/server`),
also time it: `bash scripts/hillclimb/polar_time.sh target/release/hydroid 3 /dev/null --no-cache`
(cold) and, after one run that fills the cache, `bash scripts/hillclimb/polar_time.sh
target/release/hydroid 3` (warm).
Do not commit. Return: what the gap was, how you fixed it (files), which cases now pass, gate
output summary (including the "other added literals" list), cold and warm Polar times if measured,
for a new catalog concept its 2 example libraries, and anything you could not fix.
