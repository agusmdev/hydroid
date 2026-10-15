# Pydantic `Field(default_factory=...)` reading a secrets file

A settings model built by hand (no pydantic-settings) fills `api_key` with
`Field(default_factory=read_api_key)`, which reads `/run/secrets/api_key` with
`Path.read_text`. Pydantic calls the factory inside `BaseModel.__init__` whenever the field is
not given, so `Settings()` in an `async def` endpoint reads the file on the loop.

hydroid models validators and `__post_init__` on construction, but apparently not default
factories stored in `Field(...)` metadata, so it misses the call.

Negatives: a model with no factory, `asyncio.to_thread(Settings)`, and `Settings(api_key=...)`
(the factory is not called when the value is passed - the runtime confirms nothing blocks).
