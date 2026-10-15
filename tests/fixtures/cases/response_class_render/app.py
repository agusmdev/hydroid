import json
import time
from pathlib import Path
from typing import Any

from fastapi import APIRouter, FastAPI
from fastapi.responses import JSONResponse, Response


class AuditedResponse(Response):
    def render(self, content: Any) -> bytes:
        Path("audit.log").write_text(str(content))  # expect: pathlib.Path.write_text via route GET /audited (response rendering)
        return super().render(content)


class SignedResponse(JSONResponse):
    def __init__(self, content: Any, status_code: int = 200) -> None:
        time.sleep(0.01)  # expect: time.sleep via route POST /signed (response rendering)
        super().__init__(content, status_code=status_code)


class InheritedRender(AuditedResponse):
    media_type = "text/plain"


class AppDefault(JSONResponse):
    def render(self, content: Any) -> bytes:
        time.sleep(0.01)  # expect: time.sleep via default response class (response rendering)
        return json.dumps(content).encode()


class RouterDefault(JSONResponse):
    def init_headers(self, headers: Any = None) -> None:
        time.sleep(0.01)  # expect: time.sleep via default response class (response rendering)
        super().init_headers(headers)


class Unused(JSONResponse):
    # Never handed to FastAPI: nothing renders it on the loop.
    def render(self, content: Any) -> bytes:
        time.sleep(0.01)
        return b""


class Compact(JSONResponse):
    def render(self, content: Any) -> bytes:
        return json.dumps(content, separators=(",", ":")).encode()


app = FastAPI(default_response_class=AppDefault)
router = APIRouter(default_response_class=RouterDefault)


# FastAPI constructs the response class on the loop for sync endpoints too.
@app.get("/audited", response_class=AuditedResponse)
def audited():
    return {"ok": True}


@app.post("/signed", response_class=SignedResponse)
async def signed():
    return {"ok": True}


@app.get("/inherited", response_class=InheritedRender)
async def inherited():
    return "ok"


@app.get("/compact", response_class=Compact)
async def compact():
    return {"ok": True}


@router.get("/routed")
async def routed():
    return {"ok": True}


app.include_router(router)
