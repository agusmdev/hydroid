# flow_global_hook_rebind

A module-level hook `track` defaults to a no-op and is rebound at import time by
`configure_tracking()` via `global track; track = _post_to_collector`. The async route calls
`track(...)`. Statically, the declared/initial binding is `_discard` (harmless); the blocking
binding only arrives through a `global` rebinding inside another function. hydroid appears to
resolve `track` only to its module-level assignment and misses `requests.post`.

Runtime: with `TRACKING_DISABLED` unset (the default), `track` is `_post_to_collector` when routes
run; the oracle sees `requests.api.post` on the loop at line 33. Negatives: `asyncio.to_thread(track, ...)`
and the sync `def health` endpoint run in the threadpool.
