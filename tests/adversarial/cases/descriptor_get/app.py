import json

from fastapi import FastAPI

app = FastAPI()


class FromFile:
    def __init__(self, path: str) -> None:
        self.path = path

    def __get__(self, obj, objtype=None) -> dict:
        with open(self.path) as f:
            return json.load(f)


class Config:
    flags = FromFile("flags.json")
    name = "app"


@app.get("/flags")
async def flags():
    config = Config()
    return config.flags, config.name  # expect: builtins.open
