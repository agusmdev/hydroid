You are a black-box adversary for hydroid, a static analyzer that reports blocking calls
(sync I/O, sleeps, locks, sync DB/HTTP clients...) that run on the asyncio event loop in FastAPI
code. Your job: write {COUNT} small, realistic FastAPI apps that hydroid gets WRONG, each proven by
running it. Theme for you: **{THEME}**.

## What you may read

- `README.md` and `docs/GUIDE.md` (what hydroid claims to do)
- `crates/hydroid_core/catalog.toml` (the blocking functions it knows, by defining qualified name)
- `tests/adversarial/cases/*/app.py` with split `train` or `open` in `tests/adversarial/split.json`
  (what is already covered: do not repeat a mechanism)
- `tests/adversarial/candidates/` (other adversaries' work in progress: do not duplicate)

Do NOT read anything else under `crates/` - you attack the behavior, not the implementation.
Do NOT read cases whose split is `holdout`.

## How to write a case

Create `tests/adversarial/candidates/<snake_case_name>/app.py` and `RATIONALE.md` (why hydroid
likely gets it wrong, and why the labels are right at runtime).

Labels are trailing comments on the line where hydroid must report:
- `# expect: requests.api.get` - one diagnostic, blocking function with this defining qualified
  name (`requests.get` is `requests.api.get`; see the catalog). `# expect: *` when unsure of the
  name. Several names separated by spaces for several diagnostics on one line.
- The line is the call site in the innermost function that runs on the loop: in an `async def`,
  the line calling into the sync code that blocks; in a sync function the framework itself calls
  on the loop (startup handler, computed field during response serialization...), the blocking
  call itself.
- Lines without a label must produce no diagnostic. Add near-miss negatives (offloaded with
  `asyncio.to_thread` / `run_in_threadpool`, sync endpoints and sync dependencies - they run in a
  threadpool, awaited async look-alikes, generators never iterated on the loop).

Libraries available: fastapi, starlette, pydantic, requests, httpx, sqlalchemy, sqlmodel, redis,
pymysql, anyio, tenacity, bcrypt, python-multipart, and the standard library. Nothing else.

Realistic beats clever: code a FastAPI team could ship. One mechanism per case, kept small.
Put one blocking call per labeled line.

## How to prove it

Run, and iterate until the verdict is `BREAKS`:

```sh
python3 scripts/hillclimb/check.py tests/adversarial/candidates/<name> -v
```

The oracle imports `app.py`, runs the lifespan, calls every route and websocket once with sample
inputs from the OpenAPI schema (strings `"1"`, ints `1`, files `b"data"`), and records each catalog
function called on the loop thread. On the loop, blocking calls return a stand-in value (the text
`"{}"` that accepts any use), so later lines still run. Off the loop, network and process starts
fail at once; no server is reachable except a Unix socket the app serves itself.
Calls that cannot block are not recorded: a call returning an awaitable (an async client sharing
a sync client's methods), an operation on a non-blocking socket (asyncio's transports), a call
whose arguments skip the blocking work (`make_msgid(domain=...)`), and FastAPI's own one-time read of the
endpoint's source. Blocking inside an async library function the app awaits is recorded at the
`await` line.

- `BREAKS` - done: hydroid disagrees and the runtime proves the labels.
- `PASSES` - hydroid already gets it right. Try a different angle on the mechanism, or drop it.
- `INVALID` - your labels are wrong: the runtime saw a blocking call on an unlabeled line. The
  runtime is right. Fix the labels (maybe hydroid is then wrong elsewhere, or right).
- `UNVERIFIED` - a disputed line never ran on the loop (or ran and nothing blocked). Make it run
  with the sample inputs: no conditions the inputs fail, no exceptions before it. If it truly
  cannot be reached at runtime (an exception handler path...), say so in RATIONALE.md.
- `ERROR` - read the oracle error (`-v`), fix the app (it must import and serve).

Never shape a case around a guess of how hydroid is built. Shape it around real FastAPI and
Python semantics, then let `check.py` tell you whether hydroid is wrong.

Delete candidates you give up on. Return a short list: name, verdict, one-line mechanism.
