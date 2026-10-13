import asyncio
import time
from abc import ABC, abstractmethod
from typing import Generic, TypeVar

import requests
from fastapi import FastAPI

app = FastAPI()


# --- exact instances: a construction runs the methods its class's MRO finds ---------------------


class Model:
    def refresh(self) -> None:
        self.hydrate()

    def hydrate(self) -> None:
        pass


class FileModel(Model):
    def hydrate(self) -> None:
        time.sleep(0.1)


class SlowModel(Model):
    def refresh(self) -> None:
        time.sleep(0.1)


@app.get("/model")
async def model_refresh():
    # `FileModel.hydrate` overrides `hydrate` in a subclass: a `Model()` never runs it.
    Model().refresh()
    Model().hydrate()


@app.get("/file-model")
async def file_model_refresh():
    FileModel().refresh()  # expect: time.sleep


@app.get("/any-model")
async def any_model_refresh(kind: str):
    model = make_model(kind)
    model.refresh()  # expect: time.sleep


def make_model(kind: str) -> Model:
    return FileModel() if kind == "file" else SlowModel()


singleton = Model()


@app.get("/singleton")
async def singleton_refresh():
    singleton.refresh()


# --- variables only ever assigned constructions -------------------------------------------------


class WeatherClient:
    def forecast(self, city: str) -> dict:
        return requests.get(f"https://weather.example.com/{city}", timeout=5).json()

    def summary(self, city: str) -> str:
        return str(self.forecast(city))


class AsyncWeatherClient(WeatherClient):
    async def forecast(self, city: str) -> dict:  # type: ignore[override]
        await asyncio.sleep(0)
        return {"city": city}

    async def summary(self, city: str) -> str:  # type: ignore[override]
        return str(await self.forecast(city))


class CachedWeatherClient(WeatherClient):
    pass


weather: WeatherClient = AsyncWeatherClient()
legacy: WeatherClient = CachedWeatherClient()


def default_client() -> WeatherClient:
    return AsyncWeatherClient()


factory_made: WeatherClient = default_client()


@app.get("/forecast/{city}")
async def forecast(city: str):
    data = await weather.forecast(city)  # type: ignore[misc]
    text = await weather.summary(city)  # type: ignore[misc]
    return {"data": data, "text": text}


@app.get("/legacy/{city}")
async def legacy_forecast(city: str):
    return legacy.forecast(city)  # expect: requests.api.get


@app.get("/factory/{city}")
async def factory_forecast(city: str):
    # Not a construction: the declared class's methods may run.
    return factory_made.forecast(city)  # expect: requests.api.get


@app.get("/local/{city}")
async def local_forecast(city: str, cached: bool):
    client: WeatherClient = AsyncWeatherClient()
    if cached:
        client = CachedWeatherClient()
    return client.summary(city)  # expect: requests.api.get


configured: WeatherClient = AsyncWeatherClient()
patched: WeatherClient = AsyncWeatherClient()


def configure() -> None:
    global configured
    configured = CachedWeatherClient()


@app.get("/configured/{city}")
async def configured_forecast(city: str):
    # Rebound through `global` (and `patched` from another module): not only constructions.
    configured.forecast(city)  # expect: requests.api.get
    patched.forecast(city)  # expect: requests.api.get


# --- generic classes: the type argument picks the implementation --------------------------------


class Repository(ABC):
    @abstractmethod
    def list_ids(self) -> list[str]: ...


class RemoteRepository(Repository):
    def list_ids(self) -> list[str]:
        return list(requests.get("https://catalog.internal/ids", timeout=5).json())


class MemoryRepository(Repository):
    def __init__(self, ids: list[str]) -> None:
        self.ids = ids

    def list_ids(self) -> list[str]:
        return list(self.ids)


RepoT = TypeVar("RepoT", bound=Repository)


class BaseService(Generic[RepoT]):
    def __init__(self, repo: RepoT) -> None:
        self.repo = repo

    def count(self) -> int:
        return len(self.repo.list_ids())

    def summary(self) -> str:
        return f"{self.count()} ids"


class RemoteService(BaseService[RemoteRepository]):
    pass


class MemoryService(BaseService[MemoryRepository]):
    pass


remote_service = RemoteService(RemoteRepository())
memory_service = MemoryService(MemoryRepository(["a"]))


@app.get("/remote/count")
async def remote_count():
    return remote_service.summary()  # expect: requests.api.get


@app.get("/memory/count")
async def memory_count():
    return memory_service.summary()


@app.get("/memory/direct")
async def memory_direct():
    service: BaseService[MemoryRepository] = BaseService(MemoryRepository(["a"]))
    return service.count()


async def count_with(service: BaseService[RemoteRepository], memory: MemoryService, any_service: BaseService):
    service.count()  # expect: requests.api.get
    memory.summary()
    any_service.count()  # expect: requests.api.get
