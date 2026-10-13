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

**New here? Read the [usage guide](docs/GUIDE.md).** Adding it to CI, or letting a coding agent
set it up: [CI/CD and agent setup](#cicd-and-agent-setup).

Written in Rust on top of [ty](https://github.com/astral-sh/ty)'s semantic model (types, imports,
go-to-definition). Polar's server (1,854 files, 108k call sites) takes about as long as ty needs
to type-check it the first time (~10 s on 4 vCPUs), then ~0.35 s while nothing changed: extracted
facts are cached in `.hydroid_cache/`. `HYDROID_TIMINGS=1` prints where the time goes.

## Install and run

With [uv](https://docs.astral.sh/uv/) (prebuilt wheels for macOS, Linux, Windows; no Rust needed):

```sh
uvx --from hydroid-cli hydroid .
```

To install the `hydroid` command permanently, run `uv tool install hydroid-cli`
(without uv: `pipx install hydroid-cli` or `pip install hydroid-cli`), then run `hydroid .` anywhere.

From the source on `main` (builds once with a Rust toolchain, then cached):
`uvx --from git+https://github.com/agusmdev/hydroid hydroid .`. From a clone:
`cargo install --path crates/hydroid`. More in the [guide](docs/GUIDE.md#1-install).

```sh
hydroid path/to/project                 # finds .venv / VIRTUAL_ENV like ty
hydroid . --python .venv --format sarif > hydroid.sarif
```

| Flag | |
|---|---|
| `--python PATH` | virtualenv or interpreter to resolve imports (default: discovered) |
| `--follow-libs` | also analyze third-party function bodies |
| `--cpu` | also report CPU-bound calls (bcrypt, pbkdf2, scrypt) |
| `--strict` | report calls on the loop whose target is unknown, and fail on them |
| `--exclude GLOB` | skip files (repeatable) |
| `--no-cache` | do not read or write `.hydroid_cache/` |
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

## CI/CD and agent setup

hydroid exits `1` when it finds blocking calls, so it can gate a build as is. On GitHub,
`--format sarif` puts each blocking chain on the pull request through code scanning:

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
      - run: uv sync                      # hydroid needs the project's dependencies installed
      # Optional: pin a version for reproducible builds, e.g. hydroid-cli==0.2.0
      - run: >-
          uvx --from hydroid-cli
          hydroid . --format sarif > hydroid.sarif || [ $? -eq 1 ]
      - uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: hydroid.sarif
```

`|| [ $? -eq 1 ]` reports findings without failing the job (exit `2`, an error, still fails it);
drop it and the upload step to fail the build instead.

[docs/AGENT_SETUP.md](docs/AGENT_SETUP.md) has:

- **a prompt to paste into a coding agent** (Claude Code, Cursor, Codex…) that installs hydroid
  in your repository, configures `[tool.hydroid]`, triages the existing findings (fix,
  `# hydroid: ignore` with a reason, or report to you) and adds the CI job;
- configs for GitLab CI, pre-commit, monorepos, projects not using uv, and other CI systems;
- a path for adopting it on a code base with existing findings: report-only first, then gate.

## How it works

1. **Facts** (`crates/hydroid_ty`, the only crate that touches ty): every function body is walked
   in parallel; each call is resolved to its definitions through ty (types, then
   go-to-definition), including implicit calls (`with`, `for` and comprehensions, properties
   and their setters, descriptors, operators like `x[k]` and `k in x` on user classes,
   `__getattr__`, iterators' `__next__`, constructors with `__post_init__`, pydantic
   validators and the `default_factory` of fields the call does not give, builtins and
   `contextlib` helpers that iterate, enter or call their arguments like `sorted(xs, key=f)`,
   `stack.enter_context(cm)` or `copy_context().run(f)`, `singledispatch` implementations, SQLAlchemy
   `@validates` validators run by `Model(field=...)` and `obj.field = ...`, and reads of lazily
   loaded `relationship()` attributes, which query through `Session.execute` unless the
   relationship is eager (`lazy="selectin"`, ...) or the function loads it with an eager option
   like `selectinload(Model.attr)`, or it was already loaded off the loop: the instance was
   returned by (or passed to) an awaited offloaded helper that reads the relationship or
   refreshes it by name, with no `commit()`/`expire` in between). Callables passed as arguments are recorded as flows
   (`to_thread(f)`, `Depends(dep)`, `on_startup=[f]`, decorators), and so are callables stored
   on `self` and returned by functions. Calls through dispatch tables (`TABLE[key](...)`, or
   `send = TABLE[key]; send(...)`, where `TABLE` is a dict, list or tuple literal) resolve to the
   callables stored there, only the one under `key` when the key is a literal. When ty does not know a value's type because it comes
   from an unannotated function, the class that function returns is used; an unannotated
   `client=Depends(provider)` parameter holds what the provider returns (or yields). Objects read
   back from state, which Starlette types as `Any`, get the class the project stores under that
   name (`app.state.x = ...` for `request.app.state.x`; a lifespan's `yield {"x": ...}` or
   `request.state.x = ...` for `request.state.x`) when every store agrees on it, and
   `request.app` is the project's one `FastAPI` subclass, if it defines one. Signatures record
   what FastAPI validates and serializes: the models (and `AfterValidator`-style callables) in
   parameter annotations, the return annotation, the classes returned, whether every return
   gives a `Response` (sent as is); model fields record their nested models and their
   `Annotated` validators and serializers (through type aliases), which constructions giving
   the field data and `model_dump()` calls run. The facts of an unchanged project are reused from `.hydroid_cache/` (keyed by the
   binary, the options, the installed packages and every source file's contents).
2. **Analysis** (`crates/hydroid_core`, plain graph code): where code runs is a property of the
   call, so "does calling `f` block?" is computed once per function by a backwards BFS from
   blocking functions — shortest witness chains, no recursion, any depth. Diagnostics are the
   call sites inside loop code (`async def` bodies, sync callbacks scheduled on the loop) whose
   callee blocks. Pydantic code FastAPI runs on the loop is loop code too: validators of request
   parameters and bodies (of `def` endpoints and dependencies as well: only their bodies go to
   the threadpool), computed fields and serializers of the response, and response-model
   validators of `async def` endpoints, and the `render`/`__init__` of a `response_class=` (or
   `default_response_class=`) FastAPI constructs after every endpoint. A logging call
   (`logger.info(...)` and above; `debug` is below the levels applications run with) runs the
   `filter`/`emit`/`format` overrides of the `logging.Filter`, `logging.Handler` and
   `logging.Formatter` subclasses the project constructs, and the file and socket I/O of the
   stdlib rotating, watched and socket handlers, when the record's level reaches the lowest
   logger level the project sets (`WARNING` when it sets none) and the handler's level. A hook
   the project only attaches with `addHandler`/`addFilter`/`setFormatter` runs only for the
   loggers it is attached to: a handler for records of its logger and the loggers below it (by
   dotted name, up to one with `propagate = False`), a logger's filter only for that logger's
   own records; logger names come from `getLogger("x")`, `getLogger(__name__)`, the literal start
   of an f-string, and `logging.info(...)` logs on the root logger. Records of a logger whose
   name is not known may reach any hook. Offload APIs cut the graph; callbacks are attributed to the call site passing
   them; overrides are found by class-hierarchy analysis (test doubles only stand in for real
   classes in test code); calling a generator function runs nothing until the generator is
   iterated; a decorated function is reached through its decorator's wrappers only when they call
   it (not when they only hand it to an offload API, or to `functools.wraps`). Starting an event
   loop (`asyncio.run`) is only reported when loop code does it directly: sync bridges reaching
   it almost always check for a running loop first. Code in the `except RuntimeError:` of a `try`
   that only calls `asyncio.get_running_loop()` runs only when no loop is running: it is skipped.
3. **Catalog** (`crates/hydroid_core/catalog.toml`): blocking functions, offload APIs, loop
   callback APIs, functions that only inspect callables (`functools.wraps`) and FastAPI entry
   points, by defining qualified name. A test checks every name
   exists in a real environment.

Sync `def` endpoints and dependencies are not reported: FastAPI runs them in a threadpool.

## Limits

Python is dynamic: calls through `getattr`, untyped parameters, or containers of callables cannot
always be resolved. They are never silently dropped — `--strict` lists every one reachable on the
loop. Not modeled yet: `getattr(obj, "name")`, containers of callables (`HANDLERS[kind]()`),
metaclasses, and `.pyi`-only libraries under `--follow-libs`. A change to any source file re-extracts the whole
project (no per-file incremental cache yet).

## Development

```sh
cargo test --workspace        # needs `uv`; syncs tests/fixtures/.venv from uv.lock
scripts/corpus.sh [names]     # Polar, mealie, lnbits, dispatch, litellm, open-webui, template
```

Fixture cases live in `tests/fixtures/cases/*`; expectations are inline comments:
`# expect: time.sleep via route GET /x`, `# expect: *` (any blocking function), `# expect-unresolved`.

`tests/adversarial/` is a suite of cases written to break hydroid, split into a training half
(required to pass, by `cargo test`) and a held-out half that is only scored:

```sh
scripts/hillclimb/score.py --split train            # misses and false positives, per case
scripts/hillclimb/score.py --split holdout --quiet  # aggregate only: don't tune on it
scripts/hillclimb/polar_time.sh target/release/hydroid 5
uv run --with anthropic scripts/hillclimb/adversary.py --count 5   # Claude proposes new cases
scripts/hillclimb/oracle.py PROJECT report.json   # naive syntactic scan: candidate misses
```

ty's crates are pinned to exact versions (`=0.0.15`); they have no API stability promise.
The bundled typeshed stubs are Apache-2.0; hydroid is MIT.

Releases: [docs/RELEASING.md](docs/RELEASING.md).
