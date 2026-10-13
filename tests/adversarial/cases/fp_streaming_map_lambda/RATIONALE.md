# fp_streaming_map_lambda - false positive

`map(f, xs)` (and `filter`) is lazy in Python 3: it calls `f` only when the map object is
advanced. Handed to `StreamingResponse`, it is advanced in a worker thread. Hydroid treats the
callable given to `map` as called at the `map(...)` call site (lines 27 and 33), both for a lambda
and a plain function, and reports `time.sleep` on the loop.

Starlette (starlette 1.7.0):
- `tests/fixtures/.venv/lib/python3.13/site-packages/starlette/responses.py:235-238`: a non-`AsyncIterable` content is wrapped in
  `iterate_in_threadpool`.
- `tests/fixtures/.venv/lib/python3.13/site-packages/starlette/concurrency.py:51-57` + `_next` at 41-46: `iter()` on the loop (for a map
  object it returns itself, running nothing), then every `next()` through
  `anyio.to_thread.run_sync`, so `load_row` runs in the threadpool.

Runtime evidence: the oracle calls both streaming routes with `n=1`; the lines run on the loop
but no catalog function is recorded on the loop thread. Control: `/export-eager.csv` does
`list(map(load_row, ...))` in the `async def`, consuming the map on the loop: labeled, hydroid
and runtime agree. (Same mis-modeling seen for `filter(lambda ..., ...)`; `itertools.starmap`
and `iter(callable, sentinel)` are already treated as lazy by hydroid.)

Verdict: BREAKS.
