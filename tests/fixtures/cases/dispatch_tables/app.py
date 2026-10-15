import time

import requests
from fastapi import FastAPI

from handlers import EXPORTERS

app = FastAPI()
session = requests.Session()


def read_file(path: str) -> str:
    with open(path) as f:
        return f.read()


def noop(path: str) -> str:
    return path


VERBS = {"GET": session.get, "POST": session.post}
STEPS = [noop, read_file]
HOOKS = (noop, lambda path: time.sleep(1))


@app.get("/verbs")
async def verbs():
    send = VERBS["GET"]
    send("https://example.com")  # expect: requests.sessions.Session.get
    VERBS["POST"]("https://example.com")  # expect: requests.sessions.Session.post
    return {}


@app.get("/steps")
async def steps():
    STEPS[0]("a")
    STEPS[1]("a")  # expect: builtins.open
    return {}


@app.get("/hooks")
async def hooks(i: int):
    HOOKS[i]("a")  # expect: time.sleep
    return {}


@app.get("/export/{name}")
async def export(name: str):
    EXPORTERS["fast"](name)
    return EXPORTERS["slow"](name)  # expect: time.sleep


@app.get("/threadpool")
def threadpool():
    VERBS["GET"]("https://example.com")
    return {}


# A literal with a spread or starred element does not spell out its elements: ty's targets stand.
BASE = {"read": read_file}
SPREAD = {**BASE, "noop": noop}
STARRED = [*[read_file], noop]


@app.get("/spread/{name}")
async def spread(name: str):
    SPREAD[name]("a")  # expect: builtins.open
    STARRED[0]("a")  # expect: builtins.open
    return {}
