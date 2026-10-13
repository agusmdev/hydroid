# Library APIs ty cannot see through: library overrides of blocking methods, `with` targets of
# unannotated `__enter__`s, and decorators applied by hand.
import asyncio
import json

import pymysql
from fastapi import FastAPI
from sqlmodel import Session, create_engine, select
from tenacity import Retrying, stop_after_attempt

app = FastAPI()
engine = create_engine("sqlite://")
db = pymysql.connect(host="mysql", defer_connect=True)
retrying = Retrying(stop=stop_after_attempt(3))


def parse(raw: str) -> dict:
    return json.loads(raw)


safe_parse = retrying.wraps(parse)


class Scope:
    """A project context manager whose `__enter__` hands out something else than itself."""

    def __enter__(self):
        return json.loads("{}")

    def __exit__(self, *exc_info):
        return None

    def execute(self):
        open("/tmp/scope").close()


def count(session: Session) -> int:
    return len(session.execute(select(1)).all())


@app.get("/sqlmodel")
async def sqlmodel_execute(raw: str):
    session = Session(engine)
    # `sqlmodel.Session.execute` overrides SQLAlchemy's and delegates to it.
    rows = session.execute(select(1)).all()  # expect: sqlalchemy.orm.session.Session.execute
    total = await asyncio.to_thread(count, session)
    return {"rows": len(rows), "total": total}


@app.get("/pymysql")
async def pymysql_cursor():
    # `Cursor.__enter__` is unannotated and returns `self`.
    with db.cursor() as cur:
        cur.execute("SELECT 1")  # expect: pymysql.cursors.Cursor.execute
        row = cur.fetchone()  # expect: pymysql.cursors.Cursor.fetchone
    with Scope() as scope:
        scope.execute()
    return row


@app.get("/tenacity")
async def tenacity_wraps(raw: str):
    a = safe_parse(raw)  # expect: tenacity.BaseRetrying.wraps
    b = retrying.wraps(parse)(raw)  # expect: tenacity.BaseRetrying.wraps
    # Applying the decorator does not run it; neither does calling the original.
    wrapped = retrying.wraps(parse)
    c = parse(raw)
    d = await asyncio.to_thread(safe_parse, raw)
    e = await asyncio.to_thread(wrapped, raw)
    return [a, b, c, d, e]


@app.get("/sync")
def sync_endpoint(raw: str):
    with db.cursor() as cur:
        cur.execute("SELECT 1")
    return safe_parse(raw), Session(engine).execute(select(1)).all()
