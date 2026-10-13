# SQLAlchemy `@validates` runs on construction and attribute assignment

`@validates("email")` registers an attribute "set" event on `Customer.email`. The ORM calls the
validator synchronously whenever the attribute is set: in the declarative constructor
(`Customer(email=...)`, the default `__init__` sets each kwarg) and on `customer.email = ...`.
Here the validator checks a blocked-domains set in Redis with the sync client
(`redis.Redis.sismember`), so constructing or updating the model in an `async def` endpoint
blocks the loop.

hydroid would have to know that the mapped class's library-provided `__init__` / instrumented
attribute `__set__` dispatches to methods decorated with `sqlalchemy.orm.validates`; neither the
constructor call nor the assignment is reported.

Labels: line 43 (constructor with `email=`) and line 51 (`customer.email = ...`) - runtime
confirms the `sismember` call on the loop for both (the oracle names it via redis's
`AsyncSetCommands` alias; same function). The stand-in return value is truthy, so the validator
then raises ValueError - after the blocking call, which does not affect the labels.
Negatives: `Customer(nickname="x")` and `customer.nickname = ...` don't touch the validated
attribute; `/customers/offloaded` builds the model through `run_in_threadpool`.
