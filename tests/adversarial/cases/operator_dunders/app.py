import redis
from fastapi import FastAPI

app = FastAPI()
r = redis.Redis()


class Cache:
    def __getitem__(self, key: str) -> bytes | None:
        return r.get(key)

    def __setitem__(self, key: str, value: bytes) -> None:
        r.set(key, value)

    def __contains__(self, key: str) -> bool:
        return bool(r.exists(key))

    def __len__(self) -> int:
        return r.dbsize()


cache = Cache()


@app.get("/cache/{key}")
async def get_cached(key: str):
    if key in cache:  # expect: *
        return cache[key]  # expect: *
    cache[key] = b"x"  # expect: *
    size = len(cache)  # expect: *
    plain = {"a": 1}
    return plain["a"], size
