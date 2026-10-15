# route_path_convertor

A custom Starlette path convertor (`register_url_convertor("tenant", TenantConvertor())`) is used
in a FastAPI path `/t/{tenant:tenant}/...`. While routing, Starlette's `Route.matches` (an
`APIRoute` is a `Route`) calls `convertor.convert(value)` for each path parameter, synchronously,
inside `Router.__call__` on the event loop, before any endpoint runs. So the sync `cache.get`
(a sync `redis.Redis`) blocks the loop for every request to either route, including the `def`
endpoint `export` (the threadpool only covers the endpoint body, not route matching).

hydroid likely misses it: `convert` is never called from project code; it is a framework callback
reached through the global convertor registry and a `{name:type}` path string.

Labels: line 14 blocks on the loop (oracle confirmed). Both endpoints are clean: `dashboard` does
nothing blocking, `export` is a sync endpoint.
