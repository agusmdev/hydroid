import threading
import time
from abc import ABC, abstractmethod
from functools import cached_property
from typing import Protocol

import redis
import redis.asyncio
import requests
from fastapi import FastAPI
from sqlalchemy import create_engine, text
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import Session

app = FastAPI()
lock = threading.Lock()


# --- context managers, properties, constructors, iteration -----------------------------------


class Resource:
    def __enter__(self):
        time.sleep(1)
        return self

    def __exit__(self, *exc):
        return False


class Remote:
    def __init__(self, url: str):
        self.payload = requests.get(url).json()

    @property
    def status(self) -> int:
        return requests.head("https://example.com").status_code

    @cached_property
    def config(self) -> dict:
        return requests.get("https://example.com/config").json()

    @property
    def name(self) -> str:
        return "remote"


class Pages:
    def __iter__(self):
        for page in range(3):
            yield requests.get(f"https://example.com/{page}")


def slow_rows():
    for row in range(3):
        time.sleep(0.1)
        yield row


@app.get("/implicit")
async def implicit():
    with lock:  # expect: _thread.lock.__enter__
        pass
    with Resource():  # expect: time.sleep
        pass
    remote = Remote("https://example.com")  # expect: requests.api.get
    print(remote.status)  # expect: requests.api.head
    print(remote.config)  # expect: requests.api.get
    print(remote.name)
    for page in Pages():  # expect: requests.api.get
        print(page)
    for row in slow_rows():  # expect: time.sleep
        print(row)


# --- dynamic dispatch -----------------------------------------------------------------------


class Repo(ABC):
    @abstractmethod
    def get(self, key: str) -> str: ...


class SqlRepo(Repo):
    def get(self, key: str) -> str:
        time.sleep(0.1)
        return key


class MemoryRepo(Repo):
    def get(self, key: str) -> str:
        return key


class Store(Protocol):
    def load(self) -> bytes: ...


class DiskStore:
    def load(self) -> bytes:
        with open("/tmp/blob", "rb") as f:
            return f.read()


class Base:
    def save(self) -> None:
        requests.post("https://example.com")


class Child(Base):
    def save(self) -> None:
        super().save()


@app.get("/dispatch")
async def dispatch(repo: Repo, store: Store):
    repo.get("k")  # expect: time.sleep
    store.load()  # expect: builtins.open
    Child().save()  # expect: requests.api.post


# --- dual sync/async APIs ---------------------------------------------------------------------


@app.get("/clients")
async def clients(session: AsyncSession):
    cache = redis.asyncio.Redis()
    await cache.get("k")
    sync_cache = redis.Redis()
    sync_cache.get("k")  # expect: redis.commands.core.BasicKeyCommands.get
    await session.execute(text("select 1"))
    with Session(create_engine("sqlite://")) as db:
        db.execute(text("select 1"))  # expect: sqlalchemy.orm.session.Session.execute
