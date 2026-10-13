import contextvars

import requests
from fastapi import FastAPI
from starlette.concurrency import run_in_threadpool

app = FastAPI()

request_id: contextvars.ContextVar[str] = contextvars.ContextVar("request_id", default="-")


def push_metrics(name: str) -> None:
    requests.post("https://metrics.internal/push", json={"rid": request_id.get(), "name": name})


def render_label(name: str) -> str:
    return f"{request_id.get()}:{name}"


@app.get("/report/{name}")
async def report(name: str):
    request_id.set(name)
    ctx = contextvars.copy_context()
    ctx.run(push_metrics, name)  # expect: requests.api.post
    return {"label": ctx.run(render_label, name)}


@app.get("/report-offloaded/{name}")
async def report_offloaded(name: str):
    ctx = contextvars.copy_context()
    await run_in_threadpool(ctx.run, push_metrics, name)
    return {"ok": True}

