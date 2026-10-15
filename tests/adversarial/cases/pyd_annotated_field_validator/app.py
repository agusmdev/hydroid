import os
from typing import Annotated

from fastapi import FastAPI
from pydantic import AfterValidator, BaseModel

app = FastAPI()


def file_must_exist(value: str) -> str:
    if not os.path.isfile(value):
        return value
    return value


ExistingPath = Annotated[str, AfterValidator(file_must_exist)]


class ImportJob(BaseModel):
    source: ExistingPath
    label: str


@app.get("/imports")
async def start_import(label: str):
    job = ImportJob(source="/data/" + label, label=label)  # expect: genericpath.isfile
    return {"label": job.label}


@app.get("/imports/trusted")
async def trusted_import(label: str):
    # Near-miss: model_construct skips validation, so the validator never runs.
    job = ImportJob.model_construct(source="/data/" + label, label=label)
    return {"label": job.label}


@app.get("/imports/sync")
def sync_import(label: str):
    # Near-miss: def endpoint body runs in the threadpool.
    job = ImportJob(source="/data/" + label, label=label)
    return {"label": job.label}
