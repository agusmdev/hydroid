import time
from pathlib import Path

from fastapi import FastAPI
from pydantic import BaseModel, computed_field

app = FastAPI()


class Report(BaseModel):
    name: str

    @computed_field
    @property
    def template(self) -> str:
        return Path("template.txt").read_text()  # expect: pathlib.Path.read_text

    @computed_field
    @property
    def checked_at(self) -> float:
        time.sleep(0.01)  # expect: time.sleep
        return time.time()


@app.get("/report")
async def report(name: str):
    # FastAPI serializes the returned model on the loop (async endpoint): computed fields run there.
    return Report(name=name)


@app.get("/plain")
async def plain(name: str):
    return {"name": name.upper()}
