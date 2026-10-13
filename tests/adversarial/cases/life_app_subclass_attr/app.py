import asyncio
import json
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI, Request

CONFIG_PATH = Path(__file__).with_name("pricing.json")


class PricingTable:
    def __init__(self, path: Path) -> None:
        self.path = path

    def reload(self) -> dict:
        return json.loads(self.path.read_text())


class ShopApp(FastAPI):
    pricing: PricingTable

    def refresh_pricing(self) -> dict:
        return self.pricing.reload()


@asynccontextmanager
async def lifespan(app: ShopApp):
    app.pricing = PricingTable(CONFIG_PATH)
    yield


app = ShopApp(lifespan=lifespan)


@app.get("/prices")
async def prices(request: Request):
    shop: ShopApp = request.app
    return shop.pricing.reload()  # expect: pathlib.Path.read_text


@app.post("/prices/refresh")
async def refresh(request: Request):
    return request.app.refresh_pricing()  # expect: pathlib.Path.read_text


@app.post("/prices/refresh-offloaded")
async def refresh_offloaded(request: Request):
    return await asyncio.to_thread(request.app.refresh_pricing)
