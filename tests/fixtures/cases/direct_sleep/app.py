import asyncio
import time

from fastapi import FastAPI

app = FastAPI()


@app.get("/slow")
async def slow():
    time.sleep(1)  # expect: time.sleep
    return {"ok": True}


@app.get("/fine")
async def fine():
    await asyncio.sleep(1)
    return {"ok": True}


@app.get("/sync")
def sync_endpoint():
    # FastAPI runs `def` endpoints in a threadpool: blocking here is fine.
    time.sleep(1)
    return {"ok": True}
