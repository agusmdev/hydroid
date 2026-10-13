import asyncio
import importlib
import os

from fastapi import FastAPI

AUDIT_BACKEND = os.environ.get("AUDIT_BACKEND", "plugins.audit_file")

app = FastAPI()

# Plugin resolved once by dotted path, like Django's import_string.
audit = importlib.import_module(AUDIT_BACKEND)


@app.post("/orders/{order_id}")
async def create_order(order_id: str):
    audit.record("order.created", {"id": order_id})  # expect: builtins.open
    return {"id": order_id}


@app.post("/orders/{order_id}/cancel")
async def cancel_order(order_id: str):
    await asyncio.to_thread(audit.record, "order.cancelled", {"id": order_id})
    return {"id": order_id}


@app.post("/orders/{order_id}/refund")
def refund_order(order_id: str):
    audit.record("order.refunded", {"id": order_id})
    return {"id": order_id}
