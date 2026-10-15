import asyncio
from typing import IO

from fastapi import FastAPI
from fastapi.responses import StreamingResponse

app = FastAPI()


class LogFollower:
    """Async iterator over a log file, used for a server-sent-events stream."""

    def __init__(self, path: str, max_lines: int = 2) -> None:
        self.path = path
        self.max_lines = max_lines
        self.sent = 0
        self.fh: IO[str] | None = None

    def __aiter__(self) -> "LogFollower":
        # `async for` calls __aiter__ synchronously on the loop.
        self.fh = open(self.path)  # expect: builtins.open
        return self

    async def __anext__(self) -> str:
        if self.sent >= self.max_lines:
            raise StopAsyncIteration
        self.sent += 1
        await asyncio.sleep(0)
        return f"data: line {self.sent}\n\n"


class OffloadedLogFollower:
    def __init__(self, path: str) -> None:
        self.path = path
        self.fh: IO[str] | None = None
        self.done = False

    def __aiter__(self) -> "OffloadedLogFollower":
        return self

    async def __anext__(self) -> str:
        if self.done:
            raise StopAsyncIteration
        self.done = True
        try:
            self.fh = await asyncio.to_thread(open, self.path)
        except OSError:
            return "data: missing\n\n"
        return "data: opened\n\n"


@app.get("/logs/follow")
async def follow(path: str):
    return StreamingResponse(LogFollower(path), media_type="text/event-stream")


@app.get("/logs/follow-offloaded")
async def follow_offloaded(path: str):
    return StreamingResponse(OffloadedLogFollower(path), media_type="text/event-stream")
