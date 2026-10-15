import asyncio
import time
from typing import Callable, Protocol

from fastapi import Depends, FastAPI

app = FastAPI()


class Backoff(Protocol):
    sleep: Callable[[float], None]

    def delays(self) -> list[float]: ...


class ExponentialBackoff:
    def __init__(self, base: float = 0.01, attempts: int = 3) -> None:
        self.base = base
        self.attempts = attempts
        self.sleep = time.sleep

    def delays(self) -> list[float]:
        return [self.base * 2**i for i in range(self.attempts)]


class NoBackoff:
    sleep = staticmethod(lambda seconds: None)

    def delays(self) -> list[float]:
        return []


def get_backoff() -> Backoff:
    return ExponentialBackoff()


def get_test_backoff() -> Backoff:
    return NoBackoff()


@app.post("/jobs/{job_id}/retry")
async def retry_job(job_id: str, backoff: Backoff = Depends(get_backoff)):
    for delay in backoff.delays()[:1]:
        backoff.sleep(delay)  # expect: time.sleep
    return {"job": job_id, "retried": True}


@app.post("/jobs/{job_id}/retry-fast")
async def retry_job_fast(job_id: str, backoff: Backoff = Depends(get_test_backoff)):
    backoff.sleep(0.0)
    return {"job": job_id, "retried": True}


@app.post("/jobs/{job_id}/retry-offloaded")
async def retry_job_offloaded(job_id: str, backoff: Backoff = Depends(get_backoff)):
    for delay in backoff.delays()[:1]:
        await asyncio.to_thread(backoff.sleep, delay)
    return {"job": job_id, "retried": True}
