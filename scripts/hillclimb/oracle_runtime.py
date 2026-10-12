#!/usr/bin/env python3
"""Runtime ground truth for an adversarial case: runs the case's FastAPI app for real and records
every blocking call that executes on the event loop thread.

Every function in hydroid's catalog (crates/hydroid_core/catalog.toml) is wrapped before the case
is imported. A wrapped call made on a thread with a running event loop, while code of the case is
on the stack, is recorded at the same location hydroid reports: the call site in the innermost
function that runs on the loop (an `async def`, or a sync callback the loop calls directly, such
as a startup handler). The app then runs through Starlette's TestClient: lifespan, every route
and websocket with sample inputs built from the OpenAPI schema, and shutdown.

On the loop, a recorded call does not do its real work: `time.sleep` returns at once, file calls
run (the case runs in a temporary copy), a lock is taken only if it is free, and every other call
returns a `Stub` (the text "{}" that accepts any use), so the code after it keeps running and its later calls are recorded too
(constructors leave the object uninitialized; lock acquires really run: in a single request the
lock is free, and a later release needs it taken).
Locks, queues and other synchronization calls count only when case code makes them: libraries
take their own guard locks for a moment all the time.
Off the loop, sleeps return at once, and network connections and process starts fail at once: no
case reaches the network or runs a command (multiprocessing pools still start Python workers).

Also records the case lines that executed on the loop thread (sys.monitoring), so a line where
hydroid reports a call can be proven safe: it ran on the loop and nothing blocked.

Runs inside the fixture environment (it imports the case):
  tests/fixtures/.venv/bin/python scripts/hillclimb/oracle_runtime.py CASE_DIR [--cpu]
Prints one JSON object per line: {"kind": "block", ...}, {"kind": "error", ...}, and last
{"kind": "done", "loop_lines": {...}}. Lines are flushed at once, so a caller that kills a hung
run keeps what was seen. scripts/hillclimb/check.py runs this and compares with the labels.
"""

import argparse
import asyncio
import fnmatch
import importlib
import inspect
import io
import json
import os
import shutil
import socket
import sqlite3
import sys
import tempfile
import threading
import tomllib
import types
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CATALOG = ROOT / "crates/hydroid_core/catalog.toml"
ASYNC_FLAGS = inspect.CO_COROUTINE | inspect.CO_ASYNC_GENERATOR | inspect.CO_ITERABLE_COROUTINE

# Libraries the cases may use: imported before any patch, so locks they create at import time stay
# real (only locks the case creates are watched).
PRELOAD = [
    "fastapi", "fastapi.testclient", "starlette", "pydantic", "requests", "httpx", "sqlalchemy",
    "sqlalchemy.orm", "sqlmodel", "redis", "redis.lock", "pymysql", "anyio", "tenacity", "bcrypt",
    "sqlite3", "subprocess", "urllib.request", "http.client", "smtplib", "ftplib", "shutil",
    "tempfile", "concurrent.futures", "multiprocessing.pool", "multiprocessing.queues", "queue",
    "threading", "logging", "json", "csv", "pickle", "glob", "pathlib",
]


def emit(obj: dict) -> None:
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


