import os

import requests
from fastapi import FastAPI
from pydantic import BaseModel, field_validator

app = FastAPI()


class AvatarOut(BaseModel):
    user: str
    avatar_url: str

    @field_validator("avatar_url")
    @classmethod
    def check_reachable(cls, value: str) -> str:
        requests.head(value)  # expect: requests.api.head
        return value


class FileOut(BaseModel):
    path: str

    @field_validator("path")
    @classmethod
    def must_exist(cls, value: str) -> str:
        os.path.exists(value)  # expect: genericpath.exists
        return value


class NameOut(BaseModel):
    name: str

    @field_validator("name")
    @classmethod
    def strip(cls, value: str) -> str:
        return value.strip()


# The endpoints return plain dicts: FastAPI validates them against response_model on the loop
# (async endpoint), so the validators run there. No model is constructed in user code.
@app.get("/avatar", response_model=AvatarOut)
async def avatar(user: str):
    return {"user": user, "avatar_url": "http://cdn.invalid/" + user}


@app.get("/file")
async def file(path: str) -> FileOut:
    return {"path": path}  # type: ignore[return-value]


@app.get("/name", response_model=NameOut)
async def name(n: str):
    return {"name": n}
