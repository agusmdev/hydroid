# `@functools.cache` on a class as a singleton

Decorating a class with `functools.cache` is a common lazy-singleton idiom: `FeatureFlags()`
returns the same instance after the first call. The name `FeatureFlags` is now an
`_lru_cache_wrapper`, and calling it calls the class, whose `__init__` reads `flags.json`. On the
first request this read happens on the loop in the `async def` endpoint.

hydroid handles `@cache` on functions, but calling the cache-wrapped class does not resolve to
the class constructor, so the `__init__` I/O is missed.

Negatives: an undecorated class with no I/O, `asyncio.to_thread(FeatureFlags)`, and a `def`
endpoint (threadpool).
