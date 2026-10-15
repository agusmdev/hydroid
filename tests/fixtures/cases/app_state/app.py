import asyncio
from contextlib import asynccontextmanager

import requests
from fastapi import FastAPI, Request

from clients import AsyncCache, BlockingCache, Ledger, Reports


# --- objects stored on application and connection state are read back as `Any` ---------------


class ShopApp(FastAPI):
    def audit(self) -> str:
        return requests.get("https://audit.example.com").text


@asynccontextmanager
async def lifespan(app: ShopApp):
    app.state.ledger = await asyncio.to_thread(Ledger, ":memory:")
    app.state.cache = BlockingCache()
    # Stored under one name with two classes: what a read gets is not known.
    app.state.reports = Reports()
    yield {"cache": AsyncCache(), "session": requests.Session()}


app = ShopApp(lifespan=lifespan)


@app.middleware("http")
async def attach(request: Request, call_next):
    request.app.state.reports = {}
    return await call_next(request)


@app.get("/ledger")
async def ledger(request: Request):
    return request.app.state.ledger.total()  # expect: sqlite3.Connection.execute


@app.get("/session")
async def session(request: Request):
    http = request.state.session
    return http.get("https://example.com").text  # expect: requests.sessions.Session.get


@app.get("/audit")
async def audit(request: Request):
    return request.app.audit()  # expect: requests.api.get


@app.get("/cache")
async def cache(request: Request):
    # The connection's `cache` is the lifespan's `AsyncCache`, not the application's.
    return await request.state.cache.get("k")


@app.get("/app-cache")
async def app_cache(request: Request):
    return request.app.state.cache.get("k")  # expect: time.sleep


@app.get("/reports")
async def reports(request: Request):
    return request.app.state.reports.export()


@app.get("/offloaded")
async def offloaded(request: Request):
    return await asyncio.to_thread(request.app.state.ledger.total)
