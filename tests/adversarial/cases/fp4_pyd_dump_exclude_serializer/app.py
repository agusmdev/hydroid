from pathlib import Path
from typing import Annotated

from fastapi import FastAPI
from pydantic import BaseModel, PlainSerializer

app = FastAPI()

STORAGE = Path("/srv/storage")


def file_size(name: str) -> int:
    return (STORAGE / name).stat().st_size


# Serialized as the file's size on disk.
StoredFile = Annotated[str, PlainSerializer(file_size, return_type=int)]


class Document(BaseModel):
    title: str
    owner: str
    blob: StoredFile


@app.get("/documents/{name}/summary")
async def summary(name: str):
    doc = Document(title=name, owner="me", blob=name)
    # The blob field is excluded: its serializer never runs.
    return doc.model_dump(exclude={"blob"})


@app.get("/documents/{name}/owner")
async def owner(name: str):
    doc = Document(title=name, owner="me", blob=name)
    return doc.model_dump(include={"title", "owner"})


@app.get("/documents/{name}")
async def detail(name: str):
    doc = Document(title=name, owner="me", blob=name)
    return doc.model_dump()  # expect: pathlib.Path.stat
