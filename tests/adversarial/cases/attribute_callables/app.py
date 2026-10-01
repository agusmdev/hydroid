from typing import Callable

import requests
from fastapi import FastAPI

app = FastAPI()


def download(url: str) -> bytes:
    return requests.get(url).content


def fake(url: str) -> bytes:
    return b""


class Service:
    def __init__(self, fetch: Callable[[str], bytes]) -> None:
        self.fetch = fetch

    async def run(self, url: str) -> bytes:
        return self.fetch(url)  # expect: requests.api.get


service = Service(fetch=download)


class Holder:
    fetch = staticmethod(download)


@app.get("/fetch")
async def fetch_route(url: str):
    handler = Holder.fetch
    handler(url)  # expect: requests.api.get
    return await service.run(url)
