import threading
from contextlib import ExitStack, closing

import requests
from fastapi import FastAPI

app = FastAPI()
lock = threading.Lock()


class Conn:
    def close(self) -> None:
        requests.post("https://example.com/close")


@app.get("/stack")
async def stack():
    with ExitStack() as es:
        es.enter_context(lock)  # expect: _thread.lock.__enter__
    with closing(Conn()):  # expect: requests.api.post
        pass
    return {}
