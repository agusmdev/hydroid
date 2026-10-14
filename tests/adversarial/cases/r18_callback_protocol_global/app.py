import asyncio
import json
from typing import Protocol

from fastapi import FastAPI

app = FastAPI()


class FeatureFlagSource(Protocol):
    def __call__(self, tenant: str) -> dict[str, bool]: ...


def load_flags_from_disk(tenant: str) -> dict[str, bool]:
    try:
        with open(f"/etc/flags/{tenant}.json") as fh:
            return json.load(fh)
    except (OSError, ValueError):
        return {}


def static_flags(tenant: str) -> dict[str, bool]:
    return {"beta": False}


flag_source: FeatureFlagSource = load_flags_from_disk


@app.get("/tenants/{tenant}/flags")
async def flags(tenant: str):
    return flag_source(tenant)  # expect: builtins.open


@app.get("/tenants/{tenant}/flags/offloaded")
async def flags_offloaded(tenant: str):
    return await asyncio.to_thread(flag_source, tenant)
