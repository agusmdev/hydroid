import time
from typing import Callable


def slow_export(name: str) -> str:
    time.sleep(0.1)
    return name


def fast_export(name: str) -> str:
    return name


# Imported by the app: the table lives in another module.
EXPORTERS: dict[str, Callable[[str], str]] = {"slow": slow_export, "fast": fast_export}
