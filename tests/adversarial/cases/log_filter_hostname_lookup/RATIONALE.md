# log_filter_hostname_lookup

A `logging.Filter` subclass stamps every record with `socket.getfqdn()` (a reverse DNS lookup) for
the log aggregator. A startup handler attaches it to a `StreamHandler` with
`handler.addFilter(...)`. In the async endpoint, `logger.info(...)` runs
`Logger._log -> handle -> callHandlers -> Handler.handle -> Filterer.filter -> HostContextFilter.filter`
synchronously, so the DNS lookup blocks the loop.

hydroid likely misses it: the override `filter` is only reachable through stdlib logging's
dispatch over filters registered at runtime; no project code calls it by name. Distinct from a
handler `emit` override: the blocking code sits in a Filter attached to a handler, not in the
handler itself (the handler is a plain stdlib `StreamHandler`).

Labels: line 33 (`logger.info`) - the oracle confirms `socket.getfqdn`. Negatives: the
`asyncio.to_thread(logger.info, ...)` call offloads; `logger.debug` is below the logger level
(INFO) and returns before any filter runs; the `def list_orders` endpoint runs in the threadpool.
