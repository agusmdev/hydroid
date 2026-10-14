import asyncio
import queue
from dataclasses import dataclass, field
from typing import Any, Callable

import requests
from fastapi import FastAPI

app = FastAPI()


@dataclass
class Job:
    fn: Callable[..., Any]
    args: tuple = ()
    kwargs: dict = field(default_factory=dict)


class JobQueue:
    """In-process job queue; a worker drains it, or it is drained after each request."""

    def __init__(self) -> None:
        self.pending: queue.Queue[Job] = queue.Queue()

    def enqueue(self, fn: Callable[..., Any], *args: Any, **kwargs: Any) -> None:
        self.pending.put_nowait(Job(fn, args, kwargs))

    def run_pending(self) -> int:
        done = 0
        while True:
            try:
                job = self.pending.get_nowait()
            except queue.Empty:
                return done
            job.fn(*job.args, **job.kwargs)
            done += 1


jobs = JobQueue()


def notify_billing(account_id: str) -> None:
    try:
        requests.post("https://billing.example.com/notify", json={"account": account_id}, timeout=5)
    except requests.RequestException:
        pass


@app.post("/accounts/{account_id}/upgrade")
async def upgrade(account_id: str):
    jobs.enqueue(notify_billing, account_id)
    ran = jobs.run_pending()  # expect: requests.api.post
    return {"ran": ran}


@app.post("/accounts/{account_id}/upgrade-offloaded")
async def upgrade_offloaded(account_id: str):
    jobs.enqueue(notify_billing, account_id)
    ran = await asyncio.to_thread(jobs.run_pending)
    return {"ran": ran}
