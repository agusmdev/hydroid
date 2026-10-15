"""Storing a callable, or a context manager's exit, on an exit stack does not run it: the stack
runs what it stores where it is closed (`close()`, the end of a `with` block on it), in the
worker thread when it is closed there (`to_thread(stack.close)`, a sync background task), on a
new stack's close once `pop_all()` moved it there."""

import asyncio
import contextlib
import os
import tempfile
import time

from fastapi import FastAPI
from fastapi.responses import PlainTextResponse
from starlette.background import BackgroundTask

app = FastAPI()


def _render_csv(path: str, tenant: str) -> None:
    with open(path, "w") as fh:
        fh.write(f"tenant,{tenant}\n")


def _discard(path: str) -> None:
    try:
        os.unlink(path)
    except FileNotFoundError:
        pass


def _read_text(path: str) -> str:
    with open(path) as fh:
        return fh.read()


class AuditSpan:
    def __init__(self, resource: str) -> None:
        self.resource = resource

    def __enter__(self) -> "AuditSpan":
        return self

    def __exit__(self, *exc) -> None:
        time.sleep(0.01)


@app.get("/exports/{tenant}")
async def export(tenant: str):
    cleanup = contextlib.ExitStack()
    fd, path = await asyncio.to_thread(tempfile.mkstemp, suffix=".csv")
    os.close(fd)
    # Registering only: the callback runs when the stack is closed, in the worker below.
    cleanup.callback(_discard, path)
    try:
        await asyncio.to_thread(_render_csv, path, tenant)
        body = await asyncio.to_thread(_read_text, path)
    finally:
        await asyncio.to_thread(cleanup.close)
    return PlainTextResponse(body)


@app.get("/exports-inline/{tenant}")
async def export_inline(tenant: str):
    cleanup = contextlib.ExitStack()
    fd, path = await asyncio.to_thread(tempfile.mkstemp, suffix=".csv")
    os.close(fd)
    cleanup.callback(_discard, path)
    try:
        await asyncio.to_thread(_render_csv, path, tenant)
    finally:
        # Closing the stack on the loop runs the unlink here.
        cleanup.close()  # expect: os.unlink
    return PlainTextResponse("ok")


@app.get("/spans/{name}")
async def spans(name: str):
    # The exit moves to a new stack, closed by a sync background task, in the threadpool.
    with contextlib.ExitStack() as stack:
        stack.enter_context(AuditSpan(name))
        deferred = stack.pop_all()
    return PlainTextResponse(name, background=BackgroundTask(deferred.close))


@app.get("/spans-inline/{name}")
async def spans_inline(name: str):
    with contextlib.ExitStack() as stack:  # expect: time.sleep
        stack.enter_context(AuditSpan(name))
    return name


@app.get("/spans-branch/{name}")
async def spans_branch(name: str, offload: bool = False):
    stack = contextlib.ExitStack()
    stack.enter_context(AuditSpan(name))
    if offload:
        await asyncio.to_thread(stack.close)
    else:
        stack.close()  # expect: time.sleep
    # Closed on every path: leaving this block runs nothing more.
    with stack:
        pass
    return name


@app.get("/spans-early/{name}")
async def spans_early(name: str, keep: bool = False):
    # A `return` leaves the `with` block before the exits move: they run at its end.
    with contextlib.ExitStack() as stack:  # expect: time.sleep
        stack.enter_context(AuditSpan(name))
        if not keep:
            return name
        deferred = stack.pop_all()
    await asyncio.to_thread(deferred.close)
    return name


@app.get("/spans-never-closed/{name}")
async def spans_never_closed(name: str):
    stack = contextlib.ExitStack()
    stack.callback(_discard, name)
    return name
