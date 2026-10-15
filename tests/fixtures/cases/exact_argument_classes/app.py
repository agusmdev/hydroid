"""A helper typed with an abstract stream reads whatever its callers give it. A caller passing an
instance it constructs (exactly of that class) gets the helper's calls on that parameter
dispatched on that class: an in-memory buffer reads no file, a project stream may block."""

import io
import time
from typing import BinaryIO

from fastapi import FastAPI

app = FastAPI()


class SlowStream(io.BytesIO):
    def read(self, size: int | None = -1) -> bytes:
        time.sleep(0.01)
        return super().read(size)


def checksum(stream: BinaryIO) -> int:
    return sum(stream.read())


def first_line(stream: BinaryIO) -> bytes:
    return stream.readline()


@app.post("/checksum")
async def upload(payload: bytes):
    return {"sum": checksum(io.BytesIO(payload))}


@app.post("/slow-checksum")
async def slow_upload(payload: bytes):
    return {"sum": checksum(SlowStream(payload))}  # expect: time.sleep


@app.get("/first-line")
async def head():
    with open("data.bin", "rb") as f:  # expect: builtins.open
        # A real file: the read may block.
        line = first_line(f)  # expect: typing.IO.readline
    buffer = io.BytesIO(b"x\ny")
    return {"line": first_line(buffer) + line}
