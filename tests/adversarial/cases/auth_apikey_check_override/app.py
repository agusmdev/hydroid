import asyncio
import sqlite3

from fastapi import Depends, FastAPI, HTTPException, Security
from fastapi.security import APIKeyHeader

db = sqlite3.connect(":memory:", check_same_thread=False)
db.execute("CREATE TABLE api_keys (key TEXT PRIMARY KEY, owner TEXT)")


def lookup_owner(api_key: str | None) -> str | None:
    row = db.execute("SELECT owner FROM api_keys WHERE key = ?", (api_key,)).fetchone()
    return row[0] if row else None


class DatabaseAPIKeyHeader(APIKeyHeader):
    """API key scheme that validates the key against the api_keys table."""

    def check_api_key(self, api_key: str | None) -> str | None:
        owner = lookup_owner(api_key)  # expect: sqlite3.Connection.execute
        if owner is None and self.auto_error:
            raise HTTPException(status_code=403, detail="Invalid API key")
        return owner


api_key_scheme = DatabaseAPIKeyHeader(name="X-API-Key", auto_error=False)
plain_scheme = APIKeyHeader(name="X-API-Key", auto_error=False)

app = FastAPI()


@app.get("/reports")
async def reports(owner: str | None = Security(api_key_scheme)):
    return {"owner": owner}


@app.get("/reports-offloaded")
async def reports_offloaded(api_key: str | None = Security(plain_scheme)):
    owner = await asyncio.to_thread(lookup_owner, api_key)
    return {"owner": owner}
