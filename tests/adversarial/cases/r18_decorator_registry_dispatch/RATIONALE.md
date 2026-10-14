# Registry dict filled by a parametrized decorator, dispatched by key

`EXPORTERS` starts as an empty annotated dict and is populated at import time by
`@exporter("csv")` / `@exporter("s3")`, whose inner `register` does `EXPORTERS[fmt] = fn`.
The async route looks the handler up (`EXPORTERS[fmt]`) and calls it inline. hydroid resolves
dispatch tables only when they are dict/list/tuple literals holding the callables; here the
callables reach the dict through a subscript store inside the decorator, so the call is
unresolved and the blocking `Path.write_text` (and `requests.put`) is missed.

Runtime: the oracle passes `fmt="csv"` by default (query default), so `export_csv` runs on the
loop and calls `pathlib.Path.write_text` - confirmed. The `-offloaded` route hands the same
handler to `asyncio.to_thread`, a negative.
