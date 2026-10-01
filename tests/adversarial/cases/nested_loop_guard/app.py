import asyncio

from fastapi import FastAPI

app = FastAPI()


async def notify() -> None:
    await asyncio.sleep(0)


def notify_from_sync() -> None:
    # The usual bridge: schedule on the running loop, start one only when there is none.
    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        loop = None
    if loop is not None:
        loop.create_task(notify())
    else:
        asyncio.run(notify())


@app.post("/events")
async def events():
    notify_from_sync()
    asyncio.run(notify())  # expect: asyncio.runners.run
    return {}
