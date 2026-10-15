import asyncio
import json
import os
import queue
from typing import Any, Callable

import requests
from fastapi import FastAPI
from pydantic import BaseModel


class Settings:
    env: str = os.environ.get("APP_ENV", "development")

    @property
    def run_jobs_inline(self) -> bool:
        return self.env == "development"


settings = Settings()

_broker: "queue.Queue[str]" = queue.Queue()


class Job:
    """Thin wrapper so tasks look like `send_invoice.enqueue(...)` in services.

    In production the payload goes to the broker for the worker; in development the job
    runs inline so engineers don't need a worker process.
    """

    def __init__(self, func: Callable[..., Any]) -> None:
        self.func = func
        self.name = func.__name__

    def enqueue(self, *args: Any) -> None:
        if settings.run_jobs_inline:
            self.func(*args)
            return
        _broker.put_nowait(json.dumps({"job": self.name, "args": args}))


def job(func: Callable[..., Any]) -> Job:
    return Job(func)


@job
def sync_customer_to_crm(customer_id: str) -> None:
    requests.post(
        "https://crm.internal/api/customers/sync",
        json={"id": customer_id},
        timeout=5,
    )


@job
def warm_cache(customer_id: str) -> None:
    print("warming", customer_id)


class CustomerIn(BaseModel):
    id: str
    name: str


class CustomerService:
    async def create(self, data: CustomerIn) -> dict:
        customer = {"id": data.id, "name": data.name}
        warm_cache.enqueue(data.id)
        sync_customer_to_crm.enqueue(data.id)  # expect: requests.api.post
        return customer


app = FastAPI()


@app.post("/customers")
async def create_customer(body: CustomerIn):
    return await CustomerService().create(body)


@app.post("/customers/batch")
async def create_customer_offloaded(body: CustomerIn):
    await asyncio.to_thread(sync_customer_to_crm.enqueue, body.id)
    return {"id": body.id}
