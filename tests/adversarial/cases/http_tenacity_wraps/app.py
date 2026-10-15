import asyncio
import json

from fastapi import FastAPI
from tenacity import Retrying, stop_after_attempt, wait_exponential

app = FastAPI()

retrying = Retrying(stop=stop_after_attempt(3), wait=wait_exponential(max=2), reraise=True)


def parse_payload(raw: str) -> dict:
    return json.loads(raw)


safe_parse = retrying.wraps(parse_payload)


@app.get("/parse")
async def parse(raw: str):
    a = safe_parse(raw)  # expect: *
    b = await asyncio.to_thread(safe_parse, raw)
    return {"a": a, "b": b}
