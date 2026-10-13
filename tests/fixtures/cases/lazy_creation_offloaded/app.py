"""Creating a lazy object on the loop runs nothing; what advances it runs its work, wherever that
is. Lazy iterables (`map(f, xs)`, generator expressions) handed to an offload API together with
the consumer pulling them, generators returned by a generator `__iter__` (`iter(obj)`), and the
generator inside what a `@contextmanager` function returns, entered in the threadpool."""

import asyncio
import time
from contextlib import ExitStack, contextmanager
from typing import Iterator

from fastapi import FastAPI
from fastapi.concurrency import contextmanager_in_threadpool
from starlette.concurrency import iterate_in_threadpool, run_in_threadpool

app = FastAPI()


def price(sku: str) -> float:
    time.sleep(0.01)
    return 1.0


def slow_keys() -> list[str]:
    time.sleep(0.01)
    return ["a"]


@app.get("/offloaded-consumers")
async def offloaded_consumers():
    a = await asyncio.to_thread(list, map(price, ["a"]))
    b = await run_in_threadpool(dict, ((k, price(k)) for k in ["a"]))
    lazy = (price(k) for k in ["a"])
    c = await asyncio.to_thread(sum, lazy)
    d = await asyncio.to_thread(sorted, filter(None, map(lambda k: price(k), ["a"])))
    return a, b, c, d


@app.get("/offloaded-outer-iterable")
async def offloaded_outer_iterable():
    # The outermost iterable of a generator expression is evaluated when it is created.
    return await asyncio.to_thread(list, (price(k) for k in slow_keys()))  # expect: time.sleep


@app.get("/consumed-here")
async def consumed_here():
    # Not an offload API: the consumer pulls the elements on the loop.
    a = sum(price(k) for k in ["a"])  # expect: time.sleep
    b = list(map(price, ["a"]))  # expect: time.sleep
    return a, b


class Export:
    def __iter__(self) -> Iterator[str]:
        time.sleep(0.01)
        yield "row"


class Eager:
    def __iter__(self) -> Iterator[str]:
        time.sleep(0.01)
        return iter(["row"])


@app.get("/generator-iter-offloaded")
async def generator_iter_offloaded():
    rows = iter(Export())
    first = await asyncio.to_thread(next, rows, None)
    return [first] + [line async for line in iterate_in_threadpool(rows)]


@app.get("/generator-iter-advanced-here")
async def generator_iter_advanced_here():
    rows = iter(Export())  # expect: time.sleep
    return next(rows)


@app.get("/generator-iter-for")
async def generator_iter_for():
    return [row for row in Export()]  # expect: time.sleep


@app.get("/eager-iter")
async def eager_iter():
    # A plain `__iter__` runs when `iter()` calls it.
    rows = iter(Eager())  # expect: time.sleep
    return await asyncio.to_thread(list, rows)


def pages() -> Iterator[int]:
    time.sleep(0.01)
    yield 1


@app.get("/generator-variable")
async def generator_variable():
    created = pages()
    iterated = pages()  # expect: time.sleep
    for page in iterated:
        pass
    return await asyncio.to_thread(list, created)


@contextmanager
def legacy_session() -> Iterator[str]:
    time.sleep(0.01)
    yield "session"


@app.get("/cm-in-threadpool")
async def cm_in_threadpool():
    unused = legacy_session()
    async with contextmanager_in_threadpool(legacy_session()) as session:
        return session, unused


@app.get("/cm-entered")
async def cm_entered():
    with legacy_session() as session:  # expect: time.sleep
        pass
    cm = legacy_session()  # expect: time.sleep
    with cm:
        pass
    with ExitStack() as stack:
        stack.enter_context(legacy_session())  # expect: time.sleep
    return session
