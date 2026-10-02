import time
from typing import Callable

import requests
from fastapi import FastAPI

app = FastAPI()


def load_remote() -> bytes:
    return requests.get("https://example.com").content


def get_loader() -> Callable[[], bytes]:
    return load_remote


def make_sleeper(seconds: float):
    def sleeper() -> None:
        time.sleep(seconds)

    return sleeper


@app.get("/loader")
async def loader():
    get_loader()()  # expect: requests.api.get
    nap = make_sleeper(1)
    nap()  # expect: time.sleep
    return {}
