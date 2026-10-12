You improve hydroid, a Rust static analyzer (built on ty's semantic model) that reports blocking
calls reachable on the asyncio event loop in FastAPI code. Read `CLAUDE.md` if present,
`README.md`, `docs/GUIDE.md`, and the code under `crates/` as needed.

Mechanism to fix: **{MECHANISM}**. Failing cases (split `open`): {CASES}.
See them fail: `cargo build --release -p hydroid && python3 scripts/hillclimb/score.py --split train`
lists each open case's misses and false positives. One case in full:
`target/release/hydroid tests/adversarial/cases/<name> --python tests/fixtures/.venv --no-cache`.
Each case's `RATIONALE.md` says what runs on the loop and why.

## Rules

- Fix the general mechanism, not the cases. No special cases for case names, file names, line
  numbers or variable names. A catalog entry (`crates/hydroid_core/catalog.toml`) is fine when the
  gap is a missing blocking API, and must name the real defining function.
- You may change `crates/` (except `crates/hydroid/tests/fixtures.rs` and
  `crates/hydroid/tests/common/`), the catalog, docs, and add regression fixtures under
  `tests/fixtures/cases/` (a fixture per new behavior is welcome).
- You must NOT read these held-out cases: {HOLDOUT}. You must NOT edit `scripts/hillclimb/`,
  `tests/adversarial/`, `tests/fixtures/pyproject.toml`, `tests/fixtures/uv.lock`,
  `.claude/hillclimb/`, `.claude/skills/`. The gate rejects such diffs.
- No false positives anywhere: precision matters more than recall. Performance matters: hydroid
  runs on large codebases (Polar: ~1,900 files); avoid whole-program passes that scale badly.
- Match the surrounding code style and comment density.

## Done when

```sh
python3 scripts/hillclimb/gate.py --base {BASE} --fast
```

prints `GATE PASSED` (the main session runs the full gate with clippy and records the round).
Do not commit. Return: what the gap was, how you fixed it (files), which cases now pass, gate
output summary, and anything you could not fix.
