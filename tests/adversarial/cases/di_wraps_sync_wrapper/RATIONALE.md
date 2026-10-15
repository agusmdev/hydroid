# Sync `functools.wraps` wrapper around an async dependency runs on the loop

FastAPI decides whether a dependency is awaited or sent to the threadpool by looking at the
*unwrapped* callable (`inspect.unwrap`, which follows `__wrapped__` set by `functools.wraps`).
`@audited async def current_user` is replaced by a plain `def wrapper`, but `inspect.unwrap`
returns the original `async def`, so FastAPI classifies it as a coroutine callable and does
`await wrapper(**values)` directly on the event loop. The sync wrapper body - here an audit
`requests.post` - therefore runs on the loop before it hands back the coroutine.

Likely hydroid miss: `Depends(current_user)` resolves to a sync `def wrapper`, and sync
dependencies are treated as threadpool code.

Negatives: the same decorator on a sync dependency (`current_tenant`) and `throttled` on a sync
dependency used in `dependencies=[...]`: their unwrapped target is sync, so FastAPI runs the
wrapper in the threadpool (the runtime oracle sees nothing on the loop there).
