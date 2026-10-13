The idiomatic PyMySQL pattern `with conn.cursor() as cur: cur.execute(...)` inside an async
endpoint. `Cursor.__enter__` returns `self`, so `cur` is the cursor and `execute` / `fetchone`
are `pymysql.cursors.Cursor` methods doing socket I/O on the loop. hydroid flags
`conn.cursor().execute` when the cursor is bound by assignment but loses the type when it is
bound by the `with ... as` target (pymysql's `Cursor.__enter__` is unannotated).

Near-misses: the same code in a helper run with `asyncio.to_thread`, and a sync endpoint
(threadpool). `defer_connect=True` makes the module-level connection importable without a server;
the runtime records `Cursor.execute` and `Cursor.fetchone` on the labeled lines.
