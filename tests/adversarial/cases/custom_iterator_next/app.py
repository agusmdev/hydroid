import socket

from fastapi import FastAPI

app = FastAPI()


class Lines:
    def __init__(self, sock: socket.socket) -> None:
        self.sock = sock

    def __iter__(self):
        return self

    def __next__(self) -> bytes:
        data = self.sock.recv(1024)
        if not data:
            raise StopIteration
        return data


class Counter:
    def __init__(self) -> None:
        self.n = 0

    def __iter__(self):
        return self

    def __next__(self) -> int:
        self.n += 1
        if self.n > 3:
            raise StopIteration
        return self.n


@app.get("/lines")
async def lines():
    out = []
    for line in Lines(socket.socket()):  # expect: _socket.socket.recv
        out.append(line)
    for n in Counter():
        out.append(n)
    return out
