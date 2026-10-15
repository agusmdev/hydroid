"""Project callables assigned over library methods on an instance, which library code calls on
the loop: `app.openapi = custom_openapi` (FastAPI's `/openapi.json` route, an `async def`
closure registered by `FastAPI.setup`, calls `self.openapi()`)."""

import time
from pathlib import Path

from fastapi import FastAPI
from fastapi.openapi.utils import get_openapi

NOTES = Path("notes.md")

app = FastAPI()


def custom_openapi() -> dict:
    schema = get_openapi(title=app.title, version=app.version, routes=app.routes)
    schema["info"]["description"] = NOTES.read_text()  # expect: pathlib.Path.read_text
    return schema


app.openapi = custom_openapi


def slow_stack():
    # `Starlette.__call__` (an `async def`) builds the stack on the first request.
    time.sleep(0.1)  # expect: time.sleep
    return app.router


app.build_middleware_stack = slow_stack


def custom_setup() -> None:
    # `setup` only runs from `FastAPI.__init__`, before this assignment: never on the loop.
    time.sleep(0.1)


app.setup = custom_setup


class DocsApp(FastAPI):
    def __init__(self) -> None:
        super().__init__()
        self.openapi = self.cached_schema

    def cached_schema(self) -> dict:
        time.sleep(0.1)  # expect: time.sleep
        return {}


def not_a_library_method() -> None:
    time.sleep(0.1)


app.extra_hook = not_a_library_method


@app.get("/items")
async def items() -> list[int]:
    return []
