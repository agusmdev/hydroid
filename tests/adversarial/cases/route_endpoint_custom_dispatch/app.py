import time

import requests
from fastapi import FastAPI
from starlette.endpoints import HTTPEndpoint
from starlette.requests import Request
from starlette.responses import JSONResponse, Response


class APIEndpoint(HTTPEndpoint):
    """Base view: turns upstream errors into 502 instead of a 500 page."""

    async def dispatch(self) -> None:
        request = Request(self.scope, receive=self.receive)
        handler = getattr(self, request.method.lower(), self.method_not_allowed)
        try:
            response = handler(request)  # expect: requests.api.get
        except requests.RequestException:
            response = JSONResponse({"error": "upstream"}, status_code=502)
        await response(self.scope, self.receive, self.send)


class PricesView(APIEndpoint):
    def get(self, request: Request) -> Response:
        r = requests.get("http://prices.internal/latest")
        return JSONResponse({"code": r.status_code})


class PingView(HTTPEndpoint):
    def get(self, request: Request) -> Response:
        time.sleep(0.01)
        return JSONResponse({"pong": True})


app = FastAPI()
app.add_route("/prices", PricesView, methods=["GET"])
app.add_route("/ping", PingView, methods=["GET"])
