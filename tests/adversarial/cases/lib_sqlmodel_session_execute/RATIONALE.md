`sqlmodel.Session` overrides `execute` (deprecated in favor of `exec`, but still widely used for
aggregate/multi-column selects) and delegates to `sqlalchemy.orm.Session.execute` via `super()`.
The session comes from the standard SQLModel generator dependency, used in an async endpoint, so
the query runs on the loop. hydroid knows `Session.exec` and SQLAlchemy's `Session.execute`, but
the call resolves to `sqlmodel.orm.session.Session.execute`, which is neither, so it is missed.
The runtime records `sqlalchemy.orm.session.Session.execute` on the labeled line.

Near-misses: the same query in a helper offloaded with `asyncio.to_thread`, and a sync endpoint
(threadpool).
