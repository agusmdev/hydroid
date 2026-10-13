import asyncio
import logging
import logging.config
import tempfile
from pathlib import Path

from fastapi import FastAPI

LOG_DIR = Path(tempfile.gettempdir())

LOGGING = {
    "version": 1,
    "disable_existing_loggers": False,
    "formatters": {
        "plain": {"format": "%(asctime)s %(levelname)s %(name)s %(message)s"},
    },
    "handlers": {
        "audit_file": {
            "class": "logging.handlers.RotatingFileHandler",
            "filename": str(LOG_DIR / "audit.log"),
            "maxBytes": 10_000_000,
            "backupCount": 5,
            "delay": True,
            "formatter": "plain",
        },
        "security_file": {
            "class": "logging.handlers.RotatingFileHandler",
            "filename": str(LOG_DIR / "security.log"),
            "maxBytes": 10_000_000,
            "backupCount": 5,
            "delay": True,
            "formatter": "plain",
        },
    },
    "loggers": {
        "audit": {"handlers": ["audit_file"], "level": "INFO", "propagate": False},
        "security": {"handlers": ["security_file"], "level": "INFO", "propagate": False},
    },
}

audit = logging.getLogger("audit")
security = logging.getLogger("security")


def setup_logging() -> None:
    logging.config.dictConfig(LOGGING)


app = FastAPI(on_startup=[setup_logging])


@app.delete("/users/{user_id}")
async def delete_user(user_id: str):
    audit.info("user %s deleted", user_id)  # expect: builtins.open
    return {"deleted": user_id}


@app.post("/users/{user_id}/lock")
async def lock_user(user_id: str):
    # Offloaded: the file handler runs in a worker thread.
    await asyncio.to_thread(security.info, "user %s locked", user_id)
    return {"locked": user_id}


@app.get("/users/{user_id}")
async def get_user(user_id: str):
    # Below the logger level: no record is created, no handler runs.
    audit.debug("user %s read", user_id)
    return {"id": user_id}
