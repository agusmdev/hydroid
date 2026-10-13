"""Handlers and filters run only for the loggers they are attached to (and, for handlers, the
loggers below them, up to one that does not propagate)."""

import logging
import socket
import time

import requests
from fastapi import FastAPI

app = FastAPI()


class SlackHandler(logging.Handler):
    def emit(self, record: logging.LogRecord) -> None:
        requests.post("https://hooks.example.com/audit", json={"text": record.getMessage()})


class HostFilter(logging.Filter):
    def filter(self, record: logging.LogRecord) -> bool:
        record.host = socket.getfqdn()
        return True


def slow_filter(record: logging.LogRecord) -> bool:
    time.sleep(0.01)
    return True


audit = logging.getLogger("audit")
audit.propagate = False
audit.addHandler(SlackHandler())

# A filter on a logger runs only for records logged on that very logger.
logging.getLogger("billing").addFilter(slow_filter)


def setup() -> None:
    handler = logging.StreamHandler()
    handler.addFilter(HostFilter())
    logging.getLogger("jobs").addHandler(handler)


log = logging.getLogger(__name__)


def named(name: str) -> logging.Logger:
    return logging.getLogger(f"svc.{name}")


@app.get("/quiet")
async def quiet():
    # Neither `audit` nor below it.
    log.warning("module logger")
    named("payments").error("svc logger")
    logging.warning("root logger")
    logging.getLogger("auditing").warning("only shares a prefix")
    # Below `billing`: its filter is not consulted for propagated records.
    logging.getLogger("billing.invoices").warning("child")
    return {}


@app.get("/loud")
async def loud(name: str):
    audit.warning("audited")  # expect: requests.api.post
    logging.getLogger("audit.users").warning("child of audit")  # expect: requests.api.post
    logging.getLogger("billing").warning("billing")  # expect: time.sleep
    logging.getLogger("jobs.nightly").warning("child of jobs")  # expect: socket.getfqdn
    # Not known: may be any logger.
    logging.getLogger(name).warning("dynamic")  # expect: *
    return {}
