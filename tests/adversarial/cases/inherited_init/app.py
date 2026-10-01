import sqlite3

from fastapi import FastAPI

app = FastAPI()


class Base:
    def __init__(self) -> None:
        self.conn = sqlite3.connect("app.db")


class Child(Base):
    pass


class GrandChild(Child):
    def __init__(self) -> None:
        super().__init__()


class Slow:
    def __new__(cls):
        sqlite3.connect("x.db")
        return super().__new__(cls)


@app.get("/init")
async def init_route():
    Child()  # expect: _sqlite3.connect
    GrandChild()  # expect: _sqlite3.connect
    Slow()  # expect: _sqlite3.connect
    return {}
