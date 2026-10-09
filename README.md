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

With [uv](https://docs.astral.sh/uv/), using the prebuilt binaries of the latest release
(macOS, Linux, Windows; no Rust needed):

```sh
uvx --from hydroid-cli --find-links https://github.com/agusmdev/hydroid/releases/expanded_assets/v0.1.0 hydroid .
```

Or straight from the source on `main` (builds once with a Rust toolchain, then cached):

```sh
uvx --from git+https://github.com/agusmdev/hydroid hydroid .
```

To install the `hydroid` command permanently, replace `uvx --from` with `uv tool install`
(e.g. `uv tool install git+https://github.com/agusmdev/hydroid`), then run `hydroid .` anywhere.
From a clone: `cargo install --path crates/hydroid`. More in the [guide](docs/GUIDE.md#1-install).

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
      - run: >-
          uvx --from hydroid-cli --find-links https://github.com/agusmdev/hydroid/releases/expanded_assets/v0.1.0
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
   `__getattr__`, iterators' `__next__`, constructors with `__post_init__` and pydantic
   validators, builtins and `contextlib` helpers that iterate, enter or call their arguments like
   `sorted(xs, key=f)` or `stack.enter_context(cm)`, `singledispatch` implementations). Callables passed as arguments are recorded as flows
   (`to_thread(f)`, `Depends(dep)`, `on_startup=[f]`, decorators), and so are callables stored
   on `self` and returned by functions. When ty does not know a value's type because it comes
   from an unannotated function, the class that function returns is used. The facts of an unchanged project are reused from `.hydroid_cache/` (keyed by the
   binary, the options, the installed packages and every source file's contents).
2. **Analysis** (`crates/hydroid_core`, plain graph code): where code runs is a property of the
   call, so "does calling `f` block?" is computed once per function by a backwards BFS from
   blocking functions — shortest witness chains, no recursion, any depth. Diagnostics are the
   call sites inside loop code (`async def` bodies, sync callbacks scheduled on the loop) whose
   callee blocks. Offload APIs cut the graph; callbacks are attributed to the call site passing
   them; overrides are found by class-hierarchy analysis (test doubles only stand in for real
   classes in test code); calling a generator function runs nothing until the generator is
   iterated; a decorated function is reached through its decorator's wrappers only when they call
   it. Starting an event loop (`asyncio.run`) is only reported when loop code does it directly:
   sync bridges reaching it almost always check for a running loop first.
3. **Catalog** (`crates/hydroid_core/catalog.toml`): blocking functions, offload APIs, loop
   callback APIs and FastAPI entry points, by defining qualified name. A test checks every name
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
