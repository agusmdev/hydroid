import os
import time
from typing import Annotated

import requests
from fastapi import Body, Depends, FastAPI, Query
from pydantic import AfterValidator, BaseModel, field_validator

app = FastAPI()


def known_region(value: str) -> str:
    requests.get(f"https://regions.example.com/{value}")  # expect: requests.api.get
    return value


class Filters(BaseModel):
    path: str = "data"

    @field_validator("path")
    @classmethod
    def exists(cls, value: str) -> str:
        os.path.exists(value)  # expect: genericpath.exists
        return value


def region_scope(region: Annotated[str, AfterValidator(known_region), Query()] = "eu") -> str:
    # The dependency body runs in the threadpool; its parameters are validated before that,
    # on the event loop, by solve_dependencies.
    time.sleep(0.01)
    return region


def search_filters(filters: Annotated[Filters, Body()]) -> Filters:
    os.path.exists(filters.path)
    return filters


@app.post("/search")
async def search(
    region: Annotated[str, Depends(region_scope)],
    filters: Annotated[Filters, Depends(search_filters)],
):
    return {"region": region, "path": filters.path}
