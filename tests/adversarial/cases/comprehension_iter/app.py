import time

import requests
from fastapi import FastAPI

app = FastAPI()


class Pages:
    def __iter__(self):
        for page in range(3):
            yield requests.get(f"https://example.com/{page}")


class Cheap:
    def __iter__(self):
        return iter([1, 2, 3])


@app.get("/pages")
async def pages():
    bodies = [p.text for p in Pages()]  # expect: requests.api.get
    count = sum(1 for _ in Pages())  # expect: requests.api.get
    seen = {p.status_code for p in Pages()}  # expect: requests.api.get
    index = {p.url: p for p in Pages()}  # expect: requests.api.get
    cheap = [c for c in Cheap()]
    return bodies, count, seen, index, cheap


@app.get("/materialize")
async def materialize():
    everything = list(Pages())  # expect: requests.api.get
    as_tuple = tuple(Pages())  # expect: requests.api.get
    first = next(iter(Pages()))  # expect: requests.api.get
    return everything, as_tuple, first


def ticks():
    for i in range(3):
        time.sleep(0.1)
        yield i


@app.get("/ticks")
async def tick_route():
    return [t for t in ticks()]  # expect: time.sleep
