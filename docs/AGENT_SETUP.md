# Setting up hydroid with a coding agent, and in CI/CD

Two parts:

1. [A prompt](#1-prompt-for-a-coding-agent) to paste into Claude Code, Cursor, Codex or any coding
   agent. It installs hydroid in your repository, configures it, deals with the findings that are
   already there, and adds a CI job.
2. [A CI/CD reference](#2-cicd-reference) with ready-to-use configs for when you set it up
   yourself.

## 1. Prompt for a coding agent

Copy everything inside the block below into your agent, from the root of the repository. Change
`v0.1.0` if a newer [release](https://github.com/agusmdev/hydroid/releases) exists.

````markdown
Set up hydroid in this repository. hydroid is a static analyzer that finds blocking calls
(`time.sleep`, `requests.get`, sync DB drivers, file I/O…) reachable from code running on the
asyncio event loop in FastAPI / asyncio apps, and prints the call chain that reaches each one.
Docs: https://github.com/agusmdev/hydroid/blob/main/docs/GUIDE.md

How to run it (prebuilt binaries, no Rust needed; it needs `uv`):

    uvx --from hydroid-cli --find-links https://github.com/agusmdev/hydroid/releases/expanded_assets/v0.1.0 hydroid <PROJECT_ROOT>

Facts you need:
- It resolves imports like a type checker, so the project's dependencies must be installed in a
  virtualenv first (`uv sync`, `poetry install`, `pip install -r ...` into `.venv`). It finds the
  venv via `VIRTUAL_ENV`, an active Conda env, `<root>/.venv`, then `python3` on PATH. Otherwise
  pass `--python path/to/.venv`.
- `<PROJECT_ROOT>` is the directory containing the Python package(s) and, ideally, the
  `pyproject.toml`. Config is read from `<PROJECT_ROOT>/pyproject.toml` under `[tool.hydroid]`.
- Exit status: 0 clean, 1 blocking calls found (or unresolved calls with --strict), 2 hydroid
  itself failed (bad path, bad config, no Python found).
- Flags: `--python PATH`, `--exclude GLOB` (repeatable), `--follow-libs`, `--cpu`, `--strict`,
  `--no-cache`, `--format human|json|sarif`. Config keys: `python`, `exclude`, `follow-libs`,
  `cpu`, `strict`. Flags win over config.
- `--format json` prints `diagnostics` (each with location, function, chain, sink,
  reached_from), `unresolved` and `stats`. Paths are relative to the project root.
- A finding's `-->` line is the call inside loop code to fix. `# hydroid: ignore` on that exact
  line suppresses it.
- It writes a cache to `<PROJECT_ROOT>/.hydroid_cache/`.

Do this, in order, and stop to ask me only if a step is genuinely ambiguous:

1. Find the FastAPI/asyncio project root(s) in this repo (look for `FastAPI(`, `APIRouter(`,
   `asyncio.run(`, and the `pyproject.toml` / requirements file next to them). If there are
   several services, handle each one separately.
2. Install the project's dependencies into a virtualenv with the tool the repo already uses.
   Do not switch package managers.
3. Run hydroid once with `--format human` and once with `--format json`. If it exits 2, fix the
   setup (wrong root, missing venv) before going on. Read the `stats` in the JSON: if few call
   sites resolved, the venv is probably wrong.
4. Add `.hydroid_cache/` to `.gitignore`.
5. Add a `[tool.hydroid]` table to the project's `pyproject.toml` (create a minimal
   `pyproject.toml` with only that table if there is none). Set `python` only if the venv is not
   discovered automatically. Add `exclude` globs for code that never runs on the event loop in
   production: migrations, one-off scripts, and tests if they produce noise.
6. If the code base wraps a blocking client of its own (an internal SDK, a sync HTTP or DB
   wrapper hydroid cannot see through, a custom offload helper), declare it, using the
   *defining* module path of each function:

       [[tool.hydroid.sink]]
       category = "sdk"
       advice = "use the async client"
       functions = ["acme.client.Client.fetch"]

       [tool.hydroid.offload]        # helpers that run their callable in a thread
       functions = ["myapp.utils.run_sync"]

7. Triage every finding. For each one, read the chain and decide:
   - Real bug (blocking work on the loop): report it to me in a table with the file:line, the
     chain and the fix you recommend (`await asyncio.to_thread(...)`, make the endpoint a sync
     `def`, or switch to an async library). Do NOT fix application code unless I asked for it.
   - False positive (the call never actually runs on the loop, or is trivially fast): add
     `# hydroid: ignore` on the `-->` line with a short reason in the same comment, e.g.
     `# hydroid: ignore - cached after startup, never hits disk`.
   - If there are many real findings that I won't fix now, say so; the CI job below is then added
     in report-only mode (step 8) instead of failing the build.
8. Add a CI job using the CI system this repo already has (GitHub Actions, GitLab CI, …).
   Use the reference configs in
   https://github.com/agusmdev/hydroid/blob/main/docs/AGENT_SETUP.md#2-cicd-reference.
   Pin the hydroid release (`v0.1.0` above). The job must install the dependencies exactly like
   the existing test job does, then run hydroid.
   - On GitHub with code scanning available: upload SARIF so findings appear on pull requests.
   - Gate mode (no known findings left): let the job fail on exit status 1.
   - Report-only mode (known findings left): do not fail on exit status 1, but always fail on 2.
9. Optionally add a pre-commit hook if the repo already uses pre-commit.
10. Run hydroid one last time with the final configuration, and tell me: the command(s) you
    added, the config, the findings you fixed or suppressed and why, and the open findings.
````

## 2. CI/CD reference

All examples install hydroid from the prebuilt wheels of a pinned GitHub release, which takes a
few seconds. Building from source (`cargo install --git https://github.com/agusmdev/hydroid
hydroid`) also works but takes minutes.

hydroid needs the project's dependencies installed, so run it after the same install step as your
tests. Exit status `1` means findings, `2` means hydroid could not run: never ignore `2`.

### GitHub Actions: fail the build

```yaml
# .github/workflows/hydroid.yml
name: hydroid
on:
  pull_request:
  push:
    branches: [main]

env:
  HYDROID: uvx --from hydroid-cli --find-links https://github.com/agusmdev/hydroid/releases/expanded_assets/v0.1.0 hydroid

jobs:
  hydroid:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: astral-sh/setup-uv@v6
      - run: uv sync                      # or your usual install into .venv
      - run: $HYDROID .
```

### GitHub Actions: code scanning (findings on the pull request)

Each blocking chain shows up as an alert with a code flow, annotated on the diff. Requires
GitHub code scanning (public repositories, or GitHub Advanced Security).

```yaml
jobs:
  hydroid:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      security-events: write
    steps:
      - uses: actions/checkout@v4
      - uses: astral-sh/setup-uv@v6
      - run: uv sync
      - name: hydroid
        # exit 1 (findings) is reported through code scanning; exit 2 (error) fails the job
        run: $HYDROID . --format sarif > hydroid.sarif || [ $? -eq 1 ]
      - uses: github/codeql-action/upload-sarif@v3
        if: always() && hashFiles('hydroid.sarif') != ''
        with:
          sarif_file: hydroid.sarif
          category: hydroid
```

To both annotate and fail, run without `|| [ $? -eq 1 ]` and keep the upload step's
`if: always()`.

### Project in a subdirectory (monorepos)

hydroid prints paths relative to the project root it was given. Run it from that directory and
tell the SARIF upload where it is so annotations land on the right files:

```yaml
      - run: uv sync
        working-directory: backend
      - run: $HYDROID . --format sarif > hydroid.sarif || [ $? -eq 1 ]
        working-directory: backend
      - uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: backend/hydroid.sarif
          checkout_path: ${{ github.workspace }}/backend
          category: hydroid-backend
```

For several services, use a matrix over the directories, with one `category` each.

### Not using uv for the project

hydroid only needs a Python environment with the dependencies installed. Point it at whatever
your build creates:

```yaml
      - uses: actions/setup-python@v5
        with: { python-version: "3.12" }
      - run: pip install -r requirements.txt     # or: poetry install / pdm install
      - uses: astral-sh/setup-uv@v6               # only to fetch hydroid
      - run: $HYDROID . --python "$(python -c 'import sys; print(sys.prefix)')"
```

### GitLab CI

GitLab shows code quality and SAST reports in its own formats, not SARIF, so keep the JSON
report as an artifact and let the job status gate the merge request:

```yaml
hydroid:
  image: ghcr.io/astral-sh/uv:python3.12-bookworm
  variables:
    HYDROID: uvx --from hydroid-cli --find-links https://github.com/agusmdev/hydroid/releases/expanded_assets/v0.1.0 hydroid
  script:
    - uv sync
    - $HYDROID . --format json > hydroid.json || true   # the report, kept as an artifact
    - $HYDROID .                # log output and job status (reuses the cache: fast)
  artifacts:
    when: always
    paths: [hydroid.json]
```

Report-only: add `allow_failure: { exit_codes: [1] }` to the job.

### Other CI systems (CircleCI, Jenkins, Buildkite, Azure Pipelines…)

The same three steps everywhere: install `uv`, install the project's dependencies, run

```sh
uvx --from hydroid-cli --find-links https://github.com/agusmdev/hydroid/releases/expanded_assets/v0.1.0 hydroid .
```

and use the exit status. Azure DevOps and many other tools also ingest the `--format sarif`
output.

### pre-commit

```yaml
# .pre-commit-config.yaml
repos:
  - repo: local
    hooks:
      - id: hydroid
        name: hydroid
        entry: uvx --from hydroid-cli --find-links https://github.com/agusmdev/hydroid/releases/expanded_assets/v0.1.0 hydroid .
        language: system
        types: [python]
        pass_filenames: false   # hydroid analyzes the whole project: blocking chains cross files
```

The second and later runs on an unchanged project reuse `.hydroid_cache/` and take well under a
second; add `.hydroid_cache/` to `.gitignore`.

### Adopting it on a project with existing findings

1. Run once and triage: fix the real ones, add `# hydroid: ignore` (with a reason) on the
   false positives, `exclude` code that never runs on the loop.
2. If real findings remain that you cannot fix yet, run report-only (code scanning, or
   `allow_failure`) so new ones are visible on pull requests without blocking them.
3. Once the count reaches zero, switch to failing the build.
4. Later, consider `--strict` (fails on calls hydroid could not resolve on the loop) once type
   annotations make that list short.
