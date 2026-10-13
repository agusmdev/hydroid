import asyncio
import json
import time
from pathlib import Path

import anyio.from_thread
from fastapi import BackgroundTasks, FastAPI

app = FastAPI()
FLAGS_FILE = Path("/tmp/flags.json")


class FeatureFlags:
    """Owned by the event loop: only touched from loop code."""

    def __init__(self) -> None:
        self.flags: dict[str, bool] = {}
        self.changed = asyncio.Event()

    def reload(self) -> None:
        FLAGS_FILE.write_text(json.dumps(self.flags))  # expect: pathlib.Path.write_text
        self.changed.set()

    def set_flag(self, name: str, value: bool) -> None:
        self.flags[name] = value
        time.sleep(0.01)  # expect: time.sleep
        self.changed.set()

    async def persist(self) -> None:
        await asyncio.to_thread(self._write)

    def _write(self) -> None:
        FLAGS_FILE.write_text(json.dumps(self.flags))


flags = FeatureFlags()


def legacy_sync_export() -> str:
    # Sync endpoint body: FastAPI runs it in a worker thread.
    time.sleep(0.01)
    return "ok"


def apply_flags_job(name: str) -> None:
    # Sync background task: Starlette runs it in a worker thread...
    time.sleep(0.01)
    # ...but this hops back onto the event loop to touch loop-owned state.
    anyio.from_thread.run_sync(flags.set_flag, name, True)


@app.post("/admin/flags/reload")
def reload_flags():
    legacy_sync_export()
    anyio.from_thread.run_sync(flags.reload)
    anyio.from_thread.run(flags.persist)
    return {"reloaded": True}


@app.post("/admin/flags/{name}")
async def enable_flag(name: str, tasks: BackgroundTasks):
    tasks.add_task(apply_flags_job, name)
    return {"queued": name}
