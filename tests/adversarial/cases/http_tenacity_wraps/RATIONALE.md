# Sync tenacity wrapper built with `Retrying.wraps`

Instead of `@retry`, a shared `Retrying` policy wraps a sync function: `safe_parse =
retrying.wraps(parse_payload)`. Calling `safe_parse` goes through `Retrying.__call__`, whose waits
between attempts use `time.sleep` - on the event loop when called from an `async def` route.

hydroid knows `tenacity.Retrying.__call__` and the `@retry` decorator, but not the callable
returned by `Retrying.wraps`, so the call is not reported. Runtime: the oracle records
`tenacity.Retrying.__call__` on the loop at line 21. The `asyncio.to_thread` call is a negative.
