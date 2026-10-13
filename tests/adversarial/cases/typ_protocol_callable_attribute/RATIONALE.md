# Protocol data member typed `Callable`, bound to `time.sleep` by the implementation

`Backoff` is a `typing.Protocol` declaring a data member `sleep: Callable[[float], None]` plus a
`delays()` method. `ExponentialBackoff` assigns `self.sleep = time.sleep` in `__init__`;
`NoBackoff` uses a no-op. The dependency `get_backoff() -> Backoff` returns
`ExponentialBackoff()`, and the async route calls `backoff.sleep(delay)`.

Likely hydroid error: Protocol *methods* are resolved structurally to the implementing classes,
but a Protocol *attribute* with a `Callable` annotation is resolved only from its declared type,
which names no function, so the call to `time.sleep` through the implementation's instance
attribute is missed. (`ExponentialBackoff().sleep(...)` on the concrete type is found.)

Runtime: `/jobs/1/retry` calls `time.sleep` on the loop at line 44 (confirmed). Negatives:
`/retry-fast` uses `NoBackoff` (no-op lambda), `/retry-offloaded` passes `backoff.sleep` to
`asyncio.to_thread`.
