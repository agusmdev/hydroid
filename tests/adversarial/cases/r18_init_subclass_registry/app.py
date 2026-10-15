import time
from typing import ClassVar

import requests
from fastapi import FastAPI

app = FastAPI()


class Notifier:
    registry: ClassVar[dict[str, type["Notifier"]]] = {}
    channel: ClassVar[str] = ""

    def __init_subclass__(cls, channel: str = "", **kwargs):
        super().__init_subclass__(**kwargs)
        cls.channel = channel
        Notifier.registry[channel] = cls

    def __init__(self, target: str) -> None:
        self.target = target


class WebhookNotifier(Notifier, channel="webhook"):
    def send(self, message: str) -> int:
        return requests.post(self.target, json={"text": message}).status_code


class LogNotifier(Notifier, channel="log"):
    def send(self, message: str) -> int:
        time.sleep(0.05)
        return 0


@app.post("/notify")
async def notify(channel: str, target: str, message: str):
    cls = Notifier.registry.get(channel) or Notifier.registry["log"]
    notifier = cls(target)
    status = notifier.send(message)  # expect: *
    return {"status": status}
