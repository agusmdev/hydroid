import asyncio
import shutil
import socket
from email.utils import make_msgid

from fastapi import FastAPI

app = FastAPI()


@app.post("/export")
async def export(name: str):
    archive = shutil.make_archive(name, "zip", "data")  # expect: shutil.make_archive
    free = shutil.disk_usage("/").free  # expect: shutil.disk_usage
    return {"archive": archive, "free": free, "host": socket.getfqdn()}  # expect: socket.getfqdn


@app.post("/message-id")
async def message_id():
    # Without a domain, `make_msgid` resolves the host name with `socket.getfqdn()`.
    return {"default": make_msgid(), "named": make_msgid(domain="example.com")}  # expect: socket.getfqdn


@app.post("/message-id-offloaded")
async def message_id_offloaded():
    return {"id": await asyncio.to_thread(make_msgid)}
