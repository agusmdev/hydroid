import json
import time
from typing import Any

from fastapi import FastAPI
from fastapi.responses import JSONResponse

app = FastAPI()


class SignedJSONResponse(JSONResponse):
    def render(self, content: Any) -> bytes:
        # Loads the signing key from disk on every render.
        with open("signing.key") as fh:  # expect: builtins.open
            key = fh.read()
        body = json.dumps({"data": content, "key_id": str(key)[:8]})
        return body.encode()


class ThrottledJSONResponse(JSONResponse):
    def render(self, content: Any) -> bytes:
        time.sleep(0.01)  # expect: time.sleep
        return super().render(content)


class PlainResponse(JSONResponse):
    def render(self, content: Any) -> bytes:
        return json.dumps(content, separators=(",", ":")).encode()


# FastAPI instantiates response_class itself, calling render on the loop for async endpoints.
@app.get("/signed", response_class=SignedJSONResponse)
async def signed(item: str):
    return {"item": item}


@app.get("/throttled", response_class=ThrottledJSONResponse)
async def throttled(item: str):
    return {"item": item}


@app.get("/plain", response_class=PlainResponse)
async def plain(item: str):
    return {"item": item}
