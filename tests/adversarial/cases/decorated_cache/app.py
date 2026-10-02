import functools
import json
from contextlib import contextmanager

import requests
from fastapi import FastAPI

app = FastAPI()


@functools.lru_cache
def settings() -> dict:
    with open("settings.json") as f:
        return json.load(f)


@functools.cache
def remote_flags() -> dict:
    return requests.get("https://example.com/flags").json()


@contextmanager
def opened(path: str):
    f = open(path)
    try:
        yield f
    finally:
        f.close()


class Client:
    @functools.cached_property
    def token(self) -> str:
        return requests.post("https://example.com/token").text

    @staticmethod
    @functools.lru_cache(maxsize=1)
    def region() -> str:
        return requests.get("https://example.com/region").text


@app.get("/cached")
async def cached():
    settings()  # expect: builtins.open
    remote_flags()  # expect: requests.api.get
    with opened("x") as f:  # expect: builtins.open
        print(f)
    client = Client()
    client.token  # expect: requests.api.post
    Client.region()  # expect: requests.api.get
    return {}
