# Backoff helper whose sleep function is a keyword default argument

`with_backoff(fn, *, sleep=time.sleep)` is a common testable retry helper: the sleep function is
injectable, and production callers rely on the default. Inside, the call is `sleep(...)`, a call
through a parameter. When the caller passes nothing, the parameter is bound to the default value
`time.sleep`, so the async endpoint `/quota` sleeps on the event loop between attempts (`Flaky`
fails once, so the sleep always runs).

hydroid likely misses it because the callee of `sleep(...)` is a parameter, and the parameter's
default value (a function object) is not used as the target when the call site omits the argument.

Labels: runtime confirms `time.sleep` on the loop at the `with_backoff(Flaky())` call (line 43).
Negatives: the async variant awaits `asyncio.sleep`; `/quota/thread` offloads via `asyncio.to_thread`.
