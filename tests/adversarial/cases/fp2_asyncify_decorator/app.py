import asyncio
import functools
import time
from typing import Awaitable, Callable, ParamSpec, TypeVar

import requests
from fastapi import FastAPI

P = ParamSpec("P")
R = TypeVar("R")

app = FastAPI()


def run_in_thread(fn: Callable[P, R]) -> Callable[P, Awaitable[R]]:
    """Turn a blocking function into an awaitable that runs in the default executor."""

    @functools.wraps(fn)
    async def wrapper(*args: P.args, **kwargs: P.kwargs) -> R:
        return await asyncio.to_thread(fn, *args, **kwargs)

    return wrapper


@run_in_thread
def fetch_profile(user_id: str) -> dict:
    try:
        return requests.get(f"https://profiles.example.com/{user_id}", timeout=5).json()
    except requests.RequestException:
        return {}


@run_in_thread
def render_report(name: str) -> str:
    time.sleep(0.05)
    return f"report for {name}"


@app.get("/profiles/{user_id}")
async def profile(user_id: str):
    return await fetch_profile(user_id)


@app.get("/reports/{name}")
async def report(name: str):
    text = await render_report(name)
    time.sleep(0.01)  # expect: time.sleep
    return {"text": text}
