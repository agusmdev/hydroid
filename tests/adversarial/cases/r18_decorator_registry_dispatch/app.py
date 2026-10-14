import asyncio
from pathlib import Path
from typing import Callable

import requests
from fastapi import FastAPI

app = FastAPI()

EXPORTERS: dict[str, Callable[[str], str]] = {}


def exporter(fmt: str):
    def register(fn: Callable[[str], str]) -> Callable[[str], str]:
        EXPORTERS[fmt] = fn
        return fn

    return register


@exporter("csv")
def export_csv(report_id: str) -> str:
    path = Path(f"/tmp/report-{report_id}.csv")
    path.write_text("id\n")
    return str(path)


@exporter("s3")
def export_s3(report_id: str) -> str:
    resp = requests.put(f"https://storage.internal/reports/{report_id}", data=b"id\n")
    return resp.text


@app.post("/reports/{report_id}/export")
async def export_report(report_id: str, fmt: str = "csv"):
    handler = EXPORTERS[fmt]
    location = handler(report_id)  # expect: pathlib.Path.write_text
    return {"location": location}


@app.post("/reports/{report_id}/export-offloaded")
async def export_report_offloaded(report_id: str, fmt: str = "csv"):
    handler = EXPORTERS[fmt]
    location = await asyncio.to_thread(handler, report_id)
    return {"location": location}
