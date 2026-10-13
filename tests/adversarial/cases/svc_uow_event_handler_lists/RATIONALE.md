Domain events collected by a unit of work and dispatched on commit through a module-level
registry `EVENT_HANDLERS = {EventType: [handler, ...]}` — a dict literal whose values are LISTS
of callables, looked up by `type(event)` and iterated with a nested `for`. This is the classic
"message bus" layout (cosmic-python style) used in layered FastAPI services.

Why hydroid likely misses it: dispatch tables are resolved when the subscript result is called
directly (`TABLE[k](...)`), but here the subscript yields a list that is iterated, and the loop
variable is called. The handler `send_welcome_email` does a sync `requests.post`.

Runtime: `POST /users` runs `UserService.register` (async) -> `async with uow` -> `__aexit__` ->
`commit()` (async, on the loop) -> `handler(event)` -> `requests.post` on the loop thread. The
oracle confirms `requests.api.post` at the labeled line. `/users/offloaded` sends the same handler
through `asyncio.to_thread` (negative).
