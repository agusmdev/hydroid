import logging
import logging.handlers
import queue

from fastapi import FastAPI

app = FastAPI()

# Ops wants an email for every payment failure.
alerts = logging.getLogger("payments.alerts")
alerts.setLevel(logging.INFO)
_mail = logging.handlers.SMTPHandler(
    mailhost=("smtp.example.com", 25),
    fromaddr="payments@example.com",
    toaddrs=["oncall@example.com"],
    subject="Payment failure",
    timeout=5.0,
)
_mail.setLevel(logging.ERROR)
alerts.addHandler(_mail)

# The recommended setup: a QueueHandler on the loop, the SMTP handler on a listener thread.
audit = logging.getLogger("payments.audit")
audit.setLevel(logging.INFO)
_q: "queue.Queue[logging.LogRecord]" = queue.Queue()
audit.addHandler(logging.handlers.QueueHandler(_q))
_listener = logging.handlers.QueueListener(_q, _mail)
_listener.start()


@app.post("/payments/{payment_id}/fail")
async def payment_failed(payment_id: str):
    alerts.info("payment %s retrying", payment_id)
    alerts.error("payment %s failed", payment_id)  # expect: smtplib.SMTP.__init__
    return {"status": "alerted"}


@app.post("/payments/{payment_id}/audit")
async def payment_audit(payment_id: str):
    audit.error("payment %s flagged", payment_id)
    return {"status": "queued"}