class Recorder:
    def __init__(self, case_dir: Path) -> None:
        self.case_dir = str(case_dir) + os.sep
        self.local = threading.local()
        self.seen: set[tuple[str, int, str]] = set()

    def in_case(self, code: types.CodeType) -> bool:
        return code.co_filename.startswith(self.case_dir)

    def rel(self, code: types.CodeType) -> str:
        return code.co_filename[len(self.case_dir):].replace(os.sep, "/")

    def on_loop(self) -> bool:
        return asyncio._get_running_loop() is not None

    def location(self, frame: types.FrameType | None) -> tuple[str, int, bool] | None:
        """(path, line, direct) where hydroid reports a call made from `frame`, or None when no
        case code is on the stack. `direct`: the case frame making the call runs on the loop."""
        stack = []  # innermost first
        while frame is not None:
            stack.append(frame)
            frame = frame.f_back
        stack.reverse()  # outermost first
        inner_async = max((i for i, f in enumerate(stack) if f.f_code.co_flags & ASYNC_FLAGS), default=-1)
        case_after = [f for f in stack[inner_async + 1:] if self.in_case(f.f_code)]
        if inner_async >= 0 and self.in_case(stack[inner_async].f_code):
            at = stack[inner_async]
        elif case_after:
            at = case_after[0]  # a sync callback the loop (or a library coroutine) calls
        else:
            before = [f for f in stack[:inner_async] if self.in_case(f.f_code)]
            if not before:
                return None
            at = before[-1]
        innermost_case = next(f for f in reversed(stack) if self.in_case(f.f_code))
        return self.rel(at.f_code), at.f_lineno, innermost_case is at

    def depth(self) -> int:
        return getattr(self.local, "depth", 0)

    def record(self, name: str, category: str, direct_only: bool, caller: types.FrameType | None,
               case_caller_only: bool = False) -> bool:
        """Records a blocking call if it runs on the loop with case code on the stack. Returns
        whether it was recorded (the caller then applies the on-loop policy)."""
        if self.depth() or not self.on_loop():
            return False
        while caller is not None and caller.f_code.co_filename == __file__:
            caller = caller.f_back  # the shims (WatchedLock.__enter__) are not callers
        if (category == "synchronization" or case_caller_only) and (caller is None or not self.in_case(caller.f_code)):
            return False  # a library's own guard lock, or its in-memory file
        loc = self.location(caller)
        if loc is None:
            return False
        path, line, direct = loc
        if direct_only and not direct:
            return False
        key = (path, line, name)
        if key not in self.seen:
            self.seen.add(key)
            emit({"kind": "block", "path": path, "line": line, "name": name, "category": category})
        return True


REC: Recorder  # set in main()

SPAWN = {
    "subprocess.run", "subprocess.call", "subprocess.check_call", "subprocess.check_output",
    "subprocess.getoutput", "subprocess.getstatusoutput", "subprocess.Popen.__init__", "os.system",
    "os.popen", "os.posix_spawn", "os.spawnl", "os.spawnv",
}
NETWORK_CONNECT = {
    "socket.create_connection", "socket.getaddrinfo", "_socket.gethostbyname",
    "_socket.gethostbyname_ex", "_socket.gethostbyaddr", "_socket.socket.connect",
}


class Stub(str):
    """Stands in for the result of a blocking call: the JSON text "{}" that also accepts any
    attribute, call, item, iteration or `with`, so the case code after it keeps running."""

    def __new__(cls, text: str = "{}"):
        return super().__new__(cls, text)

    def __getattr__(self, name):
        if name.startswith("__"):
            raise AttributeError(name)
        return b"{}" if name == "content" else Stub()

    def __call__(self, *args, **kwargs):
        return Stub()

    def __getitem__(self, key):
        return Stub()

    def __iter__(self):
        return iter(())

    def __next__(self):
        raise StopIteration

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return None


STUB_CALLS: dict[str, int] = {}


def stub(name: str):
    """What a blocking call on the loop returns instead of doing its work."""
    if name.endswith(("__init__", "__exit__")):
        return None  # __init__: the object stays uninitialized; __exit__: do not swallow errors
    if name.endswith("__iter__"):
        return iter(())
    STUB_CALLS[name] = STUB_CALLS.get(name, 0) + 1
    # `while data := sock.recv(1024)` must end: past a few calls, an empty (false) result.
    return Stub() if STUB_CALLS[name] <= 3 else Stub("")


def make_wrapper(orig, name: str, category: str, direct_only: bool, case_caller_only: bool = False):
    def policy_on_loop(args, kwargs):
        if category == "sleep":
            return None
        if category == "file" or (category == "synchronization" and name.endswith((".acquire", ".__enter__"))):
            # Files live in the temporary copy, and in a single request a lock is free: doing the
            # real work keeps the state right (a lock released later must have been taken).
            try:
                return orig(*args, **kwargs)
            except Exception:  # noqa: BLE001
                return stub(name)
        return stub(name)

    def wrapper(*args, **kwargs):
        if REC.record(name, category, direct_only, sys._getframe(1), case_caller_only):
            REC.local.depth = REC.depth() + 1
            try:
                return policy_on_loop(args, kwargs)
            finally:
                REC.local.depth -= 1
        if not REC.on_loop() or REC.depth():
            if category == "sleep":
                return None
            local = name == "_socket.socket.connect" and args[0].family == socket.AF_UNIX
            if name in SPAWN or (name in NETWORK_CONNECT and not local):
                raise OSError(f"oracle: {name} is disabled")
        # Not counted as nesting: the loop itself runs inside a wrapped call (Runner.run).
        return orig(*args, **kwargs)

    wrapper.__wrapped__ = orig
    for attr in ("__name__", "__qualname__", "__doc__"):
        try:
            setattr(wrapper, attr, getattr(orig, attr))
        except (AttributeError, TypeError):
            pass
    return wrapper


