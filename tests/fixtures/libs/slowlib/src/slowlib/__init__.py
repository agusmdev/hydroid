import asyncio
import time

import requests


def fetch_report(report_id: int) -> dict:
    return _download(f"https://reports.example.com/{report_id}")


def _download(url: str) -> dict:
    return requests.get(url).json()


def cached_total() -> int:
    return 42


async def refresh() -> None:
    time.sleep(1)


async def refresh_properly() -> None:
    await asyncio.sleep(1)
