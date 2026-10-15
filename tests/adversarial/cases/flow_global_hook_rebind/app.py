import asyncio
import os
from typing import Callable

import requests
from fastapi import FastAPI

app = FastAPI()


def _discard(event: str, data: dict) -> None:
    return None


def _post_to_collector(event: str, data: dict) -> None:
    requests.post("https://metrics.internal/collect", json={"event": event, **data}, timeout=1)


track: Callable[[str, dict], None] = _discard


def configure_tracking() -> None:
    global track
    if os.environ.get("TRACKING_DISABLED") != "1":
        track = _post_to_collector


configure_tracking()


@app.get("/items/{item_id}")
async def read_item(item_id: int):
    track("item.read", {"id": item_id})  # expect: requests.api.post
    return {"id": item_id}


@app.get("/items/{item_id}/bg")
async def read_item_offloaded(item_id: int):
    await asyncio.to_thread(track, "item.read", {"id": item_id})
    return {"id": item_id}


@app.get("/health")
def health():
    track("health", {})
    return {"ok": True}
