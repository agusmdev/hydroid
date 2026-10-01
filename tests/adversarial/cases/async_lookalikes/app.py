import asyncio

import anyio
import httpx
import redis.asyncio
from fastapi import FastAPI
from sqlalchemy import text
from sqlalchemy.ext.asyncio import AsyncSession, create_async_engine

app = FastAPI()
lock = asyncio.Lock()
sem = asyncio.Semaphore(3)
r = redis.asyncio.Redis()
engine = create_async_engine("sqlite+aiosqlite://")


@app.get("/async")
async def async_route():
    async with lock:
        pass
    async with sem:
        pass
    q: asyncio.Queue[int] = asyncio.Queue()
    q.put_nowait(1)
    q.get_nowait()
    fut: asyncio.Future[int] = asyncio.get_running_loop().create_future()
    fut.set_result(1)
    fut.result()
    event = asyncio.Event()
    event.set()
    await event.wait()
    async with httpx.AsyncClient() as client:
        await client.get("https://example.com")
    await r.get("k")
    async with AsyncSession(engine) as session:
        await session.execute(text("select 1"))
        await session.commit()
    await anyio.Path("x").read_text()
    await asyncio.sleep(1)
    task = asyncio.create_task(asyncio.sleep(1))
    await asyncio.wait_for(task, 1)
    return {}
