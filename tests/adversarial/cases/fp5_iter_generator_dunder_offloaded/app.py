import asyncio
import time
from typing import Iterator

from fastapi import FastAPI
from fastapi.responses import PlainTextResponse
from starlette.concurrency import iterate_in_threadpool

app = FastAPI()


class OrdersExport:
    """Rows of a CSV export, fetched page by page from a sync driver."""

    def __init__(self, tenant: str) -> None:
        self.tenant = tenant

    def _fetch_page(self, page: int) -> list[str]:
        time.sleep(0.001)  # stands in for a sync DB round trip
        return [f"{self.tenant},{page}"] if page < 2 else []

    def __iter__(self) -> Iterator[str]:
        page = 0
        while rows := self._fetch_page(page):
            yield from rows
            page += 1


@app.get("/export/{tenant}")
async def export(tenant: str):
    rows = iter(OrdersExport(tenant))
    # Peek the first row in a worker thread to fail fast on an empty export...
    first = await asyncio.to_thread(next, rows, None)
    if first is None:
        return PlainTextResponse("", status_code=204)
    # ...then drain the rest in the threadpool.
    lines = [first]
    async for line in iterate_in_threadpool(rows):
        lines.append(line)
    return PlainTextResponse("\n".join(lines))


@app.get("/export-inline/{tenant}")
async def export_inline(tenant: str):
    # Near miss: iterating on the loop runs the pages' queries here.
    return PlainTextResponse("\n".join(OrdersExport(tenant)))  # expect: time.sleep
