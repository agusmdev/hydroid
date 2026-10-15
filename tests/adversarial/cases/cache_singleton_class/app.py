import asyncio
import functools
import json
from pathlib import Path

from fastapi import FastAPI

app = FastAPI()


@functools.cache
class FeatureFlags:
    """Process-wide singleton: functools.cache returns the same instance on every call."""

    def __init__(self) -> None:
        self.flags: dict = json.loads(Path("flags.json").read_text())

    def enabled(self, name: str) -> bool:
        return bool(self.flags.get(name))


class Defaults:
    def __init__(self) -> None:
        self.flags = {"beta": False}


@app.get("/flags/{name}")
async def flag(name: str):
    return {"enabled": FeatureFlags().enabled(name)}  # expect: pathlib.Path.read_text


@app.get("/defaults")
async def defaults():
    return Defaults().flags


@app.get("/flags-offloaded")
async def flags_offloaded():
    flags = await asyncio.to_thread(FeatureFlags)
    return flags.flags


@app.get("/flags-sync")
def flags_sync():
    return FeatureFlags().flags
