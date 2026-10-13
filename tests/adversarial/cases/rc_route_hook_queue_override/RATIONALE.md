# Route-class hook overridden by a non-blocking subclass (false positive)

`FileAuditRoute.get_route_handler` returns the classic FastAPI-docs wrapper (time the request,
then `self.record(entry)`). The base `record` appends to a file; the subclass
`QueuedAuditRoute.record` only does `asyncio.Queue.put_nowait`. The only router uses
`route_class=QueuedAuditRoute`, and FastAPI constructs the route objects itself
(`self.route_class(...)` in `APIRouter.add_api_route`), so every `self` in the wrapper is a
`QueuedAuditRoute`: the per-request `self.record(entry)` call never touches the disk.

hydroid likely resolves `self.record` from the base class's closure to the base implementation
(no project construction of the route class narrows `self`) and reports `pathlib.Path.open` on
line 28. The oracle runs both routes: line 28 runs on the loop and nothing blocks.

A plain `/healthz` route is registered first so FastAPI's one-time `inspect.getsourcelines`
(linecache read of app.py, for endpoint error context) happens on it, not inside the wrapper.
