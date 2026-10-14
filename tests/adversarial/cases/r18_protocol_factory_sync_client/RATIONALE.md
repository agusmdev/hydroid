# r18_protocol_factory_sync_client

A typed factory `build_transport() -> Transport` returns a concrete `httpx.Client` behind a
`typing.Protocol` interface. `httpx.Client` satisfies the Protocol structurally (no inheritance),
so the static type at the call site `transport.get(...)` is the Protocol method, whose body is
`...`. Unless the analyzer follows the factory's actual returned value (or matches library classes
structurally against the Protocol), it sees no blocking implementation.

At runtime `transport` is an `httpx.Client`, so `transport.get` is `httpx._client.Client.get`,
a sync HTTP request on the loop thread. Negatives: the same call offloaded with
`asyncio.to_thread`, and an `httpx.AsyncClient` behind an async Protocol, awaited.
