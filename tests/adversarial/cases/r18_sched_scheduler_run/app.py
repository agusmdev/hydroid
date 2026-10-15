import asyncio
import sched
import time

from fastapi import FastAPI

app = FastAPI()


class Maintenance:
    """Tiny in-process scheduler for housekeeping jobs."""

    def __init__(self) -> None:
        self.scheduler = sched.scheduler(time.monotonic, time.sleep)
        self.ran: list[str] = []

    def plan(self, delay: float, job: str) -> None:
        self.scheduler.enter(delay, 1, self.ran.append, (job,))

    def drain(self) -> list[str]:
        self.scheduler.run()
        return self.ran

    def tick(self) -> list[str]:
        self.scheduler.run(blocking=False)
        return self.ran


@app.post("/maintenance/run")
async def run_maintenance():
    m = Maintenance()
    m.plan(0.02, "vacuum")
    m.plan(0.04, "reindex")
    return {"ran": m.drain()}  # expect: *


@app.post("/maintenance/inline")
async def run_inline():
    s = sched.scheduler(time.monotonic, time.sleep)
    s.enter(0.02, 1, print, ("cleanup",))
    s.run()  # expect: *
    return {"ok": True}


@app.post("/maintenance/offloaded")
async def run_offloaded():
    m = Maintenance()
    m.plan(0.02, "vacuum")
    return {"ran": await asyncio.to_thread(m.drain)}
