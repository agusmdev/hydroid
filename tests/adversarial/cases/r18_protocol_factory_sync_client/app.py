import asyncio
from typing import Any, Awaitable, Protocol

import httpx
from fastapi import FastAPI

app = FastAPI()


class Transport(Protocol):
    def get(self, url: str, **kwargs: Any) -> Any: ...


class AsyncTransport(Protocol):
    def get(self, url: str, **kwargs: Any) -> Awaitable[Any]: ...


def build_transport() -> Transport:
    return httpx.Client(base_url="https://billing.internal", timeout=5)


def build_async_transport() -> AsyncTransport:
    return httpx.AsyncClient(base_url="https://billing.internal", timeout=5)


transport: Transport = build_transport()
async_transport: AsyncTransport = build_async_transport()


@app.get("/invoices")
async def invoices():
    resp = transport.get("/invoices")  # expect: httpx._client.Client.get
    return {"status": resp.status_code}


@app.get("/invoices/offloaded")
async def invoices_offloaded():
    try:
        await asyncio.to_thread(transport.get, "/invoices")
    except httpx.HTTPError:
        pass
    return {}


@app.get("/invoices/async")
async def invoices_async():
    try:
        await async_transport.get("/invoices")
    except httpx.HTTPError:
        pass
    return {}
