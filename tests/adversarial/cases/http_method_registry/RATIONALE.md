# Bound `requests.Session` methods stored in a dispatch dict

A gateway keeps `{"GET": session.get, "POST": session.post, ...}` and picks the method by verb.
Calling `DISPATCH["GET"](...)` or a local alias of it from an `async def` route performs a sync
HTTP request on the event loop.

hydroid does not track bound methods through dict values, so these calls stay unresolved and are
not reported. Labels: the oracle sees `requests.sessions.Session.get` / `.post` on the loop at
lines 19 and 25. The `def` route uses the same dict in the threadpool (negative).
