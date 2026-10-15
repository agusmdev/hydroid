import time
from typing import Annotated

from fastapi import FastAPI
from pydantic import BaseModel, PlainSerializer

app = FastAPI()


def slow_label(value: str) -> str:
    time.sleep(0.01)
    return value.upper()


Label = Annotated[str, PlainSerializer(slow_label)]


class Report(BaseModel):
    title: Label


@app.get("/report")
async def report(title: str):
    r = Report(title=title)
    data = r.model_dump()  # expect: time.sleep
    return {"data": data}


@app.get("/report/title")
async def report_title(title: str):
    # Near-miss: attribute access does not serialize.
    r = Report(title=title)
    return {"title": r.title}


@app.get("/report/sync")
def report_sync(title: str):
    return {"data": Report(title=title).model_dump()}
