# flow_event_bus_hooks

An in-process event bus: `@bus.on("user.created")` registers sync handlers by appending them to a
`defaultdict(list)` stored on the instance (`self._handlers[event].append(fn)`); `bus.emit(...)`
later iterates that list and calls each handler. The callable flows into a container through a
method call made by a decorator factory, then out of it via iteration, so it is neither a callable
stored directly on `self` nor a dict/list literal dispatch table. hydroid likely sees `handler` as
an unresolved callable and misses `requests.post` inside `notify_crm`.

Runtime: `create_user` (async) calls `bus.emit` on the loop, which runs `notify_crm` ->
`requests.post` on the loop thread (oracle confirms line 48). Negatives: `asyncio.to_thread(bus.emit, ...)`
and the sync `def` endpoint run in threads.
