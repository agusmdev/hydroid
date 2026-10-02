# Using hydroid

hydroid finds code that blocks the asyncio event loop in FastAPI apps: a `time.sleep`, a
`requests.get`, a sync database query… anywhere below an `async def`, however deep.

## 1. Install

You need [uv](https://docs.astral.sh/uv/getting-started/installation/).

**Prebuilt binaries** (macOS, Linux, Windows; no Rust needed), from the GitHub release:

```sh
uvx --from hydroid-cli --find-links https://github.com/agusmdev/hydroid/releases/expanded_assets/v0.1.0 hydroid .
```

Install the `hydroid` command permanently the same way:

```sh
uv tool install hydroid-cli --find-links https://github.com/agusmdev/hydroid/releases/expanded_assets/v0.1.0
hydroid .
```

**From source** (latest `main`; needs a Rust toolchain, `curl https://sh.rustup.rs -sSf | sh`;
the first build takes about a minute, then it is cached):

```sh
uvx --from git+https://github.com/agusmdev/hydroid hydroid .
uv tool install git+https://github.com/agusmdev/hydroid      # permanent
```

Pin a version with `git+https://github.com/agusmdev/hydroid@v0.1.0`. From a clone,
`cargo install --path crates/hydroid` also works.

## 2. Prepare the project to check

hydroid resolves imports against the project's installed dependencies, like a type checker. With uv:

```sh
cd my-fastapi-app
uv sync            # creates .venv with your dependencies
```

hydroid finds the environment the same way ty does: `VIRTUAL_ENV`, an active Conda env,
`<project>/.venv`, then `python3` on `PATH`. Point it elsewhere with `--python path/to/.venv`.

## 3. Run it

```sh
hydroid .                      # the project root (where your package lives)
hydroid backend --python .venv # project in a subdirectory, venv elsewhere
```

Exit status: `0` nothing found, `1` blocking calls found, `2` hydroid could not run.

## 4. Read a finding

```
error[blocking-http]: `requests.api.get` blocks the event loop
  --> app/main.py:12:12 in `app.main.get_user`
   = reached from route GET /users/{user_id}: app.main.get_user (app/main.py:11)
   = blocking chain (3 calls):
       app/main.py:12:12  services.load -> app.services.load
       app/services.py:7:22  fetch -> app.repo.fetch
       app/repo.py:5:16  http_get -> requests.api.get
   = help: use an async client (`httpx.AsyncClient`, `aiohttp`) or `await asyncio.to_thread(...)`
```

- `-->` is **where to fix it**: the call inside code running on the event loop (`async def`, or a
  sync callback the loop runs).
- `reached from` is the FastAPI entry point (route, dependency, middleware, …) that gets there.
- `blocking chain` walks from that call down to the function that blocks.
- `help` suggests the non-blocking alternative.

Typical fixes:

```python
# before
async def get_user(user_id: int):
    return services.load(user_id)

# 1. move the sync work to a thread
async def get_user(user_id: int):
    return await asyncio.to_thread(services.load, user_id)

# 2. or make the endpoint sync: FastAPI runs `def` endpoints in a threadpool
def get_user(user_id: int):
    return services.load(user_id)

# 3. or use an async library all the way down (httpx.AsyncClient, AsyncSession, redis.asyncio)
```

## 5. What counts as "on the event loop"

| Runs on the loop (checked) | Runs in a thread (not reported) |
|---|---|
| `async def` endpoints, dependencies, middleware, exception handlers, websockets | `def` endpoints and `def` dependencies |
| `lifespan`, startup/shutdown handlers (also `def` ones) | sync background tasks |
| async background tasks | `asyncio.to_thread`, `run_in_threadpool`, `loop.run_in_executor`, `anyio.to_thread.run_sync` |
| callbacks given to `loop.call_soon`, `call_later`, `add_done_callback` | `ThreadPoolExecutor.submit`, `threading.Thread(target=...)` |

Awaiting an API is never blocking: `await redis.asyncio.Redis().get(k)` is fine,
`redis.Redis().get(k)` is reported.

## 6. Options

| Flag | `pyproject.toml` key | |
|---|---|---|
| `--python PATH` | `python` | virtualenv (or interpreter) to resolve imports |
| `--follow-libs` | `follow-libs` | also analyze installed libraries' code (finds blocking inside dependencies) |
| `--cpu` | `cpu` | also report CPU-heavy calls (bcrypt, pbkdf2, scrypt) |
| `--strict` | `strict` | list calls hydroid could not resolve, and fail on them |
| `--exclude GLOB` | `exclude` | skip files, e.g. `migrations/**` (repeatable) |
| `--no-cache` | | do not reuse or save extracted facts in `.hydroid_cache/` |
| `--format human\|json\|sarif` | | output format |

Flags win over `pyproject.toml`:

```toml
[tool.hydroid]
python = ".venv"
exclude = ["migrations/**", "scripts/**"]
follow-libs = false
strict = false
```

## 7. Teach it about your own blocking functions

Add sinks (by the **defining** module path of the function) in `pyproject.toml`:

```toml
[[tool.hydroid.sink]]
category = "sdk"
advice = "use AcmeAsyncClient"
functions = [
    "acme.client.Client.fetch",
    "acme.client.Client.*",       # `*` matches within one dotted segment
]

[[tool.hydroid.blocking_decorator]]   # sync functions decorated with these block when called
category = "retry"
advice = "decorate a coroutine instead"
functions = ["mylib.retry.retrying"]

[tool.hydroid.offload]                 # callables passed to these run in a thread
functions = ["mylib.jobs.run_in_worker"]
```

The builtin list lives in [`crates/hydroid_core/catalog.toml`](../crates/hydroid_core/catalog.toml).

## 8. Silence a finding

```python
time.sleep(0.01)  # hydroid: ignore
```

The comment goes on the line hydroid reports (the `-->` line).

## 9. Unresolved calls

Python is dynamic; some calls cannot be resolved statically (`getattr(obj, name)()`, untyped
parameters, callables stored in dicts). hydroid never drops them silently: the summary line counts
them, and `--strict` lists each one reachable on the loop and exits `1`. Adding type annotations
usually resolves them.

## 10. CI (GitHub code scanning)

```yaml
# .github/workflows/hydroid.yml
name: hydroid
on: [push, pull_request]
jobs:
  hydroid:
    runs-on: ubuntu-latest
    permissions:
      security-events: write
    steps:
      - uses: actions/checkout@v4
      - uses: astral-sh/setup-uv@v6
      - run: uv sync
      - run: cargo install --git https://github.com/agusmdev/hydroid hydroid
      - run: hydroid . --format sarif > hydroid.sarif || true
      - uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: hydroid.sarif
```

Each blocking chain shows up as a code flow on the pull request. Drop `|| true` and the
upload step to simply fail the build instead.

## 11. JSON output

`--format json` prints the full report: `diagnostics` (location, function, chain, sink,
reached_from), `unresolved`, and `stats` (files, functions, call sites resolved, timings).
