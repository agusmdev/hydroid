import asyncio
import json
import socket
import time
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI

PORT_FILE = Path("/tmp/invalidation.port")
cache: dict[str, str] = {}


class InvalidationProtocol(asyncio.DatagramProtocol):
    """Listens for cache-invalidation datagrams from sibling workers."""

    def connection_made(self, transport: asyncio.BaseTransport) -> None:
        self.transport = transport
        port = transport.get_extra_info("sockname")[1]
        # Advertise our port so sibling workers can find us.
        PORT_FILE.write_text(str(port))  # expect: pathlib.Path.write_text
        self.host = socket.gethostname()
        asyncio.get_running_loop().run_in_executor(None, self.warm_up)

    def warm_up(self) -> None:
        # Runs in the default executor, not on the loop.
        time.sleep(0.01)

    def datagram_received(self, data: bytes, addr: tuple[str, int]) -> None:
        key = json.loads(data)["key"]
        cache.pop(key, None)


@asynccontextmanager
async def lifespan(app: FastAPI):
    loop = asyncio.get_running_loop()
    transport, _ = await loop.create_datagram_endpoint(
        InvalidationProtocol, local_addr=("127.0.0.1", 0)
    )
    yield
    transport.close()


app = FastAPI(lifespan=lifespan)


@app.get("/cache/{key}")
async def read_cache(key: str):
    return {"value": cache.get(key)}
