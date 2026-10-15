import asyncio
from abc import ABC, abstractmethod

import redis
from fastapi import FastAPI

app = FastAPI()


class RateLimiter(ABC):
    @abstractmethod
    def hit(self, key: str) -> int: ...


class _RedisLimiter:
    def __init__(self, url: str) -> None:
        self._client = redis.Redis.from_url(url)

    def hit(self, key: str) -> int:
        count = self._client.incr(f"rl:{key}")
        return count if isinstance(count, int) else 0


RateLimiter.register(_RedisLimiter)


def make_limiter(url: str) -> RateLimiter:
    limiter = _RedisLimiter(url)
    assert isinstance(limiter, RateLimiter)
    return limiter


limiter = make_limiter("redis://cache.internal:6379/0")


@app.get("/search")
async def search(q: str):
    hits = limiter.hit(q)  # expect: redis.commands.core.BasicKeyCommands.incr
    return {"q": q, "hits": hits}


@app.get("/search/offloaded")
async def search_offloaded(q: str):
    try:
        hits = await asyncio.to_thread(limiter.hit, q)
    except redis.RedisError:
        hits = 0
    return {"q": q, "hits": hits}
