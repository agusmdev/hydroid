import time

import requests
from fastapi import FastAPI

app = FastAPI()


@app.get("/local")
async def local(url: str):
    def fetch() -> bytes:
        return requests.get(url).content

    load = lambda: open("x").read()  # noqa: E731
    fetch()  # expect: requests.api.get
    load()  # expect: builtins.open
    pause = time.sleep
    pause(0.1)  # expect: time.sleep
    return {}
