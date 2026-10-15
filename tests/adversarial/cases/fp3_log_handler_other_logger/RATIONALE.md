# Handler attached only to a non-propagating named logger

The Slack handler (which does a blocking `requests.post` in `emit`) is attached only to the
`audit` logger, which has `propagate = False`. The app logs from other loggers:
`logging.getLogger(__name__)`, a helper returning `logging.getLogger(f"svc.{name}")`, and the
root logger via `logging.warning(...)`.

hydroid reports all three log calls as reaching `SlackAuditHandler.emit`, as if every handler in
the project ran for every logger. It apparently ignores the logger hierarchy (and treats
`__name__` / f-string / root loggers as possibly being `audit`).

## Why the labels are right

`Logger.callHandlers` (CPython 3.13, `logging/__init__.py:1730-1758`) walks only the handlers of
the emitting logger **and its parents**, stopping at a logger with `propagate` false:

```python
c = self
while c:
    for hdlr in c.handlers:
        ...
        hdlr.handle(record)
    if not c.propagate:
        c = None
    else:
        c = c.parent
```

`audit` is not an ancestor of `app` (or whatever `__name__` is), `svc.payments`, or the root
logger, so its handlers are never visited for those records. Root has no handlers, so those
records go to `lastResort` (stderr). Only `audit_log.info(...)` in `delete_user` reaches the Slack
handler (`audit` level is INFO, handler level NOTSET), which the oracle confirms; the other three
lines ran on the loop and nothing blocked.
