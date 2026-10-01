import time

from fastapi import FastAPI
from fastapi.responses import StreamingResponse

app = FastAPI()


def read_chunks(path: str):
    with open(path, "rb") as f:
        while chunk := f.read(65536):
            yield chunk


def slow_numbers():
    for i in range(10):
        time.sleep(0.1)
        yield str(i)


@app.get("/download")
async def download(path: str):
    # Starlette iterates sync iterators in a worker thread; creating the generator runs nothing.
    return StreamingResponse(read_chunks(path))


@app.get("/numbers")
async def numbers():
    gen = slow_numbers()
    return StreamingResponse(gen)


@app.get("/consumed")
async def consumed():
    return "".join(slow_numbers())  # expect: time.sleep
