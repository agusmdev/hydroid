import time
from pathlib import Path

from fastapi import FastAPI
from starlette.middleware import Middleware
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.types import ASGIApp, Receive, Scope, Send


class LoadsRules(BaseHTTPMiddleware):
    def __init__(self, app: ASGIApp, path: Path) -> None:
        super().__init__(app)
        self.rules = path.read_text()  # expect: pathlib.Path.read_text via middleware (construction)

    async def dispatch(self, request, call_next):
        return await call_next(request)


class Warmup:
    def __init__(self, app: ASGIApp) -> None:
        self.app = app
        time.sleep(0.01)  # expect: time.sleep via middleware (construction)

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        await self.app(scope, receive, send)


class Plain:
    def __init__(self, app: ASGIApp, header: str) -> None:
        self.app = app
        self.header = header

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        await self.app(scope, receive, send)


def build_tagger(app: ASGIApp) -> ASGIApp:
    time.sleep(0.01)  # expect: time.sleep via middleware
    return app


app = FastAPI(middleware=[Middleware(Warmup)])
app.add_middleware(LoadsRules, path=Path("rules.txt"))
app.add_middleware(Plain, header="x-plain")
app.add_middleware(build_tagger)
