import time
from pathlib import Path

from fastapi import FastAPI
from pydantic import BaseModel, field_serializer, model_serializer

app = FastAPI()


class Invoice(BaseModel):
    number: str
    currency: str

    @field_serializer("currency")
    def currency_label(self, value: str) -> str:
        # Looks up a display label from a local file at serialization time.
        return Path("currencies.txt").read_text() or value  # expect: pathlib.Path.read_text


class AuditEntry(BaseModel):
    action: str

    @model_serializer
    def dump(self) -> dict:
        time.sleep(0.01)  # expect: time.sleep
        return {"action": self.action, "ts": time.time()}


class Plain(BaseModel):
    name: str

    @field_serializer("name")
    def upper(self, value: str) -> str:
        return value.upper()


@app.get("/invoice")
async def invoice(number: str):
    # Async endpoint: FastAPI serializes the returned model on the loop.
    return Invoice(number=number, currency="EUR")


@app.get("/audit")
async def audit(action: str):
    return AuditEntry(action=action)


@app.get("/plain")
async def plain(name: str):
    return Plain(name=name)
