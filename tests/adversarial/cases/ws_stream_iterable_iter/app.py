import sqlite3
from typing import Iterator

from fastapi import FastAPI
from fastapi.responses import StreamingResponse

app = FastAPI()

DB_PATH = "reports.db"


class CsvExport:
    """Streams query results as CSV lines; the query runs when iteration starts."""

    def __init__(self, limit: int) -> None:
        self.limit = limit

    def __iter__(self) -> Iterator[str]:
        conn = sqlite3.connect(DB_PATH)  # expect: _sqlite3.connect
        try:
            rows = list(conn.execute("SELECT id, name FROM items LIMIT ?", (self.limit,)))
        finally:
            conn.close()
        return iter(f"{r[0]},{r[1]}\n" for r in rows)


class LazyCsvExport:
    """Same export, but __iter__ is a generator: iter() only creates it."""

    def __init__(self, limit: int) -> None:
        self.limit = limit

    def __iter__(self) -> Iterator[str]:
        conn = sqlite3.connect(DB_PATH)
        try:
            for r in conn.execute("SELECT id, name FROM items LIMIT ?", (self.limit,)):
                yield f"{r[0]},{r[1]}\n"
        except sqlite3.Error:
            yield "error\n"
        finally:
            conn.close()


@app.get("/export.csv")
async def export_csv(limit: int):
    # Starlette's iterate_in_threadpool calls iter(content) on the loop, then next() in a thread.
    return StreamingResponse(CsvExport(limit), media_type="text/csv")


@app.get("/export-lazy.csv")
async def export_lazy_csv(limit: int):
    return StreamingResponse(LazyCsvExport(limit), media_type="text/csv")
