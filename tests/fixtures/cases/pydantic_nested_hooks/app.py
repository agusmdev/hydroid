import asyncio
import os
import time
from pathlib import Path
from typing import Any, Optional

from fastapi import FastAPI
from fastapi.responses import JSONResponse, PlainTextResponse, Response
from pydantic import BaseModel, computed_field, field_serializer, field_validator

from labels import Checked, Shouted

app = FastAPI()


# --- constructing a model validates the data given to its fields into nested models ------------


class Attachment(BaseModel):
    path: str

    @field_validator("path")
    @classmethod
    def must_exist(cls, value: str) -> str:
        os.path.exists(value)
        return value


class Envelope(BaseModel):
    attachment: Optional[Attachment] = None


class Message(BaseModel):
    subject: str
    envelope: Envelope | None = None
    attachments: list[Attachment] = []


@app.get("/messages")
async def messages(subject: str):
    Message(subject=subject, envelope={"attachment": {"path": subject}})  # expect: genericpath.exists
    Message.model_validate({"subject": subject})  # expect: genericpath.exists
    Message(**{"subject": subject})  # expect: genericpath.exists
    # Nothing given to validate into `Attachment`.
    Message(subject=subject)
    Message(subject=subject, attachments=[])
    # Model instances are kept as they are.
    existing = Attachment.model_construct(path=subject)
    Message(subject=subject, attachments=[existing])
    await asyncio.to_thread(Message, subject=subject, attachments=[{"path": subject}])
    return {}


# --- `Annotated` validators and serializers on fields, through aliases from another module ------


class Ticket(BaseModel):
    code: Checked
    title: Shouted
    note: str = ""


@app.get("/tickets")
async def tickets(code: str):
    ticket = Ticket(code=code, title="x")  # expect: pathlib.Path.exists
    Ticket.model_construct(code=code, title="x")
    data = ticket.model_dump_json()  # expect: time.sleep
    return {"data": data, "note": ticket.note}


def ticket_dump(ticket: Ticket) -> dict[str, Any]:
    return ticket.model_dump()


@app.get("/tickets/helper")
async def tickets_helper(ticket_id: str):
    ticket = Ticket.model_construct(code=ticket_id, title="x")
    return ticket_dump(ticket)  # expect: time.sleep


# --- `response_model=None`: the return annotation is not a response model ----------------------


class Audited(BaseModel):
    name: str

    @field_validator("name")
    @classmethod
    def audit(cls, value: str) -> str:
        Path("audit.log").write_text(value)
        return value


@app.get("/audited", response_model=None)
async def audited() -> Audited:
    return {"name": "x"}  # type: ignore[return-value]


# --- `response_model_exclude`: excluded fields are not serialized -------------------------------


class Secret(BaseModel):
    name: str
    token: str

    @field_serializer("token")
    def mask(self, value: str) -> str:
        time.sleep(0.01)
        return "***"


class Badge(BaseModel):
    name: str
    token: str

    @field_serializer("token")
    def mask(self, value: str) -> str:
        time.sleep(0.01)  # expect: time.sleep via route GET /badge (response serialization)
        return "***"


@app.get("/secret", response_model=Secret, response_model_exclude={"token"})
async def secret():
    return {"name": "a", "token": "b"}


@app.get("/badge", response_model=Badge, response_model_exclude={"name": True})
async def badge():
    return {"name": "a", "token": "b"}


# --- endpoints returning `Response` instances are sent as they are ------------------------------


class Slow(BaseModel):
    name: str

    @computed_field
    @property
    def label(self) -> str:
        return Path("label.txt").read_text()


class SlowJSON(JSONResponse):
    def render(self, content: Any) -> bytes:
        time.sleep(0.01)
        return super().render(content)


class SlowText(JSONResponse):
    def render(self, content: Any) -> bytes:
        time.sleep(0.01)  # expect: time.sleep via route GET /maybe (response rendering)
        return super().render(content)


@app.get("/raw", response_model=Slow, response_class=SlowJSON)
async def raw(fmt: str):
    if fmt == "text":
        return PlainTextResponse("x")
    elif fmt == "empty":
        return Response(b"")
    return JSONResponse({"name": "x"})


# One branch falls through to serialization.
@app.get("/maybe", response_class=SlowText)
async def maybe(fmt: str):
    if fmt == "text":
        return PlainTextResponse("x")
    return {"name": "x"}
