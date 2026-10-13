# mw_route_handler_per_request

A custom `APIRoute` subclass (via `APIRouter(route_class=...)`, the pattern from FastAPI's
"Custom Request and APIRoute class" docs) reads a JSON policy file in its sync
`get_route_handler()`. It looks like import-time setup (and `APIRoute.__init__` does call it
there, off the loop), but in the installed FastAPI 0.141.1, a route reached through
`include_router` carries an "effective route context", and `APIRoute.handle()` rebuilds the
handler on every request:

```python
# fastapi/routing.py:1262-1279 (APIRoute.handle, async)
effective_context = _get_scope_effective_route_context(scope)
if effective_context is not None and effective_context.original_route is self:
    ...
    app = request_response(self.get_route_handler())
```

So the `Path.read_text()` in `get_route_handler` runs on the event loop for each request.

hydroid likely does not treat `get_route_handler` overrides as code the framework calls on the
loop (only the async handler they return).

Labels: line 24 is the blocking call in a sync method the framework calls on the loop (the oracle
confirms `pathlib.Path.read_text`). Negative: `/policy` reads the file via `asyncio.to_thread`.

Note on the case shape: the handler is not wrapped in an `async def custom_route_handler` that
awaits the original, because then the oracle records a `builtins.open` on the await line: FastAPI
calls `inspect.getsourcelines(endpoint)` on the first request of each endpoint
(`_extract_endpoint_context`, fastapi/routing.py:290), reading the source via linecache on the
loop, and that library-internal read is attributed to the innermost case frame.