NOT_REEXPORTS = {"posix", "nt", "_io", "zipimport", "_imp", "marshal"}


# --- resolving catalog names --------------------------------------------------------------------

def import_prefix(parts: list[str]):
    for i in range(len(parts), 0, -1):
        try:
            return importlib.import_module(".".join(parts[:i])), parts[i:]
        except ImportError:
            continue
    return None, parts


def expand(pattern: str) -> list[tuple[str, object, str]]:
    """Catalog name (with `*` globs) -> [(qualname, owner, attribute)] to patch."""
    parts = pattern.split(".")
    module, rest = import_prefix(parts)
    if module is None or not rest:
        return []
    prefix = module.__name__
    owners: list[tuple[str, object]] = [(prefix, module)]
    for i, seg in enumerate(rest):
        last = i == len(rest) - 1
        nxt = []
        for qual, owner in owners:
            names = [n for n in vars(owner) if not n.startswith("__") or n == seg] if "*" in seg else [seg]
            for n in names:
                if not fnmatch.fnmatchcase(n, seg):
                    continue
                if last:
                    nxt.append((f"{qual}.{n}", owner, n))
                else:
                    val = getattr(owner, n, None)
                    if isinstance(val, type) and ("*" not in seg or val.__module__ == prefix):
                        nxt.append((f"{qual}.{n}", val))
        owners = nxt
    out = []
    for qual, owner, attr in owners:
        val = owner.__dict__.get(attr) if hasattr(owner, "__dict__") else None
        if val is None and not isinstance(owner, types.ModuleType):
            continue
        if isinstance(owner, types.ModuleType) and "*" in pattern:
            obj = getattr(owner, attr, None)
            if not callable(obj) or getattr(obj, "__module__", None) != owner.__name__ or isinstance(obj, type):
                continue
        out.append((qual, owner, attr))
    return out


def patch_catalog(cpu: bool) -> list[str]:
    catalog = tomllib.loads(CATALOG.read_text())
    replaced: dict[int, object] = {}
    bound_cls: dict[int, object] = {}
    skipped = []
    # A `blocking_decorator` (tenacity.retry) needs no wrapper: its calls reach Retrying.__call__.
    for group in catalog.get("sink", []):
        if group.get("opt_in") and not cpu:
            continue
        category = group["category"]
        for pattern in group["functions"]:
            targets = expand(pattern)
            if not targets:
                skipped.append(pattern)
            for qual, owner, attr in targets:
                raw = owner.__dict__[attr] if not isinstance(owner, types.ModuleType) else getattr(owner, attr)
                kind = type(raw) if isinstance(raw, (staticmethod, classmethod)) else None
                func = raw.__func__ if kind else raw
                if not callable(func):
                    skipped.append(qual)
                    continue
                # Low-level file calls inside a library are its plumbing (multiprocessing writes a
                # byte to a pipe); its real file work goes through `open`, which counts anywhere.
                low_level = qual.startswith(("os.", "genericpath.", "posixpath."))
                wrapper = make_wrapper(func, qual, category, bool(group.get("direct")), low_level)
                wrapper = kind(wrapper) if kind else wrapper
                try:
                    setattr(owner, attr, wrapper)
                except (TypeError, AttributeError):
                    skipped.append(qual)  # a C type: handled by the shims below where it matters
                    continue
                if isinstance(owner, types.ModuleType):
                    replaced[id(raw)] = (raw, wrapper)
                elif kind is classmethod:  # `open = TarFile.open` re-exports the bound method
                    bound_cls[id(func)] = (func, getattr(owner, attr))
    # Re-exports (`requests.get` is `requests.api.get`, `sqlite3.connect` is `_sqlite3.connect`),
    # but not the import system's own (`posix.stat`): an import is not a call hydroid reports.
    for modname, mod in list(sys.modules.items()):
        if modname in NOT_REEXPORTS or modname.startswith(("importlib", "_frozen_importlib")):
            continue
        d = getattr(mod, "__dict__", None)
        if not isinstance(d, dict):
            continue
        for k, v in list(d.items()):
            hit = replaced.get(id(v)) or bound_cls.get(id(getattr(v, "__func__", None)))
            if hit is not None and (hit[0] is v or hit[0] is getattr(v, "__func__", None)):
                try:
                    d[k] = hit[1]
                except TypeError:
                    pass
    return skipped


