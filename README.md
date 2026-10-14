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
   in parallel, and each call is resolved through ty to the functions it can run, implicit calls
   included (`with`, `for`, properties, descriptors, operators, constructors). Callables passed
   as arguments, stored on instances or returned are recorded as flows. What a library does
   that its source does not show (stub-only stdlib, frameworks calling project code later) comes
   from the catalog. The facts of an unchanged project are reused from `.hydroid_cache/`.
2. **Analysis** (`crates/hydroid_core`, plain graph code): "does calling `f` block?" is computed
   once per function by a backwards search from blocking functions, giving shortest witness
   chains. Diagnostics are the call sites in loop code (`async def` bodies, callbacks the loop
   runs, code a framework runs on the loop around an entry point) whose callee blocks. Offload
   APIs cut the graph. Overrides are found by class hierarchy, narrowed by what is known about
   the instance (or argument). Generators run when iterated, not when called.
3. **Catalog** (`crates/hydroid_core/catalog.toml`): library knowledge as data, by defining
   qualified name. It covers:
   - what blocks, and what offloads or schedules on the loop;
   - what functions do with their arguments;
   - framework entry points and what the framework runs around them;
   - model hooks, observers (logging), lazily loaded attributes, and state namespaces.
   Library knowledge goes here, not in the Rust code. Every table can be extended from
   `pyproject.toml`, and a test checks that every name exists in a real environment.

Sync `def` endpoints and dependencies are not reported: FastAPI runs them in a threadpool. The
details, and what hydroid knowingly misses: [docs/GUIDE.md](docs/GUIDE.md#12-how-it-works).

## Limits

Python is dynamic: calls through `getattr`, untyped parameters, or containers of callables cannot
always be resolved. They are never silently dropped: `--strict` lists every one reachable on the
loop. What hydroid knowingly misses, and why: [docs/GUIDE.md](docs/GUIDE.md#known-limits).

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
