# Sync and async redis clients share their command methods (`redis.commands.core`): awaited on
# the async client, `get` only builds a coroutine; on the sync client it blocks.
import redis
import redis.asyncio as aioredis
from fastapi import FastAPI

app = FastAPI()
async_client = aioredis.Redis()
sync_client = redis.Redis()


@app.get("/async")
async def read_async():
    try:
        value = await async_client.get("k")
    except Exception:
        value = None
    return {"value": str(value)}


@app.get("/sync")
async def read_sync():
    value = sync_client.get("k")  # expect: redis.commands.core.BasicKeyCommands.get
    return {"value": str(value)}
