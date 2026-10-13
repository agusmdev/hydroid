# logging.handlers.SMTPHandler: logger.error() sends an email synchronously

Attaching an `SMTPHandler` for alert emails is a classic ops setup. `logger.error(...)` dispatches
synchronously to every handler at or above its level; `SMTPHandler.emit` constructs
`smtplib.SMTP(mailhost, port, timeout=...)` (connect), then `send_message` and `quit`. From an
`async def` endpoint this blocks the event loop for the full SMTP round trip.

hydroid likely misses it because the blocking happens behind `Logger.error` -> `Handler.handle` ->
`SMTPHandler.emit`, dynamic dispatch through stdlib logging configured at import time.

Labels: runtime confirms `smtplib.SMTP.__init__` (and send_message/quit) on the loop at line 34.
Negatives: `alerts.info(...)` is below the handler's ERROR level so it never emits; the `audit`
logger uses the recommended `QueueHandler` (non-blocking `put_nowait`), with the SMTP handler on a
`QueueListener` thread.
