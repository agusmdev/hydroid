# `iter(obj)` on a class whose `__iter__` is a generator, drained in the threadpool

`OrdersExport.__iter__` is a generator function: it pages through a sync driver
(`_fetch_page`, `time.sleep` standing in for the query). The async endpoint does

```python
rows = iter(OrdersExport(tenant))                    # line 31
first = await asyncio.to_thread(next, rows, None)
async for line in iterate_in_threadpool(rows): ...
```

Runtime semantics:

- `iter(x)` calls `type(x).__iter__(x)`. Because `__iter__` contains `yield`, that call only
  creates a generator object; none of its body (so no `_fetch_page`) runs. Line 31 does no I/O.
- `asyncio.to_thread(next, rows, None)` runs `next(rows)` - the first page query - in the
  default executor (`asyncio/threads.py`: `functools.partial(ctx.run, func, *args, **kwargs)`
  then `loop.run_in_executor(None, func_call)`).
- `starlette.concurrency.iterate_in_threadpool` does `as_iterator = iter(iterator)` (a no-op
  on a generator) and then `yield await anyio.to_thread.run_sync(_next, as_iterator)` for each
  element, with `_next` calling `next(iterator)`: every later page is fetched in a worker
  thread.

hydroid reports `time.sleep` at line 31: it treats `iter(obj)` as running `__iter__`'s body,
although it already knows that calling a generator function runs nothing (calling
`export.__iter__()` directly is the same call).

Runtime: the oracle sees line 31 run on the loop with nothing blocking. Near miss:
`"\n".join(OrdersExport(tenant))` iterates on the loop (line 46) - labeled, confirmed.
