# r18_sched_scheduler_run

Stdlib `sched.scheduler(time.monotonic, time.sleep)` is a tiny in-process job scheduler.
`scheduler.run()` (blocking=True, the default) waits for each event by calling its `delayfunc`,
here `time.sleep`, in the calling thread. Calling it from an `async def` (directly, or through a
small `Maintenance.drain()` helper) sleeps on the event loop until every planned job has run.

Why hydroid likely misses it: stdlib bodies are not analyzed, and `sched.scheduler.run` is not in
the catalog; the `time.sleep` it calls is only passed as a constructor argument (stored as
`delayfunc`), never called in project code.

Runtime: `time.sleep` recorded on the loop at both labeled lines (events are planned 20-40 ms
ahead, so `run()` must sleep). `run(blocking=False)` (in `tick`, unused) never sleeps, and the
`/maintenance/offloaded` route runs `drain` via `asyncio.to_thread` (negative).
