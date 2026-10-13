import asyncio
import time

from fastapi import FastAPI
from starlette.concurrency import run_in_threadpool

app = FastAPI()


class PriceRepo:
    def price(self, sku: str) -> float:
        time.sleep(0.001)  # stands in for a sync DB round trip
        return 1.0


repo = PriceRepo()


@app.get("/prices")
async def prices(skus: str):
    keys = skus.split(",")
    # The whole lookup loop runs in a worker thread: map() and the generator are lazy, only
    # list()/dict() (called in the thread) pull their elements.
    a = await asyncio.to_thread(list, map(repo.price, keys))
    b = await run_in_threadpool(dict, ((k, repo.price(k)) for k in keys))
    c = await asyncio.to_thread(sum, (repo.price(k) for k in keys))
    return {"a": a, "b": b, "c": c}


@app.get("/prices/eager")
async def prices_eager(skus: str):
    keys = skus.split(",")
    # Near miss: list() runs here, on the loop.
    return list(map(repo.price, keys))  # expect: time.sleep
