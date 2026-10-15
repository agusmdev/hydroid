"""Annotations are read through their types, whatever names spell them: class variables are not
model fields (`ClassVar`, also imported under another name), and a variable or parameter
declared as a collection of model instances (`list`, `typing.List`, `Sequence`, a `dict`'s
values, unions, `Optional`) holds instances, which pydantic keeps as they are."""

import os
from collections.abc import Sequence
from typing import ClassVar as Shared
from typing import List, Optional, Union

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


class Mailbox(BaseModel):
    # A class variable, not a field: giving `default=` data validates nothing.
    default: Shared[Attachment] = Attachment.model_construct(path="")
    attachments: list[Attachment] = []
    index: dict[str, Attachment] = {}


def from_list(items: List[Attachment]) -> Mailbox:
    return Mailbox(attachments=items)


def from_sequence(items: Optional[Sequence[Attachment]]) -> Mailbox:
    return Mailbox(attachments=items)


def from_mapping(items: dict[str, Attachment]) -> Mailbox:
    return Mailbox(index=items)


def from_union(items: Union[list[Attachment], tuple[Attachment, ...]]) -> Mailbox:
    return Mailbox(attachments=items)


def from_data(items: list[dict[str, str]]) -> Mailbox:
    return Mailbox(attachments=items)


@app.get("/mailbox")
async def mailbox(path: str):
    Mailbox(default={"path": path})
    from_list([])
    from_sequence(None)
    from_mapping({})
    from_union([])
    from_data([{"path": path}])  # expect: genericpath.exists
    return {}