# --- shims for C types the catalog names but that cannot be patched -----------------------------

def install_shims(case_dir: str) -> None:
    def from_case(depth: int = 2) -> bool:
        return sys._getframe(depth).f_code.co_filename.startswith(case_dir)

    # socket.socket is a Python subclass of _socket.socket: its methods can be wrapped there.
    for meth in ("connect", "recv", "recv_into", "recvfrom", "send", "sendall", "sendto"):
        orig = getattr(socket.socket, meth)
        setattr(socket.socket, meth, make_wrapper(orig, f"_socket.socket.{meth}", "network", False))

    # queue.SimpleQueue is a C type: a subclass whose get is wrapped.
    import queue

    queue.SimpleQueue = type("SimpleQueue", (queue.SimpleQueue,), {
        "get": make_wrapper(queue.SimpleQueue.get, "_queue.SimpleQueue.get", "synchronization", False)})

    # Locks the case creates: a Python object around the C lock.
    real_lock, real_rlock = threading.Lock, threading.RLock

    class WatchedLock:
        kind = "_thread.lock"

        def __init__(self, inner):
            self._inner = inner
            self.acquire = make_wrapper(inner.acquire, f"{self.kind}.acquire", "synchronization", False)

        def __enter__(self):
            self.acquire()
            return self

        def __exit__(self, *exc):
            self._inner.release()

        def __getattr__(self, name):
            return getattr(self._inner, name)

    class WatchedRLock(WatchedLock):
        kind = "_thread.RLock"

    def lock_factory(*a, **k):
        inner = real_lock(*a, **k)
        return WatchedLock(inner) if from_case() else inner

    def rlock_factory(*a, **k):
        inner = real_rlock(*a, **k)
        return WatchedRLock(inner) if from_case() else inner

    threading.Lock = lock_factory
    threading.RLock = rlock_factory

    # sqlite3 connections and cursors: subclasses with wrapped methods.
    def wrap_methods(cls, prefix, names):
        ns = {n: make_wrapper(getattr(cls, n), f"{prefix}.{n}", "database", False) for n in names}
        return type(cls.__name__, (cls,), ns)

    Cursor = wrap_methods(sqlite3.Cursor, "sqlite3.Cursor", ["execute", "executemany", "fetchone", "fetchall"])
    Conn = wrap_methods(sqlite3.Connection, "sqlite3.Connection", ["execute", "executemany", "executescript", "commit"])
    real_cursor = Conn.cursor
    Conn.cursor = lambda self, factory=Cursor: real_cursor(self, factory)
    connect = sqlite3.connect  # already the catalog wrapper of _sqlite3.connect

    def sqlite_connect(*a, **k):
        k.setdefault("factory", Conn)
        return connect(*a, **k)

    sqlite3.connect = sqlite_connect

    # Files the case opens: a proxy whose reads and writes are `typing.IO.*` calls.
    real_open = __import__("builtins").open  # the catalog wrapper of builtins.open

    class WatchedFile:
        def __init__(self, inner):
            self._inner = inner
            for n in ("read", "readline", "readlines", "write", "writelines", "flush", "truncate"):
                if hasattr(inner, n):
                    setattr(self, n, make_wrapper(getattr(inner, n), f"typing.IO.{n}", "file", False))

        def __enter__(self):
            return self

        def __exit__(self, *exc):
            self._inner.close()

        def __iter__(self):
            return self

        def __next__(self):
            line = self.readline()
            if not line:
                raise StopIteration
            return line

        def __getattr__(self, name):
            return getattr(self._inner, name)

    def watched_open(*a, **k):
        f = real_open(*a, **k)
        return WatchedFile(f) if isinstance(f, io.IOBase) and from_case() else f

    import builtins

    builtins.open = watched_open
    io.open = watched_open

    # UploadFile.file is a SpooledTemporaryFile (a Python class). Only the case's own calls count:
    # Starlette's `await upload.read()` reads a small upload from memory on the loop.
    for n in ("read", "readline", "readlines", "write", "writelines", "flush"):
        orig = getattr(tempfile.SpooledTemporaryFile, n, None)
        if orig is not None:
            setattr(tempfile.SpooledTemporaryFile, n, make_wrapper(orig, f"typing.IO.{n}", "file", False, True))


