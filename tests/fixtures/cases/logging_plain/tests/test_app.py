import logging
import socket

from app import log


class ResolvingHandler(logging.Handler):
    """A test double: only test code's logging reaches it."""

    def emit(self, record: logging.LogRecord) -> None:
        socket.getfqdn()


async def test_index() -> None:
    logging.getLogger().addHandler(ResolvingHandler())
    log.warning("from a test")  # expect: socket.getfqdn
