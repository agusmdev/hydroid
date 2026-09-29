import asyncio

from .repo import fetch


def load(user_id: int) -> dict:
    return normalize(fetch(user_id))


def normalize(payload: dict) -> dict:
    return {k.lower(): v for k, v in payload.items()}


async def fetch_async() -> int:
    await asyncio.sleep(0)
    return 1


def make_coroutine():
    return fetch_async()
