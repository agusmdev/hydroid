import sqlite3
import time

from fastapi import APIRouter, FastAPI


def warm_cache():
    time.sleep(1)  # expect: time.sleep


def init_db():
    sqlite3.connect("app.db")  # expect: _sqlite3.connect


def flush():
    time.sleep(0.5)  # expect: time.sleep


def close_db():
    time.sleep(0.2)  # expect: time.sleep


app = FastAPI(on_startup=[warm_cache])
app.router.add_event_handler("startup", init_db)
router = APIRouter(on_shutdown=[flush])
router.add_event_handler("shutdown", close_db)
app.include_router(router)
