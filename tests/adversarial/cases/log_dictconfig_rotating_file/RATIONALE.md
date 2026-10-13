# log_dictconfig_rotating_file

`logging.config.dictConfig` (run in a startup handler) builds a stdlib
`logging.handlers.RotatingFileHandler` named by a string (`"class": ...`) with `delay=True`, the
common config to avoid creating empty log files. With `delay=True` the file is opened by the first
`emit` (`FileHandler.emit -> self._open() -> open(...)`), and `RotatingFileHandler.shouldRollover`
also touches the filesystem; all of it runs synchronously inside `audit.info(...)` on whatever
thread logs. In the async endpoint, that is the event loop.

hydroid likely misses it: the handler class only appears as a string in a config dict, and the
blocking work is inside stdlib logging (`Logger.info -> ... -> FileHandler.emit`), which no project
function calls by name.

Labels: line 54 (`audit.info`) is the first record on the `audit` logger, so the oracle records
`builtins.open` there. Negatives: the offloaded `asyncio.to_thread(security.info, ...)` uses a
separate logger/handler (`security.log`) so route order cannot change which call opens the audit
file; `audit.debug` is below the logger level and returns before creating a record.
