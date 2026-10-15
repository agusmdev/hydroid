import sqlite3

from fastapi import FastAPI

app = FastAPI()


class Repository:
    def save(self, item: dict) -> dict:
        conn = sqlite3.connect("app.db")
        conn.execute("INSERT INTO items (name) VALUES (?)", (item["name"],))
        return item


class MemoryRepository:
    def save(self, item: dict) -> dict:
        return item


class AuditMixin:
    def save(self, item: dict) -> dict:
        item = {**item, "audited": True}
        return super().save(item)  # type: ignore[misc]


class TimestampMixin:
    def save(self, item: dict) -> dict:
        item = {**item, "ts": 0}
        return super().save(item)  # type: ignore[misc]


class ItemRepository(AuditMixin, TimestampMixin, Repository):
    pass


class FakeItemRepository(AuditMixin, MemoryRepository):
    pass


repo = ItemRepository()
fake = FakeItemRepository()


@app.post("/items")
async def create_item(name: str):
    return repo.save({"name": name})  # expect: _sqlite3.connect


@app.post("/fake-items")
async def create_fake_item(name: str):
    return fake.save({"name": name})
