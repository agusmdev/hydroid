# `Thread.run()` instead of `Thread.start()` runs the target on the loop

`threading.Thread(target=f, args=...)` only stores `f`; `start()` runs it in a new thread, but
`run()` calls `self._target(*self._args, **self._kwargs)` in the *calling* thread. Calling `run()`
by mistake is a common bug: in an `async def` it runs the webhook POST on the event loop.

hydroid treats `threading.Thread.__init__` as an offload (the target is never followed), so the
`worker.run()` call that actually executes the target is missed.

Labels: `worker.run()` in `create_order` (the line in the async def that calls into the blocking
target); the runtime sees `requests.api.post` there. Near miss: the same thread with `start()`
in `create_order_bg` runs the POST in a worker thread, off the loop.
