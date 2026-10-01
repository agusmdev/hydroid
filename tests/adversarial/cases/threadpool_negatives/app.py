import time

import requests
from fastapi import Depends, FastAPI, Request
from fastapi.responses import JSONResponse

app = FastAPI()


class Auth:
    def __call__(self, request: Request) -> str:
        return requests.get("https://auth.example.com").text


def db():
    time.sleep(0.1)
    try:
        yield "db"
    finally:
        time.sleep(0.1)


class Boom(Exception):
    pass


@app.exception_handler(Boom)
def on_boom(request: Request, exc: Boom):
    # Starlette runs sync exception handlers in a threadpool.
    requests.post("https://errors.example.com")
    return JSONResponse({}, status_code=500)


@app.get("/items")
async def items(user: str = Depends(Auth()), conn: str = Depends(db)):
    return {"user": user, "conn": conn}
