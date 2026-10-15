# fp2_running_loop_branch

Mechanism: a sync helper `emit_audit` shared by async handlers and sync code checks
`asyncio.get_running_loop()`. Only when it raises `RuntimeError` (no loop running in this
thread) does it fall back to a blocking `requests.post`; with a running loop it does
`queue.put_nowait` and returns. Called from an `async def` endpoint the loop is always running,
so the blocking branch is dead on the loop.

Library evidence: `asyncio/events.py` `get_running_loop` (line ~758) is thread-specific and raises
`RuntimeError('no running event loop')` only when `_get_running_loop()` is None. FastAPI runs
`def` endpoints with `run_in_threadpool` (`fastapi/routing.py` `run_endpoint_function`, line ~354),
where no loop is running, so `refund_order` legitimately takes the sync branch off the loop.

hydroid reports `requests.api.post` at line 36 (the call in `cancel_order`). The oracle runs
`/orders/1/cancel` and sees no blocking call on the loop. This differs from the known
`nested_loop_guard` case (which is about `asyncio.run` in the no-loop branch); here the guarded
call is ordinary blocking I/O. Positive control: line 49.
