# mod_importlib_plugins

The audit backend is a module chosen by dotted path at import time
(`audit = importlib.import_module(AUDIT_BACKEND)`, the Django `import_string` pattern). Statically
the value is just `ModuleType`, so `audit.record(...)` has no resolvable target and hydroid
reports nothing. The default path is a string literal in the same file (`plugins.audit_file`),
whose `record` opens and appends to a file.

Runtime: `create_order` is `async def`, so `audit.record` runs on the loop and calls
`builtins.open` (then `write`). `cancel_order` offloads with `asyncio.to_thread` and
`refund_order` is a sync endpoint (threadpool): both unlabeled, correctly.
