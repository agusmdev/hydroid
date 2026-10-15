# mod_lazy_package_getattr

The `clients` package imports its submodules lazily with a PEP 562 module `__getattr__`
(common for heavy SDK wrappers to keep startup fast): `clients.geo` is not bound in
`clients/__init__.py`; the first access calls `__getattr__("geo")`, which runs
`importlib.import_module(".geo", "clients")` and returns the submodule.

Statically `clients.geo` is whatever `__getattr__` returns (`ModuleType`), so
`clients.geo.resolve(host)` does not resolve to `clients.geo.resolve` and hydroid misses the DNS
lookup. Runtime: the `async def resolve` route calls `socket.gethostbyname` on the loop.
`/quote` reaches a pure function (negative) and `/resolve-sync` is a sync endpoint (threadpool).
