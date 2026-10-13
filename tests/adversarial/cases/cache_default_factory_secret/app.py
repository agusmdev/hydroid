import asyncio
import os
from pathlib import Path

from fastapi import FastAPI
from pydantic import BaseModel, Field

SECRETS_DIR = Path(os.environ.get("SECRETS_DIR", "/run/secrets"))


def read_secret(name: str = "api_key") -> str:
    path = SECRETS_DIR / name
    try:
        return path.read_text().strip()
    except OSError:
        return ""


def read_api_key() -> str:
    return read_secret("api_key")


class Settings(BaseModel):
    app_name: str = "billing"
    api_key: str = Field(default_factory=read_api_key)


class ProviderConfig(BaseModel):
    region: str = "eu-west-1"


app = FastAPI()


@app.get("/settings")
async def show_settings():
    settings = Settings()  # expect: pathlib.Path.read_text
    return {"app": settings.app_name, "has_key": bool(settings.api_key)}


@app.get("/provider")
async def provider():
    cfg = ProviderConfig()
    return {"region": cfg.region}


@app.get("/settings-offloaded")
async def settings_offloaded():
    settings = await asyncio.to_thread(Settings)
    return {"app": settings.app_name}


@app.get("/explicit")
async def explicit():
    settings = Settings(api_key="given")
    return {"app": settings.app_name}
