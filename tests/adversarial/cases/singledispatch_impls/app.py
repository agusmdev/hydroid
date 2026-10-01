import time
from functools import singledispatch

from fastapi import FastAPI

app = FastAPI()


@singledispatch
def render(value: object) -> str:
    return str(value)


@render.register
def _(value: int) -> str:
    time.sleep(0.1)
    return str(value)


@app.get("/render")
async def render_route(n: int):
    return render(n)  # expect: time.sleep
