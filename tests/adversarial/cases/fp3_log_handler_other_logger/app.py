import logging

import requests
from fastapi import FastAPI

app = FastAPI()


class SlackAuditHandler(logging.Handler):
    """Posts audit events to a Slack webhook."""

    def emit(self, record: logging.LogRecord) -> None:
        try:
            requests.post("https://hooks.slack.invalid/audit", json={"text": self.format(record)}, timeout=2)
        except requests.RequestException:
            self.handleError(record)


# Only the dedicated "audit" logger ships to Slack, and it does not propagate.
audit_log = logging.getLogger("audit")
audit_log.propagate = False
audit_log.setLevel(logging.INFO)
audit_log.addHandler(SlackAuditHandler())

log = logging.getLogger(__name__)


def service_logger(name: str) -> logging.Logger:
    return logging.getLogger(f"svc.{name}")


@app.get("/orders/{order_id}")
async def get_order(order_id: str):
    log.warning("order %s requested", order_id)
    return {"order_id": order_id}


@app.get("/payments/{payment_id}")
async def get_payment(payment_id: str):
    service_logger("payments").error("payment %s looked up", payment_id)
    return {"payment_id": payment_id}


@app.get("/health")
async def health():
    logging.warning("health probe")
    return {"ok": True}


@app.delete("/users/{user_id}")
async def delete_user(user_id: str):
    audit_log.info("user %s deleted", user_id)  # expect: requests.api.post
    return {"deleted": user_id}
