import json

from fastapi import FastAPI

app = FastAPI()


class LazySettings:
    def __init__(self) -> None:
        self._data: dict | None = None

    def __getattr__(self, name: str):
        if self._data is None:
            with open("settings.json") as f:
                self._data = json.load(f)
        return self._data[name]


settings = LazySettings()


@app.get("/debug")
async def debug():
    return settings.DEBUG  # expect: builtins.open
