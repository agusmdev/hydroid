# Lazy-loaded relationship accessed in an async endpoint

`author.books` on a persistent `Author` whose `books` relationship uses the default
`lazy="select"` strategy: the first attribute access runs `LazyLoader._emit_lazyload`, which calls
`Session.execute(select(Book)...)` synchronously on the calling thread. The endpoint fetched the
object correctly off the loop (`run_in_threadpool(db.get, ...)`), but the plain attribute read in
the list comprehension then runs a SELECT on the event loop. This is the classic "I offloaded the
query but the lazy load still blocks" bug.

hydroid sees only an attribute read on a `Mapped[list[Book]]` (an `InstrumentedAttribute`
descriptor in the library) and has no notion that it emits SQL, so it reports nothing.

Labels: line 58 (the comprehension reading `author.books`) - runtime confirms
`Session.execute` on the loop. Negatives: `/books-eager` loads with `selectinload` inside the
threadpool call so the access is a plain collection read (runtime: no blocking); `/books/{id}/author`
is a sync endpoint (threadpool), so its lazy load is fine.
