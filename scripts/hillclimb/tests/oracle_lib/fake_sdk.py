"""A library-style async SDK for the oracle tests (on PYTHONPATH, outside every case dir): its
coroutine blocks the loop inside, with a sync HTTP call."""

import requests


async def fetch(url: str) -> str:
    return _get(url)


def _get(url: str) -> str:
    return str(requests.get(url, timeout=5))
