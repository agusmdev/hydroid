import asyncio
from collections import defaultdict
from typing import Callable

import requests
from fastapi import FastAPI
from pydantic import BaseModel

app = FastAPI()


class EventBus:
    def __init__(self) -> None:
        self._handlers: dict[str, list[Callable[[dict], None]]] = defaultdict(list)

    def on(self, event: str):
        def register(fn: Callable[[dict], None]) -> Callable[[dict], None]:
            self._handlers[event].append(fn)
            return fn

        return register

    def emit(self, event: str, payload: dict) -> None:
        for handler in self._handlers[event]:
            handler(payload)


bus = EventBus()


@bus.on("user.created")
def notify_crm(payload: dict) -> None:
    requests.post("https://crm.internal/hooks/users", json=payload, timeout=2)


@bus.on("user.created")
def audit(payload: dict) -> None:
    print("audit", payload)


class UserIn(BaseModel):
    email: str


@app.post("/users")
async def create_user(user: UserIn):
    payload = {"email": user.email}
    bus.emit("user.created", payload)  # expect: requests.api.post
    return payload


@app.post("/users/bulk")
async def create_users(user: UserIn):
    payload = {"email": user.email}
    await asyncio.to_thread(bus.emit, "user.created", payload)
    return payload


@app.post("/users/sync")
def create_user_sync(user: UserIn):
    payload = {"email": user.email}
    bus.emit("user.created", payload)
    return payload
