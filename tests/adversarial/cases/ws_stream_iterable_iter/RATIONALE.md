# ws_stream_iterable_iter - miss

`StreamingResponse(content)` with a sync iterable wraps it in `iterate_in_threadpool`
(tests/fixtures/.venv/lib/python3.13/site-packages/starlette/responses.py:235-238). That async generator runs on the loop and calls
`iter(iterator)` there (tests/fixtures/.venv/lib/python3.13/site-packages/starlette/concurrency.py:52) before offloading each `next()` to a worker
thread. Only the `next()` calls go to the thread. A user iterable whose `__iter__` is a plain
method that does the work up front (open a DB connection, run the query, return an iterator over
the rows) runs that work on the loop.

hydroid treats everything inside a sync iterable passed to `StreamingResponse` as threadpool
code (or does not model `iter()` as called on the loop), so it misses `sqlite3.connect` in
`CsvExport.__iter__`.

Near miss: `LazyCsvExport.__iter__` is a generator function, so `iter()` only creates the
generator and the query runs in the thread at the first `next()`. No label, and the runtime
shows nothing blocking on the loop there.

Runtime: the oracle streams `/export.csv` and records `_sqlite3.connect` on the loop thread at
line 19.
