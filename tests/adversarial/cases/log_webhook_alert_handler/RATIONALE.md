# log_webhook_alert_handler

A custom `logging.Handler` subclass posts ERROR records to a chat webhook with `requests.post`
inside `emit`. A startup handler attaches it with `logger.addHandler(...)`. The async endpoint
calls `logger.error(...)`: the stdlib runs `Logger.error -> _log -> handle -> callHandlers ->
Handler.handle -> WebhookAlertHandler.emit` synchronously on the calling thread, so the HTTP
request blocks the loop.

hydroid likely misses it: the call path goes through stdlib `logging` internals (not analyzed by
default), and the user override `emit` is reached only through the `Handler` registered at
runtime, never called by name from project code.

Labels: line 38 (`logger.error`) is the call into blocking code from the async endpoint; the
oracle confirms `requests.api.post`. Negatives: `logger.info` on line 37 is below the handler level
(ERROR), so `Handler.handle` never calls `emit`; `asyncio.to_thread(logger.error, ...)` offloads;
the `def capture` endpoint runs in the threadpool.
