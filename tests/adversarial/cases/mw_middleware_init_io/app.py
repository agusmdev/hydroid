import asyncio
from pathlib import Path

from fastapi import FastAPI, Request, Response
from starlette.middleware.base import BaseHTTPMiddleware, RequestResponseEndpoint
from starlette.types import ASGIApp

ALLOWLIST = Path(__file__).with_name("allowlist.txt")


def read_allowlist(path: Path) -> set[str]:
    return set(path.read_text().split())


class IPAllowlistMiddleware(BaseHTTPMiddleware):
    """Tags requests coming from clients listed in the allowlist file."""

    def __init__(self, app: ASGIApp, allowlist_path: Path) -> None:
        super().__init__(app)
        # Starlette builds the middleware stack lazily, inside `Starlette.__call__`
        # (the first lifespan or request event), so this constructor runs on the loop.
        self.allowed = set(allowlist_path.read_text().split())  # expect: pathlib.Path.read_text

    async def dispatch(self, request: Request, call_next: RequestResponseEndpoint) -> Response:
        host = request.client.host if request.client else ""
        response = await call_next(request)
        response.headers["x-allowlisted"] = "1" if host in self.allowed else "0"
        return response


app = FastAPI()
app.add_middleware(IPAllowlistMiddleware, allowlist_path=ALLOWLIST)


@app.get("/allowlist")
async def show_allowlist():
    # Reloading per request is offloaded: fine.
    entries = await asyncio.to_thread(read_allowlist, ALLOWLIST)
    return {"count": len(entries)}
