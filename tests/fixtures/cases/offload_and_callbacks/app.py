import asyncio
import functools
import threading
import time
from concurrent.futures import ThreadPoolExecutor

import anyio
import requests
from fastapi import FastAPI
from fastapi.concurrency import run_in_threadpool

app = FastAPI()
executor = ThreadPoolExecutor()


def blocking_helper(n: int = 0) -> int:
    time.sleep(n)
    return n


# --- work moved off the loop is fine ---------------------------------------------------------


@app.get("/offloaded")
async def offloaded(url: str):
    await asyncio.to_thread(requests.get, url)
    await asyncio.to_thread(lambda: requests.get(url))
    await asyncio.to_thread(blocking_helper)
    await asyncio.to_thread(functools.partial(blocking_helper, 1))
    await run_in_threadpool(blocking_helper, 1)
    await asyncio.get_running_loop().run_in_executor(None, blocking_helper)
    await anyio.to_thread.run_sync(blocking_helper)
    executor.submit(blocking_helper, 1)
    threading.Thread(target=blocking_helper).start()
    return {}


@app.get("/misused")
async def misused():
    # The helper runs on the loop: its *result* is what gets sent to the thread.
    await asyncio.to_thread(blocking_helper(1))  # expect: time.sleep


# --- callables flowing through parameters -----------------------------------------------------


def blocking_transform(x: int) -> int:
    time.sleep(x)
    return x


def apply(callback, x: int) -> int:
    return callback(x)


def forward(cb) -> int:
    return apply(cb, 1)


def run_twice(fn) -> None:
    fn()
    fn()


@app.get("/callbacks")
async def callbacks():
    apply(blocking_transform, 1)  # expect: time.sleep
    forward(blocking_transform)  # expect: time.sleep
    run_twice(lambda: time.sleep(1))  # expect: time.sleep
    apply(abs, 1)
    return {}


# --- decorators --------------------------------------------------------------------------------


def logged(func):
    @functools.wraps(func)
    def wrapper(*args, **kwargs):
        return func(*args, **kwargs)

    return wrapper


def slow_down(func):
    def wrapper(*args, **kwargs):
        time.sleep(0.1)
        return func(*args, **kwargs)

    return wrapper


def retry(times: int):
    def decorator(func):
        def wrapper(*args, **kwargs):
            for _ in range(times):
                result = func(*args, **kwargs)
            return result

        return wrapper

    return decorator


def threaded(func):
    async def wrapper(*args):
        return await asyncio.to_thread(func, *args)

    return wrapper


@logged
def blocking_logged() -> None:
    requests.get("https://example.com")


@slow_down
def harmless() -> None:
    pass


@retry(times=3)
def blocking_retried() -> None:
    time.sleep(1)


@threaded
def heavy() -> None:
    time.sleep(1)


@app.get("/decorated")
async def decorated():
    blocking_logged()  # expect: requests.api.get
    harmless()  # expect: time.sleep
    blocking_retried()  # expect: time.sleep
    await heavy()
    return {}


# --- sync callbacks scheduled on the loop -------------------------------------------------------


def on_tick() -> None:
    time.sleep(0.5)  # expect: time.sleep


def on_later() -> None:
    requests.get("https://example.com")  # expect: requests.api.get


def on_done(future) -> None:
    time.sleep(0.5)  # expect: time.sleep


def with_arg(n: int) -> None:
    time.sleep(n)  # expect: time.sleep


@app.get("/scheduled")
async def scheduled():
    loop = asyncio.get_running_loop()
    loop.call_soon(on_tick)
    loop.call_later(1, on_later)
    loop.call_soon(functools.partial(with_arg, 1))
    task = asyncio.ensure_future(asyncio.sleep(1))
    task.add_done_callback(on_done)
    return {}
