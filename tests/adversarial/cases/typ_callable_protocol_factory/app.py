import os
from typing import Protocol

import requests
from fastapi import Depends, FastAPI

app = FastAPI()

RATES_URL = os.environ.get("RATES_URL", "https://rates.internal/latest")


class RateFetcher(Protocol):
    def __call__(self, currency: str) -> float: ...


def fetch_live_rate(currency: str) -> float:
    resp = requests.get(RATES_URL, params={"currency": currency}, timeout=5)
    resp.raise_for_status()
    return resp.json()["rate"]


def fixed_rate(currency: str) -> float:
    return 1.0


def make_rate_fetcher() -> RateFetcher:
    if os.environ.get("RATES_MODE") == "fixed":
        return fixed_rate
    return fetch_live_rate


get_rate: RateFetcher = make_rate_fetcher()


@app.get("/convert/{currency}")
async def convert(currency: str, amount: float = 1.0):
    rate = get_rate(currency)  # expect: requests.api.get
    return {"amount": amount, "rate": rate}


@app.get("/quote/{currency}")
async def quote(currency: str, fetcher: RateFetcher = Depends(make_rate_fetcher)):
    return {"rate": fetcher(currency)}  # expect: requests.api.get


@app.get("/convert/{currency}/sync")
def convert_sync(currency: str):
    try:
        rate = get_rate(currency)
    except requests.RequestException:
        rate = 1.0
    return {"rate": rate}
