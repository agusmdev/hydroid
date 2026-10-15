import logging

import requests
from fastapi import FastAPI

app = FastAPI()


class PagerHandler(logging.Handler):
    """Pages the on-call engineer: only for errors."""

    def __init__(self) -> None:
        super().__init__(level=logging.ERROR)

    def emit(self, record: logging.LogRecord) -> None:
        requests.post("https://pager.example.com/alert", json={"text": record.getMessage()})


class ChatHandler(logging.Handler):
    def emit(self, record: logging.LogRecord) -> None:
        requests.post("https://chat.example.com/log", json={"text": record.getMessage()})


def setup() -> None:
    # No logger level is set anywhere: the root logger's default, WARNING, applies.
    root = logging.getLogger()
    root.addHandler(PagerHandler())
    chat = ChatHandler()
    chat.setLevel(logging.CRITICAL)
    root.addHandler(chat)


log = logging.getLogger(__name__)


@app.get("/")
async def index():
    log.info("below the logger level")
    log.warning("below both handler levels")
    log.log(logging.WARNING, "below both handler levels")
    log.error("pages")  # expect: requests.api.post
    log.critical("pages and chats")  # expect: requests.api.post
    return {}
