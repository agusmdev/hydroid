# ws_endpoint_class_init - miss

A Starlette class-based websocket endpoint (`WebSocketEndpoint` subclass registered with
`app.router.add_websocket_route`) is an ASGI class. For each connection Starlette builds the
class and awaits it (`ChatRoom(scope, receive, send)`, then `__await__` -> `dispatch`,
tests/fixtures/.venv/lib/python3.13/site-packages/starlette/endpoints.py:62-72). The constructor is a plain `def` called on the loop thread. Loading the
moderation word list from disk there (`Path.read_text`) blocks the loop on every connection.

hydroid sees the async `on_connect`/`on_receive` methods as loop code, but it misses the
sync `__init__` override, which Starlette also runs on the loop.

Near miss: `ModeratedRoom` loads the list in `on_connect` through
`await asyncio.to_thread(load_banned)`. No label.

Runtime: the oracle connects to `/ws/chat` and records `pathlib.Path.read_text` on the loop at
line 27. That is the call inside the sync `__init__` the loop runs, and the label sits there.
