# r18_abc_register_virtual

`_RedisLimiter` does not inherit from the `RateLimiter` ABC; it is made a *virtual subclass* with
`RateLimiter.register(_RedisLimiter)`, a standard way to adapt a class to an interface. The typed
factory `make_limiter() -> RateLimiter` returns it behind the ABC type, so at the call site the
static type of `limiter.hit` is the abstract `RateLimiter.hit` (body `...`). An analyzer that finds
implementations by class hierarchy sees no override of `hit`, so it reports nothing.

At runtime `limiter` is a `_RedisLimiter` and `hit` calls `redis.Redis.incr` (sync network I/O,
`redis.commands.core.BasicKeyCommands.incr`) on the loop thread. Negative: the same method
offloaded with `asyncio.to_thread`.
