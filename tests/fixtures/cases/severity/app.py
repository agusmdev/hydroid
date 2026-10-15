import asyncio
import subprocess
import time
from contextlib import asynccontextmanager
from pathlib import Path

import requests
from fastapi import FastAPI

CONFIG = Path("config.toml")


# --- what a call waits for ---------------------------------------------------------------------


def has_config() -> bool:
    return CONFIG.exists()


def load_remote_config() -> dict:
    return requests.get("https://example.com/config").json()


def config() -> dict:
    # A file check one step away, an HTTP request two steps away.
    if has_config():
        return {}
    return load_remote_config()


async def shared_warmup():
    time.sleep(1)  # expect: time.sleep@critical


async def poll_forever():
    while True:
        time.sleep(5)  # expect: time.sleep@critical


async def migrate():
    subprocess.run(["migrate"])  # expect: subprocess.run@warn via lifecycle lifespan


# --- where it runs -----------------------------------------------------------------------------


@asynccontextmanager
async def lifespan(app: FastAPI):
    subprocess.run(["migrate"])  # expect: subprocess.run@warn via lifecycle lifespan
    has_config()  # expect: pathlib.Path.exists@warn
    await migrate()
    await shared_warmup()
    # A task runs while the server serves requests.
    task = asyncio.create_task(poll_forever())
    yield
    task.cancel()


app = FastAPI(lifespan=lifespan)


@app.on_event("shutdown")
def flush():
    time.sleep(1)  # expect: time.sleep@warn via lifecycle shutdown


@app.get("/exists")
async def exists():
    return CONFIG.exists()  # expect: pathlib.Path.exists@warn via route GET /exists


@app.get("/config")
async def get_config():
    return config()  # expect: requests.api.get@critical via route GET /config


@app.get("/warmup")
async def warmup():
    await shared_warmup()


@app.get("/ask")
async def ask():
    return input("name? ")  # expect: builtins.input@critical
