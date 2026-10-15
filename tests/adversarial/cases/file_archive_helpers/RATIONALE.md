# file_archive_helpers

Stdlib file helpers that are not themselves in the catalog but open and walk files internally:

- `shutil.make_archive(base, "zip", root)` walks `root`, creates a `zipfile.ZipFile`, opens every
  file and copies it with `shutil.copyfileobj` - a whole backup on the event loop.
- `zipfile.is_zipfile(path)` opens the path with `open()` and reads the end-of-central-directory.

hydroid only knows the catalog entry points (`zipfile.ZipFile.__init__`, `shutil.copy*`, ...) and
does not look inside stdlib bodies, so both calls in `async def` endpoints are missed.

Runtime: the oracle sees `zipfile.ZipFile.__init__`, `builtins.open` and `shutil.copyfileobj`
from line 22 and `builtins.open` from line 36, on the loop. Near-misses: `asyncio.to_thread(
shutil.make_archive, ...)` and a sync `def` endpoint calling `is_zipfile` (threadpool).
(The `/backups/verify` endpoint then raises on the oracle's stand-in value, after the labeled line
ran.)
