"""Lazy iterables given to `StreamingResponse`: Starlette advances a sync iterable in a worker
thread (`iterate_in_threadpool`), so the elements of a generator expression or a `map(f, xs)`
are computed there. Only `iter(content)` runs on the loop."""

import time

from fastapi import FastAPI
from fastapi.responses import StreamingResponse
from starlette.responses import StreamingResponse as StarletteStreamingResponse

app = FastAPI()


def load(i: int) -> str:
    time.sleep(0.01)
    return f"{i}\n"


def keep(line: str) -> bool:
    time.sleep(0.01)
    return True


def slow_range(n: int) -> list[int]:
    time.sleep(0.01)
    return list(range(n))


@app.get("/genexp")
async def genexp(n: int):
    return StreamingResponse(load(i) for i in range(n))


@app.get("/genexp-variable")
async def genexp_variable(n: int):
    lines = (load(i) for i in range(n) if keep(str(i)))
    return StreamingResponse(lines, media_type="text/plain")


@app.get("/map")
async def mapped(n: int):
    return StarletteStreamingResponse(filter(keep, map(lambda i: load(i), range(n))))


@app.get("/wrapped")
async def wrapped(n: int):
    return StreamingResponse(content=iter(zip(map(load, range(n)), (load(i) for i in range(n)))))


@app.get("/outer-iterable")
async def outer_iterable(n: int):
    # The outermost iterable of a generator expression is evaluated when it is created.
    return StreamingResponse(load(i) for i in slow_range(n))  # expect: time.sleep


@app.get("/eager")
async def eager(n: int):
    return StreamingResponse(iter([load(i) for i in range(n)]))  # expect: time.sleep


@app.get("/variable-reused")
async def variable_reused(n: int):
    lines = (load(i) for i in range(n))  # expect: time.sleep
    first = next(lines)
    return StreamingResponse(lines, headers={"x-first": first})


async def fetch(i: int) -> str:
    return str(i)


@app.get("/async-genexp")
async def async_genexp(n: int):
    # Awaiting makes it an async generator, iterated on the loop.
    return StreamingResponse(load(i) + await fetch(i) for i in range(n))  # expect: time.sleep


@app.get("/joined")
async def joined(n: int):
    return "".join(load(i) for i in range(n))  # expect: time.sleep

