import asyncio
import json
from pathlib import Path

from fastapi import FastAPI, WebSocket
from starlette.endpoints import WebSocketEndpoint
from starlette.types import Receive, Scope, Send

app = FastAPI()

BANNED_WORDS = Path("banned_words.json")


def load_banned() -> set[str]:
    try:
        return set(json.loads(BANNED_WORDS.read_text()))
    except (OSError, ValueError, TypeError):
        return set()


class ChatRoom(WebSocketEndpoint):
    encoding = "text"

    def __init__(self, scope: Scope, receive: Receive, send: Send) -> None:
        # Starlette builds the endpoint per connection, on the loop: `await ChatRoom(scope, receive, send)`.
        super().__init__(scope, receive, send)
        self.banned = load_banned()  # expect: pathlib.Path.read_text

    async def on_connect(self, websocket: WebSocket) -> None:
        await websocket.accept()

    async def on_receive(self, websocket: WebSocket, data: str) -> None:
        if not set(data.split()) & self.banned:
            await websocket.send_text(data)


class ModeratedRoom(WebSocketEndpoint):
    encoding = "text"

    async def on_connect(self, websocket: WebSocket) -> None:
        self.banned = await asyncio.to_thread(load_banned)
        await websocket.accept()

    async def on_receive(self, websocket: WebSocket, data: str) -> None:
        if not set(data.split()) & self.banned:
            await websocket.send_text(data)


app.router.add_websocket_route("/ws/chat", ChatRoom)
app.router.add_websocket_route("/ws/moderated", ModeratedRoom)
