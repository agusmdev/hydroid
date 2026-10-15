import asyncio
import logging
import socket
import time

import requests
from fastapi import FastAPI

app = FastAPI()


class WebhookHandler(logging.Handler):
    """Posts every record to a chat webhook."""

    def emit(self, record: logging.LogRecord) -> None:
        requests.post("https://hooks.example.com/log", json={"text": self.format(record)})


class HostFormatter(logging.Formatter):
    def format(self, record: logging.LogRecord) -> str:
        return f"{socket.gethostbyname('db')} {super().format(record)}"


def stamp(record: logging.LogRecord) -> bool:
    time.sleep(0.01)
    return True


def setup() -> None:
    webhook = WebhookHandler()
    webhook.setFormatter(HostFormatter())
    errors = logging.getLogger("errors")
    errors.addHandler(webhook)
    logging.getLogger("audit").addFilter(stamp)


log = logging.getLogger("errors")


def record_failure(reason: str) -> None:
    log.error("failed: %s", reason)


@app.post("/fail")
async def fail():
    # The hooks of the `errors` logger run (the `audit` filter does not); the report names the
    # shortest chain.
    log.warning("failing")  # expect: _socket.gethostbyname
    record_failure("boom")  # expect: requests.api.post
    # The root logger: no handler is attached to it or above it.
    logging.exception("module level")
    logging.getLogger("audit").warning("audited")  # expect: time.sleep
    return {}


@app.post("/quiet")
async def quiet():
    # Below the levels applications run with: no handler runs.
    log.debug("quiet")
    # Offloaded: the handlers run in a worker thread.
    await asyncio.to_thread(log.error, "offloaded")
    return {}


@app.get("/sync")
def sync_endpoint():
    # Threadpool.
    log.error("sync")
    return {}
