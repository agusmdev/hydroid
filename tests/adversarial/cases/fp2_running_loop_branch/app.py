import asyncio

import requests
from fastapi import FastAPI

app = FastAPI()

AUDIT_URL = "https://audit.example.com/events"
_pending: asyncio.Queue[dict] | None = None


def _queue() -> asyncio.Queue[dict]:
    global _pending
    if _pending is None:
        _pending = asyncio.Queue()
    return _pending


def emit_audit(event: dict) -> None:
    """Record an audit event from async request handlers and from sync scripts/workers alike."""
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        # No event loop in this thread (CLI, cron, threadpool): deliver synchronously.
        try:
            requests.post(AUDIT_URL, json=event, timeout=5)
        except requests.RequestException:
            pass
        return
    # Inside the loop: hand off to the background shipper, never block.
    _queue().put_nowait(event)


@app.post("/orders/{order_id}/cancel")
async def cancel_order(order_id: str):
    emit_audit({"action": "cancel", "order": order_id})
    return {"cancelled": order_id}


@app.post("/orders/{order_id}/refund")
def refund_order(order_id: str):
    # def endpoint: runs in the threadpool, no running loop there, so the sync path is taken.
    emit_audit({"action": "refund", "order": order_id})
    return {"refunded": order_id}


@app.post("/orders/{order_id}/ship")
async def ship_order(order_id: str):
    requests.post(AUDIT_URL, json={"action": "ship", "order": order_id}, timeout=5)  # expect: requests.api.post
    return {"shipped": order_id}
