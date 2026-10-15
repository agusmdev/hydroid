# auth_apikey_check_override

`DatabaseAPIKeyHeader` subclasses FastAPI's `APIKeyHeader` and overrides only the sync hook
`check_api_key`, which validates the key against a sqlite table. It does not override `__call__`.

At runtime FastAPI calls the scheme instance (`Security(api_key_scheme)`); its `__call__` is the
library's `async def APIKeyHeader.__call__`, so it runs on the event loop, and that body calls
`self.check_api_key(api_key)` - dispatching to the subclass override. The override therefore runs
on the loop and its `lookup_owner(...)` call executes `sqlite3.Connection.execute` there (the
oracle confirms it at line 20).

Why hydroid likely misses it: the loop entry point is a library `async def` whose body hydroid does
not analyze (no `--follow-libs`), so the virtual call from library code back into a user override
of a template-method hook is invisible. The user class only contains a *sync* method that no user
code calls.

Negative: `/reports-offloaded` uses the stock `APIKeyHeader` and offloads the same lookup with
`asyncio.to_thread`.