# --- coverage of case lines on the loop ---------------------------------------------------------

def watch_lines(case_dir: str) -> dict[str, set[int]]:
    lines: dict[str, set[int]] = {}
    mon = sys.monitoring
    tool = mon.COVERAGE_ID

    def on_line(code, line):
        if not code.co_filename.startswith(case_dir):
            return mon.DISABLE
        if asyncio._get_running_loop() is not None:
            lines.setdefault(code.co_filename[len(case_dir):].replace(os.sep, "/"), set()).add(line)
        return None

    mon.use_tool_id(tool, "hydroid-oracle")
    mon.register_callback(tool, mon.events.LINE, on_line)
    mon.set_events(tool, mon.events.LINE)
    return lines


# --- driving the app ----------------------------------------------------------------------------

def sample(schema: dict, components: dict, depth: int = 0):
    if depth > 6:
        return None
    if "$ref" in schema:
        name = schema["$ref"].rsplit("/", 1)[-1]
        return sample(components.get(name, {}), components, depth + 1)
    for key in ("anyOf", "oneOf"):
        if key in schema:
            options = [s for s in schema[key] if s.get("type") != "null"] or schema[key]
            return sample(options[0], components, depth + 1)
    if "allOf" in schema:
        return sample(schema["allOf"][0], components, depth + 1)
    if "enum" in schema:
        return schema["enum"][0]
    if "const" in schema:
        return schema["const"]
    if "default" in schema:
        return schema["default"]
    kind = schema.get("type")
    fmt = schema.get("format", "")
    if kind == "integer":
        return max(1, schema.get("minimum", 1))
    if kind == "number":
        return max(1.0, schema.get("minimum", 1.0))
    if kind == "boolean":
        return True
    if kind == "array":
        return [sample(schema.get("items", {}), components, depth + 1)]
    if kind == "object" or "properties" in schema:
        props = schema.get("properties", {})
        return {k: sample(v, components, depth + 1) for k, v in props.items()}
    if fmt == "binary" or "contentMediaType" in schema:
        return b"data"
    return {"date-time": "2024-01-01T00:00:00", "date": "2024-01-01", "email": "a@example.com",
            "uuid": "12345678-1234-5678-1234-567812345678", "uri": "https://example.com"}.get(fmt, "1")


def fill_params(path: str) -> str:
    while "{" in path:
        head, rest = path.split("{", 1)
        path = head + "1" + rest.split("}", 1)[1]
    return path


def requests_for(app) -> list[dict]:
    try:
        spec = app.openapi()
    except Exception as e:  # noqa: BLE001
        emit({"kind": "error", "where": "openapi", "error": failure(e)})
        spec = {"paths": {}}
    components = spec.get("components", {}).get("schemas", {})
    out = []
    seen = set()
    for path, ops in spec.get("paths", {}).items():
        for method, op in ops.items():
            params = op.get("parameters", [])
            url = path
            query, headers, cookies = {}, {}, {}
            for p in params:
                val = sample(p.get("schema", {}), components)
                if p["in"] == "path":
                    url = url.replace("{" + p["name"] + "}", str(val))
                elif p["in"] == "query":
                    query[p["name"]] = val
                elif p["in"] == "header":
                    headers[p["name"]] = str(val)
                elif p["in"] == "cookie":
                    cookies[p["name"]] = str(val)
            req = {"method": method.upper(), "url": url, "params": query, "headers": headers, "cookies": cookies}
            content = op.get("requestBody", {}).get("content", {})
            if "application/json" in content:
                req["json"] = sample(content["application/json"].get("schema", {}), components)
            elif "multipart/form-data" in content:
                body = sample(content["multipart/form-data"].get("schema", {}), components) or {}
                req["files"] = {k: ("f.txt", v) for k, v in body.items() if isinstance(v, bytes)}
                req["data"] = {k: v for k, v in body.items() if not isinstance(v, bytes)}
            elif "application/x-www-form-urlencoded" in content:
                req["data"] = sample(content["application/x-www-form-urlencoded"].get("schema", {}), components)
            out.append(req)
            seen.add((req["method"], path))
    # Routes the schema leaves out (include_in_schema=False, plain Starlette routes, mounts).
    for route in getattr(app, "routes", []):
        path = getattr(route, "path", None)
        methods = getattr(route, "methods", None) or set()
        if path is None or getattr(route, "endpoint", None) is None:
            continue
        url = fill_params(path)
        for m in methods:
            if (m, path) not in seen and m != "HEAD":
                out.append({"method": m, "url": url, "params": {}, "headers": {}, "cookies": {}})
    return out


