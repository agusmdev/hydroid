# Lazy `map()` / generator expression handed to an offloaded consumer

A common way to run a whole batch of sync lookups off the loop without writing a helper is to
offload the *consumer* and pass it a lazy iterable:

```python
a = await asyncio.to_thread(list, map(repo.price, keys))
b = await run_in_threadpool(dict, ((k, repo.price(k)) for k in keys))
c = await asyncio.to_thread(sum, (repo.price(k) for k in keys))
```

Python semantics: `map(f, it)` calls nothing when constructed; `f` runs on each `__next__`. A
generator expression evaluates only its first iterable (`keys`) eagerly; the element expression
`repo.price(k)` runs on each `__next__`. Both iterables are built on the loop, but `list`, `dict`
and `sum` pull the elements, and they are what is offloaded:

- `asyncio.to_thread` (`/usr/lib/python3.13/asyncio/threads.py`):
  `func_call = functools.partial(ctx.run, func, *args, **kwargs)` then
  `await loop.run_in_executor(None, func_call)` - `list(<map>)` runs in the default executor.
- `starlette.concurrency.run_in_threadpool`:
  `func = functools.partial(func, *args, **kwargs); return await anyio.to_thread.run_sync(func)`.

So `repo.price` (and its `time.sleep`) runs only in the worker thread. hydroid reports
`time.sleep` on lines 24-26, apparently treating the arguments of the map / genexp as consumed
where they are built. (The already fixed case is a lazy iterable given to `StreamingResponse`;
this is the same laziness with an offloaded builtin consumer.)

Runtime: the oracle sees lines 24-26 run on the loop with nothing blocking. Near miss: the
eager `list(map(repo.price, keys))` in `/prices/eager` runs `repo.price` on the loop (labeled,
confirmed).
