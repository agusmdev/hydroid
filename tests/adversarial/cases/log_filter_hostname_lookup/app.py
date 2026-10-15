import asyncio
import logging
import socket
import sys

from fastapi import FastAPI


class HostContextFilter(logging.Filter):
    """Stamps every record with the fully qualified host name for the log aggregator."""

    def filter(self, record: logging.LogRecord) -> bool:
        record.host = socket.getfqdn()
        return True


def setup_logging() -> None:
    handler = logging.StreamHandler(sys.stderr)
    handler.setFormatter(logging.Formatter("%(host)s %(levelname)s %(name)s %(message)s"))
    handler.addFilter(HostContextFilter())
    log = logging.getLogger("orders")
    log.setLevel(logging.INFO)
    log.addHandler(handler)
    log.propagate = False


logger = logging.getLogger("orders")
app = FastAPI(on_startup=[setup_logging])


@app.post("/orders/{order_id}/ship")
async def ship(order_id: str):
    logger.info("order %s shipped", order_id)  # expect: socket.getfqdn
    return {"order_id": order_id, "status": "shipped"}


@app.post("/orders/{order_id}/cancel")
async def cancel(order_id: str):
    # Offloaded: filter and handler run in a worker thread.
    await asyncio.to_thread(logger.info, "order %s cancelled", order_id)
    return {"order_id": order_id, "status": "cancelled"}


@app.get("/orders/{order_id}")
async def get_order(order_id: str):
    # Below the logger level: Logger.debug returns before any filter runs.
    logger.debug("order %s read", order_id)
    return {"order_id": order_id}


@app.get("/orders")
def list_orders():
    # Sync endpoint: threadpool.
    logger.info("orders listed")
    return []
