import sqlite3

from fastapi import FastAPI, HTTPException, Request
from starlette.authentication import AuthCredentials, AuthenticationBackend, BaseUser
from starlette.middleware.authentication import AuthenticationMiddleware
from starlette.requests import HTTPConnection

db = sqlite3.connect(":memory:", check_same_thread=False)
db.execute("CREATE TABLE user_roles (user_id TEXT, role TEXT)")
db.execute("INSERT INTO user_roles VALUES ('guest', 'reader')")


class DbUser(BaseUser):
    def __init__(self, user_id: str):
        self.user_id = user_id

    @property
    def is_authenticated(self) -> bool:
        return self.user_id != "guest"

    @property
    def display_name(self) -> str:
        return self.user_id

    def has_role(self, role: str) -> bool:
        row = db.execute(
            "SELECT 1 FROM user_roles WHERE user_id = ? AND role = ?", (self.user_id, role)
        ).fetchone()
        return row is not None


class HeaderBackend(AuthenticationBackend):
    async def authenticate(self, conn: HTTPConnection):
        user_id = conn.headers.get("X-User-Id", "guest")
        return AuthCredentials(["authenticated"]), DbUser(user_id)


app = FastAPI()
app.add_middleware(AuthenticationMiddleware, backend=HeaderBackend())


@app.get("/articles")
async def articles(request: Request):
    if not request.user.has_role("reader"):  # expect: sqlite3.Connection.execute
        raise HTTPException(status_code=403)
    return {"user": request.user.display_name}


@app.get("/articles-sync")
def articles_sync(request: Request):
    if not request.user.has_role("reader"):
        raise HTTPException(status_code=403)
    return {"user": request.user.display_name}
