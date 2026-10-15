import asyncio
import os

import redis
from fastapi import Depends, FastAPI


class Settings:
    redis_url: str = os.environ.get("REDIS_URL", "redis://cache.internal:6379/0")
    use_redis_cache: bool = os.environ.get("USE_REDIS_CACHE", "1") == "1"


settings = Settings()


# --- cache backends --------------------------------------------------------


class MemoryCacheBackend:
    def __init__(self) -> None:
        self._data: dict[str, str] = {}

    def _get(self, key: str) -> str | None:
        return self._data.get(key)

    def _set(self, key: str, value: str) -> None:
        self._data[key] = value


class RedisCacheBackend:
    def __init__(self) -> None:
        self._client = redis.Redis.from_url(settings.redis_url)

    def _get(self, key: str) -> str | None:
        return self._client.get(key)

    def _set(self, key: str, value: str) -> None:
        self._client.set(key, value, ex=300)


if settings.use_redis_cache:
    CacheBackend = RedisCacheBackend
else:
    CacheBackend = MemoryCacheBackend


# --- repository ------------------------------------------------------------


class FeatureFlagRepository(CacheBackend):
    prefix = "flags:"

    def is_enabled(self, name: str) -> bool:
        return self._get(self.prefix + name) == "1"

    def enable(self, name: str) -> None:
        self._set(self.prefix + name, "1")


class FeatureFlagService:
    def __init__(self, repo: FeatureFlagRepository) -> None:
        self.repo = repo

    async def check(self, name: str) -> bool:
        return self.repo.is_enabled(name)  # expect: redis.commands.core.BasicKeyCommands.get


def get_flag_service() -> FeatureFlagService:
    return FeatureFlagService(FeatureFlagRepository())


app = FastAPI()


@app.get("/flags/{name}")
async def read_flag(name: str, svc: FeatureFlagService = Depends(get_flag_service)):
    return {"name": name, "enabled": await svc.check(name)}


@app.put("/flags/{name}")
async def enable_flag(name: str, svc: FeatureFlagService = Depends(get_flag_service)):
    await asyncio.to_thread(svc.repo.enable, name)
    return {"name": name, "enabled": True}
