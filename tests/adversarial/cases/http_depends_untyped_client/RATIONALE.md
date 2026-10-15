# Unannotated `Depends` parameters holding sync HTTP clients

Dependency factories (`get_billing_client` yields an `httpx.Client`, `get_session` returns a
configured `requests.Session`) are sync, so FastAPI runs them in the threadpool - that part is
fine. But the client they produce is injected into an `async def` endpoint whose parameter has no
annotation (`client=Depends(get_billing_client)`), a very common style. The `.get(...)` call in the
endpoint body runs on the event loop and blocks.

hydroid apparently does not infer the parameter's type from the dependency's return/yield value,
so the call is left unresolved and nothing is reported.

Labels: at runtime the oracle sees `httpx._client.Client.get` and `requests.sessions.Session.get`
called on the loop thread from lines 21 and 27. The `def` endpoint `/invoices-sync` uses the same
client in the threadpool and is a negative.