def websockets_for(app) -> list[str]:
    from starlette.routing import WebSocketRoute

    out = []
    for route in getattr(app, "routes", []):
        if isinstance(route, WebSocketRoute) or type(route).__name__ == "APIWebSocketRoute":
            out.append(fill_params(route.path))
    return out


def find_app(module):
    from starlette.applications import Starlette

    apps = [v for v in vars(module).values() if isinstance(v, Starlette)]
    return apps[0] if apps else None


def failure(e: BaseException) -> str:
    """The exception, and the innermost case line it passed through."""
    at = ""
    tb = e.__traceback__
    while tb is not None:
        if REC.in_case(tb.tb_frame.f_code):
            at = f" at {REC.rel(tb.tb_frame.f_code)}:{tb.tb_lineno}"
        tb = tb.tb_next
    return repr(e)[:500] + at


def call_handlers(handlers, where: str) -> None:
    async def run():
        for h in handlers:
            try:
                r = h()
                if inspect.isawaitable(r):
                    await r
            except Exception as e:  # noqa: BLE001
                emit({"kind": "error", "where": where, "error": failure(e)})

    asyncio.run(run())


def drive(app) -> None:
    from fastapi.testclient import TestClient

    def run_all(client):
        for req in requests_for(app):
            kw = {k: v for k, v in req.items() if k not in ("method", "url") and v not in (None, {})}
            try:
                client.request(req["method"], req["url"], **kw)
            except Exception as e:  # noqa: BLE001
                emit({"kind": "error", "where": f"{req['method']} {req['url']}", "error": failure(e)})
        for url in websockets_for(app):
            try:
                with client.websocket_connect(url) as ws:
                    ws.send_text("1")
            except Exception as e:  # noqa: BLE001
                emit({"kind": "error", "where": f"websocket {url}", "error": failure(e)})

    client = TestClient(app)
    try:
        with client:
            run_all(client)
        return
    except Exception as e:  # noqa: BLE001
        emit({"kind": "error", "where": "lifespan", "error": failure(e)})
    # The lifespan failed (often a BlockedCall in a startup handler): run each event handler alone
    # so the later ones still run, and the routes without the lifespan.
    router = getattr(app, "router", None)
    call_handlers(getattr(router, "on_startup", []), "startup")
    run_all(TestClient(app))
    call_handlers(getattr(router, "on_shutdown", []), "shutdown")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("case", type=Path)
    parser.add_argument("--cpu", action="store_true", help="also record opt-in CPU-bound sinks")
    args = parser.parse_args()

    work = Path(tempfile.mkdtemp(prefix="hydroid-oracle-"))
    case_dir = (work / "case").resolve()
    shutil.copytree(args.case, case_dir, ignore=shutil.ignore_patterns("RATIONALE.md", "__pycache__"))
    os.chdir(case_dir)
    socket.setdefaulttimeout(1)

    for name in PRELOAD:
        try:
            importlib.import_module(name)
        except ImportError:
            pass
    global REC
    REC = Recorder(case_dir)
    skipped = patch_catalog(args.cpu)
    install_shims(str(case_dir) + os.sep)
    loop_lines = watch_lines(str(case_dir) + os.sep)

    sys.path.insert(0, str(case_dir))
    try:
        module = importlib.import_module("app")
    except Exception as e:  # noqa: BLE001
        emit({"kind": "error", "where": "import", "error": failure(e)})
        module = None
    app = find_app(module) if module else None
    if app is None and module is not None:
        emit({"kind": "error", "where": "import", "error": "no Starlette/FastAPI app in app.py"})
    if app is not None:
        drive(app)
    emit({"kind": "done", "loop_lines": {k: sorted(v) for k, v in loop_lines.items()},
          "unpatched": skipped})
    shutil.rmtree(work, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
