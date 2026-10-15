import os
import socket
from pathlib import Path
from typing import Annotated, Optional

from fastapi import FastAPI
from pydantic import BaseModel, PlainSerializer, field_serializer, field_validator, model_serializer

app = FastAPI()


def host_label(value: str) -> str:
    return socket.gethostbyaddr(value)[0]  # expect: _socket.gethostbyaddr via route GET /peers/{addr} (response serialization)


def file_size(name: str) -> int:
    return Path(name).stat().st_size


# --- serializers only used when dumping to JSON -------------------------------------------------

JsonHost = Annotated[str, PlainSerializer(host_label, when_used="json")]
JsonUnlessNoneHost = Annotated[Optional[str], PlainSerializer(host_label, when_used="json-unless-none")]
AnyHost = Annotated[Optional[str], PlainSerializer(host_label, when_used="unless-none")]


class Peer(BaseModel):
    addr: JsonHost
    alt: JsonUnlessNoneHost = None


class Route(BaseModel):
    peer: Peer


class Server(BaseModel):
    name: str

    @field_serializer("name", when_used="json")
    def resolve(self, value: str) -> str:
        return socket.gethostbyaddr(value)[0]


class Cluster(BaseModel):
    name: str

    @model_serializer(when_used="json")
    def resolve(self) -> dict:
        return {"name": socket.gethostbyaddr(self.name)[0]}


class Mirror(BaseModel):
    addr: AnyHost = None


@app.get("/peers")
async def peers(addr: str, mode: str):
    peer = Peer(addr=addr)
    peer.model_dump()
    peer.model_dump(mode="python")
    peer.dict()
    Route(peer=peer).model_dump()
    Server(name=addr).model_dump()
    Cluster(name=addr).model_dump()
    peer.model_dump(mode="json")  # expect: _socket.gethostbyaddr
    peer.model_dump(mode=mode)  # expect: _socket.gethostbyaddr
    peer.model_dump_json()  # expect: _socket.gethostbyaddr
    Route(peer=peer).model_dump(mode="json")  # expect: _socket.gethostbyaddr
    Server(name=addr).model_dump_json()  # expect: _socket.gethostbyaddr
    Cluster(name=addr).model_dump(mode="json")  # expect: _socket.gethostbyaddr
    Mirror(addr=addr).model_dump()  # expect: _socket.gethostbyaddr
    return {}


# FastAPI serializes responses in JSON mode.
@app.get("/peers/{addr}", response_model=Peer)
async def peer_response(addr: str):
    return {"addr": addr}


# --- fields left out of a dump --------------------------------------------------------------


class Document(BaseModel):
    title: str
    blob: Annotated[str, PlainSerializer(file_size)]

    @field_serializer("title")
    def shout(self, value: str) -> str:
        os.path.exists(value)
        return value.upper()


@app.get("/documents")
async def documents(name: str, fields: set[str]):
    doc = Document(title=name, blob=name)
    doc.model_dump(exclude={"blob"})  # expect: genericpath.exists
    doc.model_dump(exclude={"blob": True, "title": True})
    doc.model_dump(include={"title"})  # expect: genericpath.exists
    doc.model_dump(include={"title": True})  # expect: genericpath.exists
    doc.model_dump(exclude={"title"})  # expect: pathlib.Path.stat
    doc.model_dump(include=fields)  # expect: *
    doc.model_dump(exclude=fields)  # expect: *
    return {}


# --- model instances given to nested fields are not validated again ---------------------------


class Attachment(BaseModel):
    path: str

    @field_validator("path")
    @classmethod
    def must_exist(cls, value: str) -> str:
        os.path.exists(value)  # expect: genericpath.exists via route POST /messages (request validation)
        return value


class Message(BaseModel):
    subject: str
    attachments: list[Attachment] = []
    by_name: dict[str, Attachment] = {}


def stored_attachments() -> list[Attachment]:
    return []


@app.post("/messages")
async def compose(subject: str, attachments: list[Attachment]):
    stored = [Attachment.model_construct(path=subject)]
    Message(subject=subject, attachments=stored)
    Message(subject=subject, attachments=attachments)
    copies = [a.model_copy() for a in attachments]
    Message(subject=subject, attachments=copies)
    named: dict[str, Attachment] = {}
    Message(subject=subject, by_name=named)
    raw = [{"path": subject}]
    Message(subject=subject, attachments=raw)  # expect: genericpath.exists
    rows: list[dict] = []
    Message(subject=subject, attachments=rows)  # expect: genericpath.exists
    mixed = stored
    mixed = raw
    Message(subject=subject, attachments=mixed)  # expect: genericpath.exists
    return {}
