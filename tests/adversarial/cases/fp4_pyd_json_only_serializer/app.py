import socket
from typing import Annotated

from fastapi import FastAPI
from pydantic import BaseModel, PlainSerializer

app = FastAPI()


def host_label(value: str) -> str:
    # Pretty-print an address as its hostname for JSON exports.
    try:
        return socket.gethostbyaddr(value)[0]
    except OSError:
        return value


# Only applied when serializing to JSON (model_dump_json / model_dump(mode="json")).
HostAddr = Annotated[str, PlainSerializer(host_label, return_type=str, when_used="json")]


class Peer(BaseModel):
    addr: HostAddr
    port: int


@app.get("/peers/raw")
async def peer_raw(addr: str, port: int):
    peer = Peer(addr=addr, port=port)
    # Python-mode dump: the json-only serializer is skipped, addr stays as given.
    data = peer.model_dump()
    return {"peer": data}


@app.get("/peers/export")
async def peer_export(addr: str, port: int):
    peer = Peer(addr=addr, port=port)
    return {"peer": peer.model_dump_json()}  # expect: _socket.gethostbyaddr


@app.get("/peers/sync")
def peer_sync(addr: str, port: int):
    return {"peer": Peer(addr=addr, port=port).model_dump(mode="json")}
