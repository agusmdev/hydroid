# fp2_asyncify_decorator

Mechanism: an "asyncify" decorator (`run_in_thread`) replaces a blocking `def` with an
`async def wrapper(*args, **kwargs)` that forwards everything to `asyncio.to_thread(fn, ...)`.
After decoration the module-level names `fetch_profile` / `render_report` are bound to the
wrapper, so `await fetch_profile(user_id)` runs only the wrapper on the loop; the original body
(`requests.get`, `time.sleep`) runs in the default executor.

Library evidence: `asyncio/threads.py` `to_thread` does `func_call = functools.partial(ctx.run,
func, *args, **kwargs)` then `await loop.run_in_executor(None, func_call)` (lines ~24-26). The
decorator semantics are plain Python: `@run_in_thread` rebinds the name to its return value.

hydroid (likely) resolves `fetch_profile(...)` to the decorated function's original body and
reports `requests.api.get` / `time.sleep` at the `await` lines. Runtime: both endpoints run, the
oracle sees no catalog call on the loop thread at lines 41 and 46. Positive control: the raw
`time.sleep` on line 47 in the async endpoint.
