import asyncio
import random
import time
from typing import Callable, TypeVar

from fastapi import FastAPI

app = FastAPI()
T = TypeVar("T")


def with_backoff(fn: Callable[[], T], *, attempts: int = 3, base: float = 0.05, sleep: Callable[[float], None] = time.sleep) -> T:
    for attempt in range(attempts):
        try:
            return fn()
        except ValueError:
            sleep(base * 2**attempt + random.random() * 0.01)
    raise RuntimeError("gave up")


async def with_backoff_async(fn: Callable[[], T], *, attempts: int = 3, base: float = 0.05) -> T:
    for attempt in range(attempts):
        try:
            return fn()
        except ValueError:
            await asyncio.sleep(base * 2**attempt)
    raise RuntimeError("gave up")


class Flaky:
    def __init__(self) -> None:
        self.calls = 0

    def __call__(self) -> int:
        self.calls += 1
        if self.calls < 2:
            raise ValueError("transient")
        return self.calls


@app.get("/quota")
async def quota():
    used = with_backoff(Flaky())  # expect: time.sleep
    return {"used": used}


@app.get("/quota/async")
async def quota_async():
    used = await with_backoff_async(Flaky())
    return {"used": used}


@app.get("/quota/thread")
async def quota_thread():
    used = await asyncio.to_thread(with_backoff, Flaky())
    return {"used": used}
