import asyncio
import sqlite3
from contextlib import asynccontextmanager

from fastapi import FastAPI, Request


class SessionStore:
    def __init__(self, path: str):
        self.conn = sqlite3.connect(path, check_same_thread=False)
        self.conn.execute("CREATE TABLE IF NOT EXISTS sessions (sid TEXT PRIMARY KEY, user TEXT)")

    def lookup(self, sid: str):
        row = self.conn.execute("SELECT user FROM sessions WHERE sid = ?", (sid,)).fetchone()
        return row[0] if row else None


@asynccontextmanager
async def lifespan(app: FastAPI):
    app.state.sessions = await asyncio.to_thread(SessionStore, ":memory:")
    yield


app = FastAPI(lifespan=lifespan)


@app.get("/me")
async def me(sid: str, request: Request):
    user = request.app.state.sessions.lookup(sid)  # expect: sqlite3.Connection.execute
    return {"user": user}


@app.get("/me/sync")
def me_sync(sid: str, request: Request):
    return {"user": request.app.state.sessions.lookup(sid)}
