import functools
import time
from typing import Annotated

import requests
from fastapi import Depends, FastAPI, Header

app = FastAPI()


def audited(fn):
    """Record every dependency resolution in the audit service."""

    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        requests.post("https://audit.example.com/events", json={"dep": fn.__name__})  # expect: requests.api.post
        return fn(*args, **kwargs)

    return wrapper


def throttled(fn):
    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        time.sleep(0.01)
        return fn(*args, **kwargs)

    return wrapper


@audited
async def current_user(authorization: Annotated[str, Header()] = "anon") -> str:
    return authorization


@audited
def current_tenant(x_tenant: Annotated[str, Header()] = "default") -> str:
    # sync dependency: FastAPI runs the wrapper in the threadpool
    return x_tenant


@throttled
def legacy_flags() -> dict:
    return {}


CurrentUser = Annotated[str, Depends(current_user)]


@app.get("/me")
async def me(user: CurrentUser, tenant: Annotated[str, Depends(current_tenant)]):
    return {"user": user, "tenant": tenant}


@app.get("/flags", dependencies=[Depends(legacy_flags)])
async def flags():
    return {}
