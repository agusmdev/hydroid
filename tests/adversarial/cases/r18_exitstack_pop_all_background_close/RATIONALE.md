# ExitStack.pop_all hands the exits to a stack closed after the response, in the threadpool

`stack.enter_context(cm)` calls `cm.__enter__()` at once, but `cm.__exit__` only runs when the
stack holding it is closed. `stack.pop_all()` moves the registered exits to a new `ExitStack`, so
the outer `with` closes an empty stack. The new stack is closed by
`BackgroundTask(deferred.close)`: Starlette runs a sync background callable with
`run_in_threadpool`, so `AuditSpan.__exit__` (open/write of the audit log) runs in a worker thread.
`AuditSpan.__enter__` only reads `time.monotonic()`.

hydroid reports `builtins.open` at the `enter_context` lines, as if entering also ran the exit
in the calling thread.

Control: `/files-inline/{name}` keeps the exits on the stack, so the `with ExitStack()` exit runs
`AuditSpan.__exit__` on the loop (runtime: `builtins.open`, `typing.IO.write` at the `with` line).
