# fp_streaming_genexp - false positive

A generator expression passed to `StreamingResponse` is consumed off the loop. Hydroid reports
the blocking call inside the generator expression body (lines 23 and 29) as if it ran when the
expression is created in the `async def`.

Python semantics: a generator expression evaluates only its outermost iterable (`MIRRORS`,
`range(n)`) eagerly; the element expression (`fetch_part(url)`, `time.sleep(...)`) runs only
when the generator is advanced.

Starlette (starlette 1.7.0):
- `tests/fixtures/.venv/lib/python3.13/site-packages/starlette/responses.py:235-238`: a content that is not an `AsyncIterable` is wrapped
  in `iterate_in_threadpool(content)`.
- `tests/fixtures/.venv/lib/python3.13/site-packages/starlette/concurrency.py:51-57`: `iterate_in_threadpool` calls `iter()` on the loop
  (a no-op for a generator) and advances it with `anyio.to_thread.run_sync(_next, ...)`
  (`_next` at line 41-46 calls `next(iterator)`), i.e. every element is computed in a worker thread.

Runtime evidence: the oracle calls `/bundle` and `/ticks` and streams the body; the lines run
(the generator is created on the loop) but no catalog function is recorded on the loop thread
(`runtime: safe: ran on the loop, nothing blocked`); `requests.get`/`time.sleep` happen in the
anyio worker thread. Control: `/eager` passes a generator expression to `b"".join` in the
`async def`, which consumes it on the loop: labeled, hydroid and runtime agree.

Verdict: BREAKS.
