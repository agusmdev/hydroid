"""Library `async def` methods calling hooks on `self` that project subclasses override."""

import sqlite3
import time

from fastapi import Depends, FastAPI, HTTPException, Request, Security
from fastapi.security import APIKeyHeader, HTTPBearer

db = sqlite3.connect(":memory:", check_same_thread=False)


class DatabaseKeyHeader(APIKeyHeader):
    # `APIKeyHeader.__call__` (an `async def`) calls `self.check_api_key(...)`.
    def check_api_key(self, api_key: str | None) -> str | None:
        row = db.execute("SELECT owner FROM keys WHERE key = ?", (api_key,)).fetchone()  # expect: sqlite3.Connection.execute sqlite3.Cursor.fetchone via dependency (__call__)
        return row[0] if row else None


class SlowErrorKeyHeader(APIKeyHeader):
    # Reached through the library's own `check_api_key`.
    def make_not_authenticated_error(self) -> HTTPException:
        time.sleep(0.1)  # expect: time.sleep via dependency (__call__)
        return HTTPException(status_code=401)


class AuditedBearer(HTTPBearer):
    # Never called by `HTTPBearer.__call__`.
    def audit(self) -> None:
        time.sleep(0.1)

    def __init__(self) -> None:
        time.sleep(0.1)
        super().__init__()


class AwaitedKeyHeader(APIKeyHeader):
    def check_api_key(self, api_key: str | None) -> str | None:
        time.sleep(0.1)  # reported where it is awaited
        return api_key


class OffloadedKeyHeader(APIKeyHeader):
    def check_api_key(self, api_key: str | None) -> str | None:
        time.sleep(0.1)
        return api_key


database_scheme = DatabaseKeyHeader(name="X-API-Key", auto_error=False)
slow_error_scheme = SlowErrorKeyHeader(name="X-API-Key")
audited_scheme = AuditedBearer()
awaited_scheme = AwaitedKeyHeader(name="X-API-Key")
offloaded_scheme = OffloadedKeyHeader(name="X-API-Key")
plain_scheme = APIKeyHeader(name="X-API-Key", auto_error=False)

app = FastAPI()


@app.get("/database")
async def database(owner: str | None = Security(database_scheme)):
    return {"owner": owner}


@app.get("/slow-error")
async def slow_error(key: str | None = Depends(slow_error_scheme)):
    return {"key": key}


@app.get("/audited")
async def audited(credentials=Depends(audited_scheme)):
    return {"ok": True}


@app.get("/plain")
async def plain(key: str | None = Security(plain_scheme)):
    return {"key": key}


@app.get("/awaited")
async def awaited(request: Request):
    return {"key": await awaited_scheme(request)}  # expect: time.sleep via route GET /awaited


def make_coroutine(request: Request):
    # A sync function only creates the coroutine.
    return offloaded_scheme(request)
