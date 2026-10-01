import requests
from fastapi import FastAPI

app = FastAPI()


class Fetcher:
    def __call__(self, url: str) -> bytes:
        return requests.get(url).content


class Upper:
    def __call__(self, s: str) -> str:
        return s.upper()


fetcher = Fetcher()


@app.get("/call")
async def call_route(url: str):
    fetcher(url)  # expect: requests.api.get
    Fetcher()(url)  # expect: requests.api.get
    local = Fetcher()
    local(url)  # expect: requests.api.get
    Upper()("x")
    return {}
