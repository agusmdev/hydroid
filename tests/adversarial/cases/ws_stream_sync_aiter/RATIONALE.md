# ws_stream_sync_aiter - miss

An object implementing `AsyncIterable` (`__aiter__`) is used as is as the body iterator
(tests/fixtures/.venv/lib/python3.13/site-packages/starlette/responses.py:235-236). `stream_response` then runs `async for chunk in self.body_iterator`
on the loop (tests/fixtures/.venv/lib/python3.13/site-packages/starlette/responses.py:251). `async for` calls `__aiter__` **synchronously**. It is a
plain `def` and it runs on the loop thread. A log follower for server-sent events that opens its
file in `__aiter__` blocks the loop with `open()`.

hydroid misses the sync `__aiter__` that `StreamingResponse` runs on the loop.

Near miss: `OffloadedLogFollower` has a trivial `__aiter__` and opens the file in `__anext__`
through `await asyncio.to_thread(open, ...)`. No label.

Runtime: the oracle streams `/logs/follow` and records `builtins.open` on the loop at line 21.
