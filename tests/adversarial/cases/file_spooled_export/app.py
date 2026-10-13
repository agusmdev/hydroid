import io
import tempfile

from fastapi import FastAPI
from fastapi.responses import StreamingResponse

app = FastAPI()

SPOOL_LIMIT = 64 * 1024

ROWS = [{"id": i, "name": f"item-{i}", "notes": "x" * 200} for i in range(500)]


@app.get("/export.csv")
async def export_csv():
    # Spools in memory up to SPOOL_LIMIT, then rolls over to a real temp file on disk.
    spool = tempfile.SpooledTemporaryFile(max_size=SPOOL_LIMIT, mode="w+t", newline="")
    spool.write("id,name,notes\r\n")  # expect: *
    for row in ROWS:
        spool.write(f"{row['id']},{row['name']},{row['notes']}\r\n")  # expect: *
    spool.seek(0)
    body = spool.read()  # expect: *
    spool.close()
    return StreamingResponse(iter([body]), media_type="text/csv")


@app.get("/export-small.csv")
async def export_small():
    # Same idea with an in-memory buffer: never touches the disk.
    buf = io.StringIO()
    for row in ROWS[:5]:
        buf.write(f"{row['id']},{row['name']}\r\n")
    return StreamingResponse(iter([buf.getvalue()]), media_type="text/csv")
