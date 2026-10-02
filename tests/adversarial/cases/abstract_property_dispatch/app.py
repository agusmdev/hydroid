from abc import ABC, abstractmethod

import requests
from fastapi import FastAPI

app = FastAPI()


class Source(ABC):
    @property
    @abstractmethod
    def payload(self) -> bytes: ...


class Remote(Source):
    @property
    def payload(self) -> bytes:
        return requests.get("https://example.com").content


class Static(Source):
    @property
    def payload(self) -> bytes:
        return b""


@app.get("/payload")
async def payload(source: Source):
    return source.payload  # expect: requests.api.get
