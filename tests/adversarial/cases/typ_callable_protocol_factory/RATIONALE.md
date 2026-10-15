# Callable Protocol (`__call__`) chosen by a factory

`RateFetcher` is a `typing.Protocol` with only `__call__(self, currency) -> float`. The factory
`make_rate_fetcher() -> RateFetcher` returns a plain module function (`fetch_live_rate`, which
calls `requests.get`, or `fixed_rate` under `RATES_MODE=fixed`). Plain functions satisfy a
callable Protocol structurally. The result is used both as a module-level value
(`get_rate: RateFetcher = make_rate_fetcher()`) and as a FastAPI dependency
(`fetcher: RateFetcher = Depends(make_rate_fetcher)`).

Likely hydroid error: calling a value typed as a callable Protocol resolves to the Protocol's
`__call__` stub (or nothing) instead of the functions the factory returns, so the
`requests.get` reached through `get_rate(...)` / `fetcher(...)` is missed. Structural method
Protocols (`store.read()`) are handled; the callable form is not.

Runtime: `RATES_MODE` is unset, so the factory returns `fetch_live_rate` and both async routes
call `requests.get` on the loop (confirmed by the oracle). The `def` route runs in the
threadpool and is not labeled.
