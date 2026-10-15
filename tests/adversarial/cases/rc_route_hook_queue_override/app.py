import asyncio
import json
import time
from pathlib import Path
from typing import Callable

from fastapi import APIRouter, FastAPI, Request, Response
from fastapi.routing import APIRoute

AUDIT_LOG = Path("/tmp/rc_route_hook_queue_override.log")
audit_queue: asyncio.Queue[dict] = asyncio.Queue()


class FileAuditRoute(APIRoute):
    """Default audit route: appends one JSON line per request."""

    def get_route_handler(self) -> Callable:
        original_route_handler = super().get_route_handler()

        async def custom_route_handler(request: Request) -> Response:
            started = time.perf_counter()
            response = await original_route_handler(request)
            entry = {
                "path": request.url.path,
                "status": response.status_code,
                "elapsed": time.perf_counter() - started,
            }
            self.record(entry)
            return response

        return custom_route_handler

    def record(self, entry: dict) -> None:
        with AUDIT_LOG.open("a") as fh:
            fh.write(json.dumps(entry) + "\n")


class QueuedAuditRoute(FileAuditRoute):
    """Hands entries to a background writer instead of touching the disk per request."""

    def record(self, entry: dict) -> None:
        audit_queue.put_nowait(entry)


app = FastAPI()


@app.get("/healthz")
async def healthz():
    return {"ok": True}


router = APIRouter(route_class=QueuedAuditRoute)


@router.get("/orders")
async def list_orders():
    return {"orders": []}


@router.get("/orders/{order_id}")
async def get_order(order_id: int):
    return {"id": order_id}


app.include_router(router)
