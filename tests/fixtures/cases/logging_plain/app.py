import logging
import logging.handlers
import queue
import socket
import sys

from fastapi import FastAPI

app = FastAPI()


class ResolvingFilter(logging.Filter):
    """Defined, but never attached anywhere."""

    def filter(self, record: logging.LogRecord) -> bool:
        record.host = socket.getfqdn()
        return True


class LevelFilter(logging.Filter):
    def filter(self, record: logging.LogRecord) -> bool:
        return record.levelno >= logging.INFO


def setup() -> None:
    root = logging.getLogger()
    stream = logging.StreamHandler(sys.stdout)
    stream.addFilter(LevelFilter())
    stream.setFormatter(logging.Formatter("%(message)s"))
    root.addHandler(stream)
    root.addHandler(logging.NullHandler())
    root.addHandler(logging.handlers.QueueHandler(queue.SimpleQueue()))


log = logging.getLogger(__name__)


@app.get("/")
async def index():
    log.info("index")
    log.exception("failed")
    logging.warning("module level")
    return {}
