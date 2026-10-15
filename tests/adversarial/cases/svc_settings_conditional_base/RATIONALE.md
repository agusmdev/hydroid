Settings-driven strategy selection at import time: `if settings.use_redis_cache: CacheBackend =
RedisCacheBackend else: CacheBackend = MemoryCacheBackend`, and the repository then INHERITS from
the selected alias (`class FeatureFlagRepository(CacheBackend)`). A common pattern for swapping
cache/storage implementations between environments without a factory.

Why hydroid likely misses it: the base class is a name bound in two branches of an `if`, so its
type is a union of two class objects; a class whose base is not a single known class has an
unknown MRO for the type checker, and `self._get(...)` inside `is_enabled` is not resolved to
either backend's `_get`. Correct analysis: either backend may be the base, and the Redis one
calls `redis.Redis.get`.

Runtime: `USE_REDIS_CACHE` defaults to "1", so the base is `RedisCacheBackend`. `GET
/flags/{name}` -> `FeatureFlagService.check` (async) -> `repo.is_enabled` -> `self._get` ->
`Redis.get` on the loop thread; the oracle confirms it at the labeled line. `PUT /flags/{name}`
offloads `repo.enable` with `asyncio.to_thread` (negative). The dependency factory is sync, so
the repository construction runs in the threadpool (and `Redis.from_url` does not connect).
