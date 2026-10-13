import csv
import io
import time
from typing import Iterator

from fastapi import FastAPI
from fastapi.responses import StreamingResponse

app = FastAPI()


def load_row(row_id: int) -> list[str]:
    time.sleep(0.01)  # stands in for a slow sync lookup
    return [str(row_id), f"item-{row_id}"]


def to_csv_line(row: list[str]) -> str:
    buf = io.StringIO()
    csv.writer(buf).writerow(row)
    return buf.getvalue()


@app.get("/export.csv")
async def export_csv(n: int):
    # map() is lazy: load_row runs only when StreamingResponse pulls the next item,
    # and Starlette pulls items of a sync iterator in a worker thread.
    lines = map(lambda i: to_csv_line(load_row(i)), range(n))
    return StreamingResponse(lines, media_type="text/csv")


@app.get("/export-direct.csv")
async def export_direct(n: int):
    return StreamingResponse(map(to_csv_line, map(load_row, range(n))), media_type="text/csv")


@app.get("/export-eager.csv")
async def export_eager(n: int):
    rows = list(map(load_row, range(n)))  # expect: time.sleep
    return StreamingResponse(iter([to_csv_line(r) for r in rows]), media_type="text/csv")
