import logging
import logging.handlers

from fastapi import FastAPI

app = FastAPI()
log = logging.getLogger("audit")


class AuditFileHandler(logging.handlers.RotatingFileHandler):
    """Only configures the stdlib handler: its `emit` stats and writes the file."""

    def __init__(self) -> None:
        super().__init__("audit.log", maxBytes=1_000_000, backupCount=3, delay=True)


@app.on_event("startup")
async def setup() -> None:
    log.setLevel(logging.INFO)
    log.addHandler(AuditFileHandler())


@app.post("/audit")
async def audit():
    log.info("audited")  # expect: logging.handlers.RotatingFileHandler.shouldRollover
    return {}
