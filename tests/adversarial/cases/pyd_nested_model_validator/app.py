import os

from fastapi import FastAPI
from pydantic import BaseModel, field_validator

app = FastAPI()


class Attachment(BaseModel):
    path: str

    @field_validator("path")
    @classmethod
    def must_exist(cls, value: str) -> str:
        os.path.exists(value)
        return value


class Message(BaseModel):
    subject: str
    attachments: list[Attachment] = []


@app.get("/messages")
async def compose(subject: str):
    msg = Message(subject=subject, attachments=[{"path": "/tmp/" + subject}])  # expect: genericpath.exists
    return {"subject": msg.subject, "n": len(msg.attachments)}


@app.get("/messages/plain")
async def plain(subject: str):
    # Near-miss: no attachments given, the default [] is not validated.
    msg = Message(subject=subject)
    return {"subject": msg.subject}


@app.get("/messages/offloaded")
async def offloaded(subject: str):
    import asyncio

    msg = await asyncio.to_thread(Message, subject=subject, attachments=[{"path": subject}])
    return {"subject": msg.subject}
