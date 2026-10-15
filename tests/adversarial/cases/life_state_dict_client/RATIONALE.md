# life_state_dict_client

The lifespan yields a state dict (`yield {"billing": BillingClient(...)}`); Starlette copies it into
each request's scope, and the endpoint reads it back as `request.state.billing`. `State.__getattr__`
is typed `Any`, so the type of `billing` is unknown statically, and `billing.invoice(...)` (a sync
`requests.Session.get`) inside an `async def` endpoint is missed.

Runtime: the oracle runs the lifespan, then GET /invoices/1 calls `BillingClient.invoice` on the
loop thread, which calls `requests.sessions.Session.get` (line 28 label). The `asyncio.to_thread`
variant and the sync `def` endpoint run in threads (negatives). The only object ever stored under
`"billing"` is the `BillingClient` yielded by the lifespan, so the call target is determined.
