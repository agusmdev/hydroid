# asyncio protocol callbacks run on the event loop

`loop.create_datagram_endpoint(ProtocolFactory, ...)` (also `create_server`,
`create_connection`, `subprocess_exec`...) takes a protocol class; the loop instantiates it
and calls its *sync* methods (`connection_made`, `datagram_received`, `data_received`,
`connection_lost`, `error_received`) directly on the loop thread. A FastAPI lifespan that
starts a UDP cache-invalidation listener this way runs `connection_made` on the loop right
after binding; its `Path.write_text` blocks the loop.

hydroid knows sync callbacks passed to `call_soon`/`call_later`/`add_reader`... but not
protocol factories passed to the loop's transport APIs, so nothing is reported.

Label: the blocking call inside `connection_made` (a sync method the loop calls). The runtime
confirms it. Near misses: `warm_up` handed to `run_in_executor` (worker thread),
`datagram_received` (non-blocking), the async endpoint.
