import subprocess
import time
from contextlib import asynccontextmanager
from typing import Annotated

import requests
from fastapi import APIRouter, BackgroundTasks, Depends, FastAPI, Request, WebSocket
from fastapi.responses import JSONResponse
from starlette.middleware.base import BaseHTTPMiddleware


@asynccontextmanager
async def lifespan(app: FastAPI):
    subprocess.run(["migrate"])  # expect: subprocess.run via lifecycle lifespan
    yield


app = FastAPI(lifespan=lifespan)
router = APIRouter(prefix="/api")


# --- async helpers: the diagnostic sits where the loop code calls blocking code -------------


async def load_profile(user_id: int) -> dict:
    return requests.get(f"https://example.com/{user_id}").json()  # expect: requests.api.get via route GET /profile/{user_id}


@router.get("/profile/{user_id}")
async def profile(user_id: int):
    return await load_profile(user_id)


# --- dependencies ------------------------------------------------------------------------------


async def async_settings() -> dict:
    time.sleep(0.1)  # expect: time.sleep via dependency
    return {}


def sync_db():
    # Sync dependencies run in the threadpool, even for async endpoints.
    time.sleep(0.1)
    yield None


SettingsDep = Annotated[dict, Depends(async_settings)]


@router.get("/items")
async def items(settings: SettingsDep, db=Depends(sync_db)):
    return settings


async def audit_dependency():
    time.sleep(0.1)  # expect: time.sleep via dependency


@router.get("/sync-endpoint")
def sync_endpoint(audit=Depends(audit_dependency)):
    # The endpoint runs in the threadpool, its async dependency on the loop.
    time.sleep(0.1)
    return {}


# --- middleware, exception handlers, websockets, events, background tasks -------------------


@app.middleware("http")
async def timing(request: Request, call_next):
    time.sleep(0.01)  # expect: time.sleep via middleware http
    return await call_next(request)


class AuthMiddleware(BaseHTTPMiddleware):
    async def dispatch(self, request, call_next):
        requests.post("https://auth.example.com")  # expect: requests.api.post via middleware (dispatch)
        return await call_next(request)


app.add_middleware(AuthMiddleware)


class TeapotError(Exception):
    pass


@app.exception_handler(TeapotError)
async def teapot_handler(request: Request, exc: TeapotError):
    with open("/tmp/errors.log", "a") as log:  # expect: builtins.open via exception handler
        log.write(str(exc))
    return JSONResponse({"teapot": True}, status_code=418)


@app.websocket("/ws")
async def socket(websocket: WebSocket):
    time.sleep(1)  # expect: time.sleep via websocket /ws


@app.on_event("startup")
async def warm_cache():
    requests.get("https://example.com/warm")  # expect: requests.api.get via lifecycle startup


async def notify(email: str):
    requests.post("https://mail.example.com", data=email)  # expect: requests.api.post via background task


def sync_notify(email: str):
    requests.post("https://mail.example.com", data=email)


@router.post("/signup")
async def signup(background_tasks: BackgroundTasks):
    background_tasks.add_task(notify, "a@b.c")
    background_tasks.add_task(sync_notify, "a@b.c")
    return {}


app.include_router(router)
