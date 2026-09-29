"""Catalog entries reached through the spellings people actually write."""

import asyncio
import hashlib
import os.path
import queue
import shutil
import socket
import sqlite3
import subprocess
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import httpx
import pymysql
import tenacity
from fastapi import FastAPI

app = FastAPI()
jobs: queue.Queue[int] = queue.Queue()


@app.get("/stdlib")
async def stdlib():
    os.path.exists("/tmp/x")  # expect: genericpath.exists
    shutil.rmtree("/tmp/x")  # expect: shutil._RmtreeType.__call__
    socket.gethostbyname("example.com")  # expect: _socket.gethostbyname
    sqlite3.connect(":memory:")  # expect: _sqlite3.connect
    subprocess.check_output(["ls"])  # expect: subprocess.check_output
    Path("/tmp/x").read_text()  # expect: pathlib.Path.read_text
    urllib.request.urlopen("https://example.com")  # expect: urllib.request.urlopen
    jobs.get()  # expect: queue.Queue.get
    ThreadPoolExecutor().submit(print).result()  # expect: concurrent.futures._base.Future.result
    asyncio.run(asyncio.sleep(0))  # expect: asyncio.runners.run
    hashlib.pbkdf2_hmac("sha256", b"pw", b"salt", 100_000)  # CPU-bound: only with --cpu
    input()  # expect: builtins.input


@app.get("/libraries")
async def libraries():
    httpx.get("https://example.com")  # expect: httpx._api.get
    with httpx.Client() as client:
        client.get("https://example.com")  # expect: httpx._client.Client.get
    async with httpx.AsyncClient() as client:
        await client.get("https://example.com")
    pymysql.connect(host="db")  # expect: pymysql.connections.Connection.__init__
    for attempt in tenacity.Retrying():  # expect: tenacity.BaseRetrying.__iter__
        pass
    tenacity.Retrying()(print)  # expect: tenacity.Retrying.__call__
    fetch_with_retries()  # expect: tenacity.retry
    await fetch_async_with_retries()


@tenacity.retry(stop=tenacity.stop_after_attempt(3))
def fetch_with_retries() -> None:
    pass


@tenacity.retry(stop=tenacity.stop_after_attempt(3))
async def fetch_async_with_retries() -> None:
    pass
