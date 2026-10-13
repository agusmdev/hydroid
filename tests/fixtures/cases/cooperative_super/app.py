import time

import requests
from fastapi import FastAPI

app = FastAPI()


# --- cooperative mixins: `super()` follows the MRO of the instance's class --------------------


class Store:
    def save(self, item: dict) -> dict:
        time.sleep(0.1)
        return item


class MemoryStore:
    def save(self, item: dict) -> dict:
        return item


class Cache:
    def save(self, item: dict) -> dict:
        # Does not call `super()`: the chain stops here.
        return item


class AuditMixin:
    def save(self, item: dict) -> dict:
        return super().save({**item, "audited": True})  # type: ignore[misc]


class StampMixin:
    def save(self, item: dict) -> dict:
        return super().save({**item, "ts": 0})  # type: ignore[misc]


class ItemStore(AuditMixin, StampMixin, Store):
    async def handle(self, item: dict) -> dict:
        return self.save(item)  # expect: time.sleep


class FakeItemStore(AuditMixin, MemoryStore):
    async def handle(self, item: dict) -> dict:
        return self.save(item)


class CachedItemStore(AuditMixin, Cache, Store):
    pass


class OrderStore(AuditMixin, Store):
    def save(self, item: dict) -> dict:
        # ty resolves this `super()` to `AuditMixin.save`, whose own `super()` reaches `Store`.
        return super().save(item)


class Notifier:
    def notify(self) -> None:
        requests.post("https://example.com")


class QuietMixin:
    def notify(self) -> None:
        # Does not call `super()`: `Notifier.notify` never runs for a `LoudNotifier`.
        return None


class LoudNotifier(QuietMixin, Notifier):
    pass


items = ItemStore()
fakes = FakeItemStore()
cached = CachedItemStore()
orders = OrderStore()
loud = LoudNotifier()


@app.post("/items")
async def create_item(name: str):
    return items.save({"name": name})  # expect: time.sleep


@app.post("/fakes")
async def create_fake(name: str):
    return fakes.save({"name": name})


@app.post("/cached")
async def create_cached(name: str):
    return cached.save({"name": name})


@app.post("/orders")
async def create_order(name: str):
    return orders.save({"name": name})  # expect: time.sleep


@app.post("/notify")
async def notify():
    loud.notify()
