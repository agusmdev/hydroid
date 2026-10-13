# anyio.from_thread.run_sync runs its callable on the event loop

Starlette runs sync endpoints and sync background tasks in worker threads with
`anyio.to_thread.run_sync`. Code there that needs loop-owned state (an `asyncio.Event`, an
in-memory registry only touched from the loop) hops back with `anyio.from_thread.run_sync(fn,
*args)`. anyio then runs `fn` *on the event loop thread* (it schedules it with
`call_soon_threadsafe` and waits), so a blocking call inside `fn` blocks the loop, even though
the caller is a sync `def` running in the threadpool.

hydroid treats `anyio.from_thread.run_sync` like any other call made from threadpool code, so
`FeatureFlags.reload` (called from a sync endpoint) and `FeatureFlags.set_flag` (called from a
sync background task) are never reported. It is the reverse of an offload API: it moves a sync
callable onto the loop, like `loop.call_soon_threadsafe`.

Labels: the blocking call inside each callable (sync functions that run on the loop). The
runtime confirms both. Near misses: the sync endpoint body itself and the background job body
(threadpool), `anyio.from_thread.run(flags.persist)` whose coroutine offloads with
`asyncio.to_thread`, and `_write` running in that worker thread.
