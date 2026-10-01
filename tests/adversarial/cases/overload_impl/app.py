from typing import overload

from fastapi import FastAPI

app = FastAPI()


@overload
def load(key: int) -> bytes: ...
@overload
def load(key: str) -> bytes: ...
def load(key: int | str) -> bytes:
    with open(f"/data/{key}", "rb") as f:
        return f.read()


@app.get("/blob/{key}")
async def blob(key: str):
    return load(key)  # expect: builtins.open
