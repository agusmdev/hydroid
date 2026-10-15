# mw_middleware_init_io

A `BaseHTTPMiddleware` subclass overrides `__init__` to load an allowlist file with
`Path.read_text()`. Middleware classes registered with `app.add_middleware(Cls, **kw)` are not
instantiated at registration: Starlette stores a `Middleware(cls, ...)` spec and builds the stack
lazily in `Starlette.__call__` (`if self.middleware_stack is None: self.middleware_stack =
self.build_middleware_stack()`, starlette/applications.py:94-95; FastAPI's override at
fastapi/applications.py:1067 does `cls(app, *args, **kwargs)`). That first `__call__` is the
lifespan event (or first request), on the event loop, so the sync constructor blocks the loop.

hydroid likely treats the class passed to `add_middleware` only through its async `dispatch` /
`__call__`, and not the constructor that the framework calls on the loop.

Labels: line 22 is the blocking call in a sync function the framework calls on the loop (the
oracle confirms `pathlib.Path.read_text` there). Negative: the endpoint re-reads the file through
`asyncio.to_thread`.

Note on the case shape: a pure ASGI middleware (`async def __call__` awaiting `self.app`) makes the
oracle record a `builtins.open` on the `await self.app(...)` line: FastAPI 0.141 calls
`inspect.getsourcelines(endpoint)` on the first request of each endpoint
(`_extract_endpoint_context`, fastapi/routing.py:290), which reads the source file via linecache
on the loop. That is library behaviour, not something to label, so the case uses
`BaseHTTPMiddleware`, whose `call_next` runs the downstream app in a separate task.
