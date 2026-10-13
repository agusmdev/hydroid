`contextvars.Context.run(fn, *args)` calls `fn` synchronously in the current thread (with the
copied context). Code that wants to isolate contextvar changes (request id, tenant) calls
`copy_context().run(push_metrics, name)` from an `async def` route, so `push_metrics` and its
`requests.post` run on the loop. hydroid does not treat `Context.run` as a function that calls its
argument, so the flow is dropped. Runtime confirms `requests.api.post` on the loop at line 24.
Negatives: `ctx.run(render_label, ...)` (pure), and `run_in_threadpool(ctx.run, push_metrics, ...)`,
which offloads the whole thing (the oracle error for that route is the off-loop network failure,
expected).
