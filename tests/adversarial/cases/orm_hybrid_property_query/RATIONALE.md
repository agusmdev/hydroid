# `hybrid_property` instance getter that queries through `object_session(self)`

`sqlalchemy.ext.hybrid.hybrid_property` is a descriptor: on an instance, `__get__` calls the
decorated getter. A common pattern computes an aggregate in the instance getter via
`object_session(self).scalar(select(...))` and provides a SQL `expression` for class-level use.
In an async endpoint, `customer.lifetime_value` therefore runs `Session.scalar` on the event
loop even though the object itself was loaded off the loop with `run_in_threadpool`.

hydroid would need to treat the library descriptor `hybrid_property` like `property` (attribute
read -> call the getter); it does not report the read.

Labels: line 80 (`customer.lifetime_value`) - runtime confirms `Session.scalar` on the loop.
Negatives: `customer.full_name` (hybrid getter with no I/O), `/ltv-sql` uses the class-level
expression and executes the statement in the threadpool, `/sync` is a sync endpoint.
