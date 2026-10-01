import os

import requests
from fastapi import FastAPI
from pydantic import BaseModel, field_validator, model_validator

app = FastAPI()


class Webhook(BaseModel):
    url: str

    @field_validator("url")
    @classmethod
    def reachable(cls, value: str) -> str:
        requests.head(value)
        return value


class Upload(BaseModel):
    path: str

    @model_validator(mode="after")
    def exists(self):
        if not os.path.exists(self.path):
            raise ValueError("missing")
        return self


class Pure(BaseModel):
    name: str

    @field_validator("name")
    @classmethod
    def strip(cls, value: str) -> str:
        return value.strip()


@app.post("/hooks")
async def create_hook(url: str, path: str):
    hook = Webhook(url=url)  # expect: requests.api.head
    upload = Upload.model_validate({"path": path})  # expect: genericpath.exists
    pure = Pure(name="x")
    return hook, upload, pure
