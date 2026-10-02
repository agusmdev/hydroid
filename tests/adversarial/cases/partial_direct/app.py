import functools
import time

import requests
from fastapi import FastAPI

app = FastAPI()


@app.get("/partial")
async def partial_route(url: str):
    functools.partial(time.sleep, 1)()  # expect: time.sleep
    fetch = functools.partial(requests.get, url)
    fetch()  # expect: requests.api.get
    nap = time.sleep
    nap(1)  # expect: time.sleep
    return {}
