# Releasing

Releases are cut by [release-please](https://github.com/googleapis/release-please) and published
to PyPI as `hydroid-cli` by `.github/workflows/release.yml`.

## Flow

1. Pull requests are squash-merged. The PR title becomes the commit on `main`, so it must be a
   [Conventional Commit](https://www.conventionalcommits.org/) (checked by `pr-title.yml`):
   `feat:` → minor bump, `fix:`/`perf:` → patch bump (while on `0.x`), `feat!:` → breaking.
   `docs:` shows in the changelog without a bump; `build`, `ci`, `test`, `refactor`, `chore`
   are hidden.
2. Every push to `main` opens or updates the release PR `chore(main): release X.Y.Z`: version in
   `Cargo.toml` and `Cargo.lock`, and `CHANGELOG.md`. Edit the changelog text in that PR if needed.
3. Merging the release PR tags `vX.Y.Z` and creates the GitHub release. The same run then runs
   clippy and the tests, builds the wheels and the sdist, waits for approval in the `pypi`
   environment, publishes to PyPI and attaches the files to the GitHub release.

The release PR is created with `GITHUB_TOKEN`, so CI does not run on it; the release run
repeats clippy and the tests before anything is published.

## Dry run

Run the **Release** workflow by hand (Actions → Release → Run workflow), or open a PR that
touches packaging: it builds and smoke-tests every wheel without publishing.

## Setup (done once)

- PyPI → `hydroid-cli` → Publishing: trusted publisher `agusmdev/hydroid`, workflow
  `release.yml`, environment `pypi`.
- GitHub → Settings → Environments → `pypi`, with a required reviewer.

## If something breaks

- **A release run failed after the tag was made**: fix it on `main`, then re-run the failed jobs
  of that run if the fix doesn't need new code; otherwise let release-please cut the next patch.
  A version on PyPI can never be uploaded again, even after deletion.
- **`Cargo.lock` was not bumped** (`--locked` fails): the lockfile entries are matched with
  `@.name.value` in `release-please-config.json`, which depends on release-please's TOML parser.
  Check the `extra-files` against the release-please version in use.
