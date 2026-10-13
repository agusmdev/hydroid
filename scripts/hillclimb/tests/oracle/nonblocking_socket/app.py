# httpx.AsyncClient talks over asyncio's non-blocking sockets: their send/connect return at once.
# The upstream server is a Unix socket (the oracle refuses other connections).
import asyncio
import os
import tempfile
from contextlib import asynccontextmanager

import httpx
from fastapi import FastAPI

SOCK = os.path.join(tempfile.mkdtemp(), "upstream.sock")


async def upstream(reader, writer):
    await reader.readuntil(b"\r\n\r\n")
    writer.write(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
    await writer.drain()
    writer.close()


@asynccontextmanager
async def lifespan(app):
    server = await asyncio.start_unix_server(upstream, SOCK)
    yield
    server.close()


app = FastAPI(lifespan=lifespan)


@app.get("/async")
async def fetch_async():
    async with httpx.AsyncClient(transport=httpx.AsyncHTTPTransport(uds=SOCK)) as client:
        r = await client.get("http://upstream/")
    return {"status": r.status_code, "body": r.text}


@app.get("/sync")
async def fetch_sync():
    with httpx.Client(transport=httpx.HTTPTransport(uds=SOCK)) as client:
        r = client.get("http://upstream/")  # expect: httpx._client.Client.get
    return {"status": str(r.status_code)}
