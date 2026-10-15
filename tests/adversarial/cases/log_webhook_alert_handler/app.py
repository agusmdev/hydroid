import asyncio
import logging

import requests
from fastapi import FastAPI

ALERT_WEBHOOK = "https://hooks.example.com/services/T000/B000/XXXX"

logger = logging.getLogger("payments")
logger.setLevel(logging.INFO)


class WebhookAlertHandler(logging.Handler):
    """Forwards ERROR records to the on-call chat channel."""

    def __init__(self, url: str) -> None:
        super().__init__(level=logging.ERROR)
        self.url = url

    def emit(self, record: logging.LogRecord) -> None:
        try:
            requests.post(self.url, json={"text": self.format(record)}, timeout=5)
        except Exception:
            self.handleError(record)


def configure_logging() -> None:
    logger.addHandler(WebhookAlertHandler(ALERT_WEBHOOK))


app = FastAPI(on_startup=[configure_logging])


@app.post("/payments/{payment_id}/refund")
async def refund(payment_id: str):
    # INFO is below the handler level: Handler.handle filters it out before emit.
    logger.info("refund requested for %s", payment_id)
    logger.error("refund failed for %s: gateway declined", payment_id)  # expect: requests.api.post
    return {"payment_id": payment_id, "status": "declined"}


@app.post("/payments/{payment_id}/void")
async def void(payment_id: str):
    # Offloaded: the handler runs in a worker thread.
    await asyncio.to_thread(logger.error, "void failed for %s", payment_id)
    return {"payment_id": payment_id, "status": "failed"}


@app.post("/payments/{payment_id}/capture")
def capture(payment_id: str):
    # Sync endpoint: runs in the threadpool.
    logger.error("capture failed for %s", payment_id)
    return {"payment_id": payment_id, "status": "failed"}
