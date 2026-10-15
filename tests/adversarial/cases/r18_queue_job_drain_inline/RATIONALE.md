# r18_queue_job_drain_inline

A home-grown in-process job queue: `enqueue(fn, *args)` wraps the callable in a `Job` dataclass
and puts it on a stdlib `queue.Queue[Job]`; `run_pending()` drains the queue with `get_nowait()`
and calls `job.fn(...)` in the calling thread. The `upgrade` endpoint enqueues `notify_billing`
and then drains the queue inline from its `async def`, so `requests.post` runs on the event loop.

Why hydroid likely misses it: the callable reaches `job.fn(...)` only through a dataclass field
stored in a `queue.Queue` and read back with `get_nowait()`; the static target of `job.fn` is
just `Callable[..., Any]`, so the flow `enqueue(notify_billing)` -> `Job.fn` -> call in
`run_pending` is lost.

Runtime: the oracle records `requests.api.post` on the loop thread at line `ran = jobs.run_pending()`.
`get_nowait`/`put_nowait` never block (no label). The `/upgrade-offloaded` route drains the same
queue via `asyncio.to_thread` (negative).
