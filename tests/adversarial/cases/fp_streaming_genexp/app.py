import time
from typing import Iterator

import requests
from fastapi import FastAPI
from fastapi.responses import StreamingResponse

app = FastAPI()

MIRRORS = ["https://a.example.com/part1", "https://b.example.com/part2"]


def fetch_part(url: str) -> bytes:
    try:
        return requests.get(url, timeout=5).content
    except requests.RequestException:
        return b""


@app.get("/bundle")
async def bundle():
    # Starlette wraps a sync iterable in iterate_in_threadpool: each next() runs in a worker thread.
    parts = (fetch_part(url) for url in MIRRORS)
    return StreamingResponse(parts, media_type="application/octet-stream")


@app.get("/ticks")
async def ticks(n: int):
    return StreamingResponse((time.sleep(0.01) or f"{i}\n" for i in range(n)), media_type="text/plain")


@app.get("/eager")
async def eager():
    return b"".join(fetch_part(url) for url in MIRRORS)  # expect: requests.api.get
