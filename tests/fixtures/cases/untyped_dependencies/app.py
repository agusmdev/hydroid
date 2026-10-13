from collections.abc import Iterator

import httpx
import requests
from fastapi import Depends, FastAPI, Security

app = FastAPI()


# --- unannotated `Depends(provider)` parameters hold what the provider returns or yields --------


def get_client():
    with httpx.Client(base_url="https://billing.example.com") as client:
        yield client


def get_session():
    session = requests.Session()
    session.headers["Authorization"] = "Bearer token"
    return session


def make_session() -> requests.Session:
    return build()


def build():
    return requests.Session()


def get_pooled() -> Iterator[httpx.Client]:
    yield pool()


def pool():
    return httpx.Client()


class Gateway:
    def fetch(self, path: str) -> str:
        return requests.get(f"https://gateway.example.com/{path}").text


@app.get("/yielded")
async def yielded(client=Depends(get_client)):
    return client.get("/invoices").status_code  # expect: httpx._client.Client.get


@app.get("/returned")
async def returned(session=Depends(get_session)):
    return session.get("https://users.example.com").status_code  # expect: requests.sessions.Session.get


@app.get("/declared")
async def declared(session=Depends(dependency=make_session)):
    return session.post("https://users.example.com").status_code  # expect: requests.sessions.Session.post


@app.get("/declared-generator")
async def declared_generator(client=Security(get_pooled)):
    return client.get("/").status_code  # expect: httpx._client.Client.get


@app.get("/class")
async def class_dependency(gateway=Depends(Gateway)):
    return gateway.fetch("status")  # expect: requests.api.get


# --- negatives -----------------------------------------------------------------------------------


def get_async_client():
    return httpx.AsyncClient()


def get_settings():
    return {"region": "eu"}


@app.get("/async-client")
async def async_client(client=Depends(get_async_client)):
    return (await client.get("/")).status_code


@app.get("/settings")
async def settings(config=Depends(get_settings)):
    return config.get("region")


@app.get("/threadpool")
def threadpool(client=Depends(get_client)):
    return client.get("/invoices").status_code
