# life_app_state_store

The lifespan builds a `SessionStore` (offloaded with `asyncio.to_thread`, correctly) and stores it
on `app.state.sessions`. The async endpoint uses it as `request.app.state.sessions.lookup(sid)`.
Both `Request.app` and `State.__getattr__` are `Any`, so hydroid cannot see that `lookup` runs
`sqlite3.Connection.execute` on the loop. Following the single assignment to `app.state.sessions`
in the lifespan registered on this app gives the type.

Runtime: GET /me runs `lookup` on the loop thread; the oracle records
`sqlite3.Connection.execute` at line 29. The `def` endpoint runs in the threadpool (negative).
