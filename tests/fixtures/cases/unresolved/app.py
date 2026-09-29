import time
from collections.abc import Callable

from fastapi import FastAPI

app = FastAPI()
HANDLERS: dict[str, Callable[[], None]] = {}


class Plugin:
    def run(self) -> None:
        pass


def helper(client) -> None:
    client.fetch()  # expect-unresolved


@app.get("/dynamic")
async def dynamic(name: str, callback: Callable[[], None]):
    getattr(Plugin(), name)()  # expect-unresolved
    HANDLERS[name]()  # expect-unresolved
    callback()  # expect-unresolved
    helper(object())
    print(len(name))
    time.sleep(1)  # hydroid: ignore
