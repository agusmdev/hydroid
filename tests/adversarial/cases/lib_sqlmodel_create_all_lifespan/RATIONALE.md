The SQLModel tutorial's `SQLModel.metadata.create_all(engine)` is called directly in an async
lifespan. `MetaData.create_all` is not itself in the catalog, but it runs
`Engine._run_ddl_visitor`, which opens `with self.begin() as conn:` (`Engine.begin`, in the
catalog) and emits DDL over a real DB connection - blocking the loop during startup. hydroid
misses it (the blocking call is inside SQLAlchemy's schema module, reached through `MetaData`).
The runtime records `sqlalchemy.engine.base.Engine.begin` on that line.

Near-misses: the same work offloaded with `asyncio.to_thread` (wrapped helper and `drop_all`
passed as a callable), and a sync endpoint (threadpool) running `Session.exec`.
