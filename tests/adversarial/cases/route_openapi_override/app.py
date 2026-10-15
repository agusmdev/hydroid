from pathlib import Path

from fastapi import FastAPI
from fastapi.openapi.utils import get_openapi

CHANGELOG = Path(__file__).with_name("CHANGELOG.md")

app = FastAPI(title="Orders API")


@app.get("/orders")
async def list_orders() -> list[dict]:
    return []


def custom_openapi() -> dict:
    schema = get_openapi(title=app.title, version=app.version, routes=app.routes)
    # Always show the latest release notes in /docs, without a restart.
    schema["info"]["description"] = CHANGELOG.read_text()  # expect: pathlib.Path.read_text
    return schema


app.openapi = custom_openapi
