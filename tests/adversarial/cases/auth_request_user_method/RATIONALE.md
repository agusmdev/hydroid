# auth_request_user_method

The auth backend returns a custom `BaseUser` subclass (`DbUser`) whose `has_role` method queries
sqlite. Starlette stores it in `scope["user"]`, and `request.user` returns it - but Starlette types
`request.user` as `Any`.

`async def articles` calls `request.user.has_role("reader")` on the loop, which runs
`sqlite3.Connection.execute` (oracle confirms line 44). The backend always returns a `DbUser`
(header missing -> `"guest"`), so the oracle reaches the line.

Why hydroid likely misses it: `request.user` is `Any`, so the call is unresolved. Hydroid already
recovers the stored type for `request.state.x` / `request.app.state.x`, but not for
`request.user` / `request.auth`, whose value is the second / first element of the tuple returned
by the `AuthenticationBackend.authenticate` of the backend given to `AuthenticationMiddleware`.

Negative: `def articles_sync` does the same in the threadpool.
