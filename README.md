# hydroid

Finds blocking calls that run on the event loop in async FastAPI (and any asyncio) code — at any
call depth, across modules, through callbacks, decorators, context managers, properties and
subclass overrides — and shows the whole chain.

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

Written in Rust on top of [ty](https://github.com/astral-sh/ty)'s semantic model (types, imports,
go-to-definition). Polar's server (1,845 files, 107k call sites) is analyzed in ~2.5 s.

## Usage

```sh
cargo build --release
target/release/hydroid path/to/project            # finds .venv / VIRTUAL_ENV like ty
target/release/hydroid . --python .venv --format sarif > hydroid.sarif
```

| Flag | |
|---|---|
| `--python PATH` | virtualenv or interpreter to resolve imports (default: discovered) |
| `--follow-libs` | also analyze third-party function bodies |
| `--cpu` | also report CPU-bound calls (bcrypt, pbkdf2, scrypt) |
| `--strict` | report calls on the loop whose target is unknown, and fail on them |
| `--exclude GLOB` | skip files (repeatable) |
| `--format human\|json\|sarif` | output |

Exit status: `0` clean, `1` findings (or unresolved calls with `--strict`), `2` error.
Suppress a line with `# hydroid: ignore`.

Configuration in `pyproject.toml` (flags win):

```toml
[tool.hydroid]
python = ".venv"
exclude = ["migrations/**"]
strict = false

[[tool.hydroid.sink]]            # your own blocking functions (defining qualified names)
category = "sdk"
advice = "use the async client"
functions = ["acme.client.Client.fetch"]
```

## How it works

1. **Facts** (`crates/hydroid_ty`, the only crate that touches ty): every function body is walked
   in parallel; each call is resolved to its definitions through ty (types, then
   go-to-definition), including implicit calls (`with`, `for`, properties, constructors).
   Callables passed as arguments are recorded as flows (`to_thread(f)`, `Depends(dep)`, decorators).
2. **Analysis** (`crates/hydroid_core`, plain graph code): where code runs is a property of the
   call, so "does calling `f` block?" is computed once per function by a backwards BFS from
   blocking functions — shortest witness chains, no recursion, any depth. Diagnostics are the
   call sites inside loop code (`async def` bodies, sync callbacks scheduled on the loop) whose
   callee blocks. Offload APIs cut the graph; callbacks are attributed to the call site passing
   them; overrides are found by class-hierarchy analysis.
3. **Catalog** (`crates/hydroid_core/catalog.toml`): blocking functions, offload APIs, loop
   callback APIs and FastAPI entry points, by defining qualified name. A test checks every name
   exists in a real environment.

Sync `def` endpoints and dependencies are not reported: FastAPI runs them in a threadpool.

## Limits

Python is dynamic: calls through `getattr`, untyped parameters, or containers of callables cannot
always be resolved. They are never silently dropped — `--strict` lists every one reachable on the
loop. Not modeled yet: operator dunders (`__add__`, `__getitem__`), `.pyi`-only libraries under
`--follow-libs`, and aliases like `__enter__ = acquire`.

## Development

```sh
cargo test --workspace        # needs `uv`; syncs tests/fixtures/.venv from uv.lock
scripts/corpus.sh             # runs on pinned real codebases
```

Fixture cases live in `tests/fixtures/cases/*`; expectations are inline comments:
`# expect: time.sleep via route GET /x`, `# expect-unresolved`.

ty's crates are pinned to exact versions (`=0.0.15`); they have no API stability promise.
The bundled typeshed stubs are Apache-2.0; hydroid is MIT.
