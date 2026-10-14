import contextlib
import time

from fastapi import FastAPI
from fastapi.responses import PlainTextResponse
from starlette.background import BackgroundTask

app = FastAPI()

AUDIT_LOG = "/dev/null"


class AuditSpan:
    """Times a download and appends one line to the audit log when it ends."""

    def __init__(self, resource: str) -> None:
        self.resource = resource
        self.started = 0.0

    def __enter__(self) -> "AuditSpan":
        self.started = time.monotonic()
        return self

    def __exit__(self, *exc) -> None:
        elapsed = time.monotonic() - self.started
        with open(AUDIT_LOG, "a") as fh:
            fh.write(f"{self.resource} {elapsed:.3f}\n")


@app.get("/files/{name}")
async def download(name: str):
    with contextlib.ExitStack() as stack:
        span = stack.enter_context(AuditSpan(name))
        body = f"contents of {span.resource}"
        # Keep the span open until the response is sent: the exits move to a new stack, and
        # this `with` closes an empty one.
        deferred = stack.pop_all()
    # Starlette runs a sync background callable in the threadpool after the response.
    return PlainTextResponse(body, background=BackgroundTask(deferred.close))


@app.get("/files-inline/{name}")
async def download_inline(name: str):
    # Near miss: the span ends when this `with` exits, on the loop.
    with contextlib.ExitStack() as stack:  # expect: *
        span = stack.enter_context(AuditSpan(name))
        body = f"contents of {span.resource}"
    return PlainTextResponse(body)
