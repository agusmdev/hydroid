import json
import sqlite3
from dataclasses import dataclass, field

from fastapi import FastAPI

app = FastAPI()


@dataclass
class Settings:
    path: str
    data: dict = field(default_factory=dict)

    def __post_init__(self):
        with open(self.path) as f:
            self.data = json.load(f)


@dataclass
class Plain:
    name: str


@dataclass
class Repo:
    dsn: str

    def __post_init__(self):
        self.conn = sqlite3.connect(self.dsn)


@app.get("/settings")
async def settings_route():
    settings = Settings("config.json")  # expect: builtins.open
    repo = Repo(":memory:")  # expect: _sqlite3.connect
    plain = Plain("x")
    return settings.data, repo, plain
