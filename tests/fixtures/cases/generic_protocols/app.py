"""A lock existing somewhere must not make every `with` on a context manager blocking:
`AbstractContextManager` is a stdlib protocol every lock satisfies structurally."""

import contextlib
import threading
from collections.abc import Generator

from fastapi import FastAPI

app = FastAPI()
lock = threading.Lock()


@contextlib.contextmanager
def span() -> Generator[None]:
    yield


def with_lock() -> None:
    with lock:
        pass


@app.get("/")
async def handler():
    with span():
        pass
    with_lock()  # expect: _thread.lock.__enter__
