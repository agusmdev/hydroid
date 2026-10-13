# route_openapi_override

The documented FastAPI pattern for customising the schema: assign a plain function to
`app.openapi`. FastAPI registers `/openapi.json` itself (`FastAPI.setup`) as
`async def openapi(req): ... JSONResponse(self.openapi())`, so the override is a sync function
called directly from an async route on the event loop for every `/openapi.json` (and so every
`/docs` load). This override does not cache, and reads the changelog file each time:
`CHANGELOG.read_text()` blocks the loop (line 19). The label sits in `custom_openapi` because the
async caller is FastAPI's own code.

hydroid likely misses it: `custom_openapi` is never called from project code; it reaches the loop
through an instance attribute assignment that replaces a FastAPI method used by a framework
route.

Oracle: `GET /openapi.json` (a non-schema route the oracle drives) records `pathlib.Path.read_text`
on the loop at line 19. The `/orders` endpoint is clean.
