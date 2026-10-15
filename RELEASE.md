# Release

Releases are built by release-please and published to PyPI as `hydroid-cli`.

1. Merge the feature PR into `main`.
2. release-please updates its open release PR (#6). Check the version bump and the changelog
   (built from the conventional commit titles). A `feat!:` or `BREAKING CHANGE:` footer bumps the
   major version (minor before 1.0).
3. Merge the release PR. The release workflow (`.github/workflows/release.yml`) tags `vX.Y.Z`
   and creates the GitHub release.
4. The same run tests, builds the wheels and publishes `hydroid-cli` to PyPI through Trusted
   Publishing. Approve the `pypi` environment deployment when GitHub asks. Wait until it is
   green and the new version is on <https://pypi.org/project/hydroid-cli/>.
5. Upgrade and check:

   ```sh
   uv tool upgrade hydroid-cli
   hydroid --version   # must print the new version
   ```

6. Smoke test: an `async def` endpoint calling `requests.get()` must report 1 blocking call.

   ```sh
   mkdir /tmp/hydroid-smoke && cd /tmp/hydroid-smoke
   uv init -q --bare && uv add -q fastapi requests
   cat > app.py <<'PY'
   import requests
   from fastapi import FastAPI

   app = FastAPI()

   @app.get("/")
   async def root():
       return requests.get("https://example.com").text
   PY
   hydroid .   # expect 1 blocking call (requests.get, app.py:8) and exit code 1
   ```
