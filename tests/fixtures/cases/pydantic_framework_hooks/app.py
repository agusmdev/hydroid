import os
import time
from collections.abc import Iterator
from pathlib import Path
from typing import Annotated

import requests
from fastapi import Body, Depends, FastAPI
from pydantic import AfterValidator, BaseModel, computed_field, field_serializer, field_validator

app = FastAPI()


# --- request validation: FastAPI validates parameters on the loop, for sync functions too ------


def region_known(value: str) -> str:
    requests.get(f"https://regions.example.com/{value}")  # expect: requests.api.get via route GET /regions (request validation)
    return value


class Address(BaseModel):
    city: str

    @field_validator("city")
    @classmethod
    def city_exists(cls, value: str) -> str:
        os.path.exists(value)  # expect: genericpath.exists
        return value


class Order(BaseModel):
    address: Address


@app.get("/regions")
def regions(region: Annotated[str, AfterValidator(region_known)] = "eu"):
    # The body runs in the threadpool.
    time.sleep(0.01)
    return {"region": region}


@app.post("/orders")
def create_order(order: Order):
    return {"city": order.address.city}


# A `Depends` parameter is filled by calling the dependency: its type is not validated.
class Session(BaseModel):
    user: str

    @field_validator("user")
    @classmethod
    def user_on_disk(cls, value: str) -> str:
        Path(value).read_text()
        return value


def current_session() -> Session:
    return Session.model_construct(user="x")


@app.get("/me")
async def me(session: Annotated[Session, Depends(current_session)]):
    return {"user": session.user}


# --- response serialization: the returned value is dumped on the loop, sync endpoints too --------


class Report(BaseModel):
    name: str

    @computed_field
    @property
    def template(self) -> str:
        return Path("template.txt").read_text()  # expect: pathlib.Path.read_text


class Listing(BaseModel):
    name: str

    @field_serializer("name")
    def label(self, value: str) -> str:
        time.sleep(0.01)  # expect: time.sleep
        return value


@app.get("/report")
def report(name: str) -> Report:
    return Report(name=name)


@app.get("/listings", response_model=list[Listing])
async def listings():
    return [{"name": "a"}]


# --- response validation: on the loop for async endpoints only --------------------------------


class Checked(BaseModel):
    url: str

    @field_validator("url")
    @classmethod
    def reachable(cls, value: str) -> str:
        requests.head(value)  # expect: requests.api.head
        return value


@app.get("/checked", response_model=Checked)
async def checked():
    return {"url": "https://example.com"}


class Probed(BaseModel):
    path: str

    @field_validator("path")
    @classmethod
    def probe(cls, value: str) -> str:
        os.path.isfile(value)
        return value


# Sync endpoint: the return value is validated in the threadpool.
@app.get("/probed", response_model=Probed)
def probed():
    return {"path": "data"}


class Kept(BaseModel):
    path: str

    @field_validator("path")
    @classmethod
    def probe(cls, value: str) -> str:
        os.path.isdir(value)
        return value


# Already a `Kept`: validating it against `Kept` keeps the instance.
@app.get("/kept")
async def kept() -> Kept:
    return Kept.model_construct(path="data")


# A sync generator streams its items from the threadpool.
@app.get("/stream")
def stream() -> Iterator[Report]:
    yield Report.model_construct(name="a")


class Cached(BaseModel):
    key: str

    @field_validator("key")
    @classmethod
    def warm(cls, value: str) -> str:
        time.sleep(0.01)
        return value


def lookup(key):
    return Cached.model_construct(key=key)


# ty does not know what `lookup` returns: it may be (and is) a `Cached`, kept as is.
@app.get("/cached", response_model=Cached)
async def cached():
    return lookup("k")
