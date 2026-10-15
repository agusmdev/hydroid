import threading

import requests
from fastapi import FastAPI

app = FastAPI()

WEBHOOK_URL = "https://hooks.example.com/orders"


def notify_webhook(order_id: str) -> None:
    try:
        requests.post(WEBHOOK_URL, json={"order": order_id}, timeout=5)
    except requests.RequestException:
        pass


@app.post("/orders/{order_id}")
async def create_order(order_id: str):
    worker = threading.Thread(target=notify_webhook, args=(order_id,), daemon=True)
    # Bug: `run()` executes the target in the calling thread; `start()` was intended.
    worker.run()  # expect: requests.api.post
    return {"created": order_id}


@app.post("/orders/{order_id}/background")
async def create_order_bg(order_id: str):
    worker = threading.Thread(target=notify_webhook, args=(order_id,), daemon=True)
    worker.start()
    return {"created": order_id}
