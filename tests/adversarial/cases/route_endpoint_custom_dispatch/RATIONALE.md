# route_endpoint_custom_dispatch

A project base view `APIEndpoint(HTTPEndpoint)` overrides `async def dispatch` to map upstream
errors to 502. Starlette's own `HTTPEndpoint.dispatch` runs sync handlers with `run_in_threadpool`,
but the override calls the handler directly: `handler(request)`. The subclass `PricesView` has a
sync `get` that calls `requests.get`, so it now runs on the event loop inside the async `dispatch`.
Label goes on line 17, the call in the innermost loop function (`dispatch`) that enters the sync
code.

`PingView` uses the stock `HTTPEndpoint.dispatch`, so its sync `get` (with `time.sleep`) runs in the
threadpool: near-miss negative.

hydroid likely misses it: the handler is chosen with `getattr(self, request.method.lower(), ...)`
on a subclass instance, and a sync method of an `HTTPEndpoint` looks like threadpool code.
Oracle: line 17 confirmed (`requests.api.get` on the loop); `PingView.get` runs off the loop.
