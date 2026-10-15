# On the first request FastAPI reads the endpoint's source (`inspect.getsourcelines`) below the
# middleware's `await self.app(...)`: library code under a library coroutine, which no case call
# leads to. The endpoint's own blocking call is still the case's (at its call site in the
# endpoint, the innermost case coroutine).
from pathlib import Path

from fastapi import FastAPI, Request

app = FastAPI()


class Passthrough:
    def __init__(self, app):
        self.app = app

    async def __call__(self, scope, receive, send):
        await self.app(scope, receive, send)


app.add_middleware(Passthrough)


def read_settings():
    return Path("app.py").read_text()[:10]


@app.get("/")
async def index(request: Request):
    return {"settings": read_settings()}  # expect: pathlib.Path.read_text
