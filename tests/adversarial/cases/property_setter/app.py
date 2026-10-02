import requests
from fastapi import FastAPI

app = FastAPI()


class Remote:
    def __init__(self) -> None:
        self._value = 0

    @property
    def value(self) -> int:
        return self._value

    @value.setter
    def value(self, new: int) -> None:
        requests.put("https://example.com/value", json=new)
        self._value = new

    @value.deleter
    def value(self) -> None:
        requests.delete("https://example.com/value")


class Local:
    def __init__(self) -> None:
        self._n = 0

    @property
    def n(self) -> int:
        return self._n

    @n.setter
    def n(self, new: int) -> None:
        self._n = new


@app.put("/value")
async def set_value(new: int):
    remote = Remote()
    remote.value = new  # expect: requests.api.put
    del remote.value  # expect: requests.api.delete
    local = Local()
    local.n = new
    return remote.value
