import time
from pathlib import Path
from typing import Annotated

from pydantic import AfterValidator, PlainSerializer


def checked(value: str) -> str:
    Path(value).exists()
    return value


def shouted(value: str) -> str:
    time.sleep(0.01)
    return value.upper()


# Type aliases carrying pydantic hooks, used by models in another module.
Checked = Annotated[str, AfterValidator(checked)]
Shouted = Annotated[str, PlainSerializer(shouted)]
