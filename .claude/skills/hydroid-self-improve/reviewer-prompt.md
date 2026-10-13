You review adversarial test cases for hydroid (a static analyzer that reports blocking calls
running on the asyncio event loop in FastAPI apps) that the runtime oracle could not fully prove.

Cases: {CANDIDATES} (each in `tests/adversarial/candidates/<name>/`: `app.py`, `RATIONALE.md`,
`CHECK.json`). See their state with `python3 scripts/hillclimb/check.py <dir> -v`.

For each disputed line (`runtime: unreached` or `runtime: doubt` where hydroid disagrees):

1. First try to make the oracle reach it: the oracle calls every route once with sample inputs
   from the OpenAPI schema (`"1"`, `1`, `b"data"`), runs the lifespan and websockets, and replaces
   blocking calls on the loop with a stand-in (`"{}"` that accepts any use). Reorder or simplify
   the case so the line runs, without changing the mechanism under test. Re-run `check.py`.
   `BREAKS` means proven: done.
   The oracle does not record calls that cannot block: a call returning an awaitable, an
   operation on a non-blocking socket, a call giving an `unless_given` argument (catalog), and
   I/O done wholly inside library code below a library coroutine the case awaits.
2. If the line truly cannot run under the oracle (an exception handler, a path needing real
   data), judge it by FastAPI / Starlette / Python semantics. Read the library source in
   `tests/fixtures/.venv/lib/python*/site-packages/` to confirm on which thread the code runs.
   Do not read `crates/` (hydroid's source) - judge runtime truth, not hydroid's design.

Return per case: `PROVEN` (now BREAKS), `CORRECT` (labels right, give the evidence: file and line
in the library source), or `WRONG` (labels wrong, why). Do not promote anything yourself.
