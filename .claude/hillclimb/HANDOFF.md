# hydroid hillclimb: session handoff (branch `claude/eager-ramanujan-8bg26q`)

## 1. State

- **Last round recorded:** 15. It is a protocol entry: the holdout baseline after round-6 promotions, at commit `880d361`. The last fix round recorded is round 14.
- **Unrecorded fix commits:** `48ae1ca` and `db87cac` sit on top of round 15. Their gate run fails (see open items), so they were **not** recorded as round 16.
- **Last commit before this handoff:** `db87cac`.
- **Session start:** `f02284a`, recorded as round 5 (no adversarial rounds ran between round 5 and this session).
- Rounds 6-15 in `_state.json` belong to this session. Rounds 6, 9, 11, 13 and 15 are protocol entries that record the holdout baseline before any fix (user-approved).

**Scores at `db87cac`:**

| Split | Recall | False positives | Clean cases |
|---|---|---|---|
| train (incl. 7 `open`) | 160/161 labels | 1 | 68/70 |
| holdout | 4/34 labels | 0 | 3/29 |

- Session start was train 116/118, 0 FP, with no holdout.
- Holdout batch 3 was sealed in this session. 3 of its 29 cases pass, all through general fixes no fixer saw: a logging webhook handler, stdlib archive helpers, and an `Annotated` field validator.

**Polar server (4 vCPU box):**

| Binary | Cold (`--no-cache`) | Warm (cache hit) |
|---|---|---|
| Final (`db87cac`) | 10.8 / 11.2 / 11.3 s | 651 / 707 / 681 ms |
| Session start (`f02284a`), measured today | 10.8 / 9.7 / 10.3 s | 283 / 331 / 314 ms |

- Recorded in earlier rounds: round 4 warm median 365 ms; round 5 cold 10.3 s.
- Warm time is now above the 500 ms goal. The analysis phase grew from about 250 ms to 500-900 ms: per-instance specialisation, logging scopes, pydantic hook closure, template hooks.
- **Corpus diagnostics (final vs session start):** polar 104 / 104, mealie 192 / 191, dispatch 6 / 6, full-stack template 0 / 0.
  - Mealie +1 is a true positive: `dependencies.py:127`, `session.commit()` in an async dependency, reached through an unannotated `Depends` parameter.
  - Some mealie chains now name a better sink (commit instead of execute/rollback).
  - Not run: lnbits, litellm, open-webui (no clone or venv in this container; litellm is very slow). `gate.py --polar` was not used; the timings above come from running the binaries by hand.

## 2. Open items

- **Gate failure on the unrecorded commits** (`gate.py --base 880d361`): build, clippy and tests pass, then score fails.
  - `rc_route_hook_queue_override`: false positive at `app.py:28`. A base `APIRoute` subclass's `get_route_handler` wrapper calls `self.record`; it should dispatch to the `route_class` subclass FastAPI actually instantiates.
  - `mw_route_handler_per_request`: miss at `app.py:24`. `get_route_handler()` runs per request on the loop for included routes in FastAPI 0.141.
- **WIP branch `wip/route-class-hooks`** (pushed): the stopped fixer's partial work on the two cases above. It treats route_class instances as exact receivers and `get_route_handler` as a framework hook. Incomplete and not gated.
- **Leftover candidates deleted.** All were `BREAKS` at deletion and none was promoted. Each is one line in its RATIONALE; recreate one if needed.
  - `auth_middleware_on_error`: a sync `on_error` callback passed to `AuthenticationMiddleware` runs on the loop.
  - `bg_task_factory`: `loop.set_task_factory(f)`; the factory runs on every `create_task`.
  - `conc_done_future_callback`: `concurrent.futures.Future.add_done_callback` on a finished future runs inline.
  - `conc_run_in_inline_executor`: `run_in_executor` with a project `Executor` whose `submit` runs inline.
  - `di_marked_coroutine_dep`: `inspect.markcoroutinefunction` on a sync dependency, so FastAPI awaits it on the loop.
  - `file_bytesio_binaryio`: **FP**. A `BinaryIO` helper given `io.BytesIO` reports `typing.IO.read`. Fixer judged it not fixable cheaply without an in-memory allow-list.
  - `flow_methodcaller_map`: `operator.methodcaller("read_text")` applied via `map`.
  - `mod_star_all`: `from x import *` with `__all__` rebinding an earlier local definition.
  - `mw_route_class_matches`: `APIRoute.matches` override. Possibly unreachable for included routes in FastAPI 0.141.
  - `py_metaclass_call`: singleton metaclass `__call__`.
  - `rc_custom_request_cookies`: a custom Request's `cookies` property, read by `solve_dependencies` for `Cookie()` params.
  - `rc_route_url_path_for`: `route_class.url_path_for` override reached via `request.url_for`.
- **Unproven FP leads** (never became cases):
  - `Depends(Cls)` where `Cls` has `async __call__`: FastAPI only constructs it, yet hydroid reports `__call__`. Every `async def` is a root.
  - `default_response_class` applied to routes that always return a `Response`.
  - A provider annotated with a base class whose returned subclass is safe.
- **Harness limits hit** (human-approved harness changes, for later):
  - `check.py` has no `--cpu` mode, so CPU sinks can't be verified.
  - Sync and async redis share `redis.commands.core` qualnames, so awaited async redis calls show up as blocking.
  - Awaited `httpx.AsyncClient` calls record `_socket.socket.send` on the loop.
  - FastAPI 0.141's `inspect.getsourcelines(endpoint)` file read on the first request is attributed to wrapping async frames (`await self.app(...)`, `await original_route_handler(...)`). It should be ignored when the stack runs through `fastapi.routing._extract_endpoint_context`.
  - Internal `Condition.wait` / `Event.wait` inside stdlib (`Future.exception(timeout)`, `ApplyResult.get`) are not recorded.
  - `smtplib.SMTP()` / `ftplib.FTP()` with no host are flagged blocking by the oracle, though they don't connect.
- **`gate.py` holdout ratchet** compares against the last recorded `holdout_recall` even when the holdout set changed. This session worked around it by recording a protocol baseline after each promotion (user-approved). A real fix would key the ratchet by the set of holdout cases.
- **Holdout-hashed false positives** block every gate, because the gate requires 0 holdout FP. User decision: don't promote them.
  - From round 5 on, they went to fixers as review leads and were then deleted; fixers wrote their own fixtures.
  - Deleted this way after being fixed: `fp_streaming_genexp`, `fp_streaming_map_lambda`, `fp2_async_override`, `typ_generic_service_bound_repo`, `fp4_pyd_nested_instance_no_revalidate`, `fp5_cm_factory_entered_in_threadpool`.
  - `file_bytesio_binaryio` is still unfixed.
- **No flaky tests seen.** `cargo fmt --all` reformats gate-locked files (`crates/hydroid/tests/fixtures.rs`, `tests/common/`), so fixers avoided it. The repo isn't rustfmt-clean, and CI doesn't check fmt.

## 3. Library-specific rules added on this branch

Line numbers are at `db87cac`.

### `crates/hydroid_core/src/analysis.rs`

- **Pydantic dumps**
  - `:27-30`: `pydantic.main.BaseModel.model_dump` / `model_dump_json` / `dict` / `json` (`DUMPS`). A dump call runs the receiver class's serialization hooks.
- **Logging dispatch**
  - `:242-247`: log method names `exception`, `info`, `warning`/`warn`, `error`, `critical` mapped to levels 20/30/40/50.
  - `:727`: `logging.Handler`, `logging.Filter`, `logging.Formatter` are the bases whose project subclasses supply logging hooks.
  - `:752`: functions passed to `logging.Filterer.addFilter` become filters.
  - `:767`: `logging.handlers.QueueListener` present means handlers run off the loop.
  - `:779-784`: handler hooks `handle`, `filter`, `emit`.
  - `:790`: stdlib handler I/O methods `format`, `shouldRollover`, `reopenIfNeeded`, `send`.
  - `:797-808`: formatter hooks `filter`, `format`, `usesTime`, `formatMessage`, `formatTime`; plus `formatException`, `formatStack` for `.exception()` only.
  - `:906-908`: attachment scope via `logging.Logger.addHandler`, `logging.Filterer.addFilter`, `logging.Handler.setFormatter`.
  - `:955-960`: logger levels from `logging.Logger.setLevel`, `logging.LoggerAdapter.setLevel`, `logging.basicConfig`. `logging.config.dictConfig` / `fileConfig` mean any level.
  - `:996-1002`: handler levels from `logging.Handler.setLevel` and `logging.Handler.__init__(level=)`.
- **FastAPI framework hooks**
  - `:1041`: the `response_model` keyword flow is the declared response model.
  - `:1046-1047`: `endpoint` slot and the `route` entry kind, for response-class registration.
  - `:1065-1068`: `fastapi.*` calls with `response_class` / `default_response_class`. The class's `__init__`, `render`, `init_headers` (`:1077`) run on the loop.
  - `:1084-1093`: `middleware` entry kind, `starlette.*` callee, `middleware_class` / `cls` slot. The class `__init__` runs on the loop (middleware construction).
  - `:1098-1108`: `route` / `websocket` / `endpoint` and `dependency` slots on `fastapi.*`. These drive request validation (class dependency: its `__init__`), response validation and response serialization.
  - `:1125`: the `route` kind plus `returns_response` skips response hooks.
  - `:1160-1161`: entry-kind/slot matching for instance flows: `dependency` / `route` / `websocket` / `endpoint`. Used by template hooks and instance replacement.
- **Library template methods and overrides**
  - `:1200` `loop_self_calls`, `:1239` `template_hooks`: library async methods' `self.<name>()` calls dispatch to project overrides. Generic: no names.
  - `:1666` `inherited_sink`: a sync library method overriding a catalog sink inherits the sink. Library classes only. Example: `sqlmodel.orm.session.Session.execute`.

### `crates/hydroid_core/src/facts.rs`

- Doc-only references at `:69-75`, `:133`, `:166`, `:261-267`, `:284`: `when_used="json"`, `@validates`, `response_model_exclude`, `exclude=` / `include=`, `getLogger("audit")`. These are not rules.

### `crates/hydroid_ty/src/extract.rs`

- **Streaming and lazy consumers**
  - `:257`: a file must mention `StreamingResponse` to run the streaming pre-pass.
  - `:336`: `CONSUMERS`: `list`, `tuple`, `set`, `frozenset`, `dict`, `sorted`, `sum`, `min`, `max`, `any`, `all`, `next`, `join` iterate their argument.
  - `:2802-2830`: lazy arguments: `content` kwarg; `map` / `filter`; `zip` / `enumerate` / `iter` wrapping lazies.
  - `:1978`: callee exactly `starlette.responses.StreamingResponse` means a streamed lazy iterable.
- **Pydantic and typing**
  - `:297-303`: container annotations `list`, `List`, `set`, `Set`, `frozenset`, `FrozenSet`, `Sequence`, `Optional`, `Union`, `tuple`, `Tuple`, `dict`, `Dict`, `Mapping` (`declares_instances`, for nested pydantic instances).
  - `:344`: `AfterValidator`, `BeforeValidator`, `PlainValidator`, `WrapValidator` (`Annotated` field validators).
  - `:347`: `PlainSerializer`, `WrapSerializer` (`Annotated` field serializers).
  - `:1529`: the `response_model` kwarg's class flow.
  - `:1671-1672`: `response_model=None` means no model; `response_model_exclude` literal names.
  - `:1681-1695` `dump()`: `model_dump_json` / `json` mean JSON; `dict` means python; `model_dump` + `mode` (`"python"` means python); `exclude`, `include` literals.
  - `:1983` `returns_response`; `:1996`: `starlette.responses.Response` subclasses returned skip response hooks.
  - `:2302-2330`: `default_factory`, `alias` / `validation_alias` field names; `ClassVar` fields skipped.
- **Dependencies, app state, context managers**
  - `:350`: `Depends`, `Security` dependency markers (skip in request validation).
  - `:1200`: provider is the first positional arg or `dependency=` (unannotated `Depends` / `Security` params, `dependency_classes` at `:1180`).
  - `:1104-1110`: `X.app`, `X.state.name` reads typed from project stores (`state_class`).
  - `:1076`: `__enter__` lookup for unannotated `__enter__` returning `self` (`entered_class` at `:1048`).
  - `:540`, `:1933`, `:2214`: `self` / `cls` receivers.
- **Logging names and levels**
  - `:357`: `LOGS`: `debug`, `info`, `warning`, `warn`, `error`, `exception`, `critical`, `fatal`, `log`.
  - `:360`: `ATTACHES`: `addHandler`, `addFilter`, `setFormatter`.
  - `:1747-1778`: `getLogger` (+`name` kwarg), `"root"` as root, `__name__` as the module name, `logging.root`.
  - `:1830`: `propagate` assignments.
  - `:1842-1850`: `level` kwarg, `setLevel`, `log` / `__init__` first-arg levels.
  - `:1858-1869`: level names `NOTSET`, `DEBUG`, `INFO`, `WARN` / `WARNING`, `ERROR`, `FATAL` / `CRITICAL`; `logging.X` attribute; `Literal[...]` parsing (`:1875`).
- **Builtins, iteration, contextvars, asyncio**
  - `:821`: builtin-call stubs accepted: `stdlib/builtins.pyi`, `functools.pyi`, `contextlib.pyi`, `_contextvars.pyi` (adds `Context.run`).
  - `:1354`: `callback` / `run` call their first argument (`ExitStack.callback`, `Context.run`).
  - `:884`, `:1363-1365`: `iter(x)` is an implicit `x.__iter__()`, non-iterating.
  - `:1891`: `register` attribute (`functools.singledispatch` `.register`, pre-existing pattern extended).
  - `:1944`, `:2400`: `func` kwarg of `functools.partial` / wraps-style calls.
  - `:2443-2449`: `get_running_loop` resolved to `_asyncio` / `asyncio.events` (running-loop guard). `:3088`: `RuntimeError`, `Exception`, `BaseException` handlers skipped.
- **SQLAlchemy ORM**
  - `:2105` `may_be_persistent`; `:2158`, `:2167`: `AsyncSession` / `async_sessionmaker` text in the function means async ORM, so no lazy-load report.
  - `:3035-3074`: `refresh` with `attribute_names`; `commit`, `rollback`, `expire_all`, `expire`, `refresh` (no names) unload relationships.

### `crates/hydroid_ty/src/index.rs`

- **App state stores**
  - `:256`: `X.state.name = value` stores.
  - `:267`: `asynccontextmanager` lifespans' yielded dict.
  - `:482`: `X.app` receivers.
  - `:494-495`: `starlette.applications.Starlette` means app owner; `starlette.requests.HTTPConnection` means connection owner.
  - `:510-511`: owner by variable name. `app` / `application` are the app; `request` / `req` / `websocket` / `ws` / `conn` / `connection` are the connection.
- **SQLAlchemy**
  - `:287-296`: `relationship(...)` with `lazy` absent / `"select"` / `True` is lazy. Others: `selectin`, `joined`, `raise`, `noload`, `dynamic`, `write_only`.
  - `:531`: eager loader options `selectinload`, `joinedload`, `subqueryload`, `immediateload`, `contains_eager`, `raiseload`, `noload`.
  - `:553-566`: `defined_in_sqlalchemy` (path contains `/sqlalchemy/`); `validates` decorator.
- **Context managers**
  - `:543-549`: `contextmanager` decorator resolved to `contextlib` makes a generator (calling it runs nothing).
- **Pydantic serializers**
  - `:596`: `computed_field`, `field_serializer`, `model_serializer` decorators are serialization hooks.
  - `:601-603`: `when_used` = `"json"` / `"json-unless-none"` means JSON only.
  - `:613-621`: `field_serializer` field names (`*` = all), `computed_field` name.
  - `:911`: `field_serializer` / `model_serializer` for `json_only`.
  - `:991`: `ClassVar` skipped in model fields.
- **Library self-calls**
  - `:714`, `:772`, `:849`: `self` calls in library methods; `staticmethod` / `classmethod` skipped; `self` / `cls` (`self_calls` `:727`, `closure_self_calls` `:736`).

### `crates/hydroid_ty/src/lib.rs`

- `:318`: `starlette.applications.Starlette`, used to find the project's app subclass (`request.app` typing).
- `:385-393`: `sqlalchemy.orm.session` module and `sqlalchemy.orm.session.Session.execute`, the sink for lazy relationship loads.

### `crates/hydroid_core/catalog.toml` (data, not Rust)

- `:60-100`, `:180-205` (stdlib helpers that block internally):
  - network: `socket.getfqdn`, `_socket.getnameinfo`, `ssl.get_server_certificate`, `urllib.robotparser.RobotFileParser.read`, and `imaplib.IMAP4` / `IMAP4_SSL` / `poplib.POP3` / `POP3_SSL` `__init__`.
  - file: `shutil.copymode` / `copystat` / `chown` / `disk_usage` / `make_archive` / `unpack_archive` / `which`, `zipfile.is_zipfile`, `tarfile.is_tarfile`, `filecmp.cmp` / `cmpfiles`, `configparser.RawConfigParser.read`, `netrc.netrc.__init__`, `shelve.open`, `dbm.open`.
- `:218-226`: `logging.handlers.RotatingFileHandler` / `TimedRotatingFileHandler.shouldRollover`, `WatchedFileHandler.reopenIfNeeded`, `SocketHandler.send`.
- `:234-235`: `email.utils.make_msgid`, with `unless_given = ["domain"]` and `blocks_in = "socket.getfqdn"`. New catalog fields `unless_given` / `blocks_in` are parsed in `catalog.rs`.
- `:395`: `tenacity.BaseRetrying.wraps` added to the retry blocking-decorator group.
- `:436` `[log_dispatch]`: `logging.Logger` / `LoggerAdapter` `info`, `warning`, `warn`, `error`, `exception`, `critical`, `log`, and the module-level `logging.*` equivalents.
- `:465` `[inspects]`: `functools.wraps`, `functools.update_wrapper`, `inspect.signature`, `inspect.iscoroutinefunction`, `asyncio.coroutines.iscoroutinefunction`.

**Count:** about 60 distinct rules (each bullet above that is a rule, not a doc reference). 182 distinct identifier-like string literals added in Rust code (excluding comments; a few, such as `self`, `cls`, `a`, `b`, are not library names), plus about 50 catalog function names.

## 4. Holdout misses by root cause

From what was known at promotion time; no holdout file was read for this. 26 of 29 cases miss.

- **Framework calls a dunder or method on a value handed to it, outside project code:**
  - Starlette calls `iter()` on a streamed iterable (non-generator `__iter__`), or a sync `__aiter__`.
  - Starlette constructs a `WebSocketEndpoint` subclass.
  - Routing calls a registered path `Convertor.convert`.
  - asyncio calls `DatagramProtocol` methods.
  - SQLAlchemy `hybrid_property` getters on instance access.
- **Framework decides sync vs async by unwrapping or inspecting:** a sync `functools.wraps` wrapper around an async dependency, which FastAPI unwraps and awaits on the loop.
- **Framework-stored value flow not modeled:** `request.user` typed from `AuthenticationBackend.authenticate`'s return, the same class of gap as `request.state`.
- **Callables in nested or keyed containers:** dict of lists of handlers looked up by `type(event)`; handlers registered by a decorator into an instance `defaultdict(list)`.
- **Callables stored by one function, called by another:** a decorator-built `Job` object storing `self.func` then calling it inline; a module hook rebound via `global`; a parameter whose default value is `time.sleep`.
- **Protocol- or annotation-typed callables:** a Protocol with only `__call__` returned by a factory; a Protocol data member `Callable` assigned `time.sleep`.
- **Class hierarchy built dynamically:** a base class chosen by an `if` at import time (alias as base); `@functools.cache` on a class; `getattr(self, method)` dispatch in an `HTTPEndpoint.dispatch` override.
- **Dynamic imports:** `importlib.import_module("literal")`; PEP 562 package `__getattr__`.
- **Blocking inside library or stdlib bodies, not in the catalog:**
  - `MetaData.create_all` reaching `Engine.begin`
  - `SpooledTemporaryFile` read/write
  - `SMTPHandler.emit`
  - a `RotatingFileHandler` referenced by string in `dictConfig`
- **Reverse or misused offload APIs:** `anyio.from_thread.run_sync(fn)` runs `fn` on the loop; `Thread(target=f).run()` runs inline.

## 5. Lessons

- What made progress: the runtime oracle plus a holdout split. 3 holdout cases were fixed by general fixes no fixer saw, and one candidate (`cache_configparser_read`) started passing before promotion.
- False-positive adversaries aimed at the newest rules ("probe what we just added") were the most valuable. Each new rule had 2-4 false positives within one round: pydantic hooks, logging dispatch, ORM lazy loads.
- Checking the corpus after each fixer caught a real false positive the suite didn't (Polar `attempt.payout` with AsyncSession). Make the corpus diff part of every fixer's done-criteria, as later prompts did.
- Hash-split sends about a third of the false positives to holdout, where they block the gate. Decide that policy up front. The "review lead, then delete" route worked but costs holdout cases.
- Order fixers so false-positive fixes land before the gate is needed. The gate requires 0 train FP, including other open cases, so fixer N can't pass while fixer N+1's false-positive case is open.
- `gate.py`'s holdout ratchet needs to be keyed by the case set. Protocol baselines are a manual workaround.
- Rules grew library-specific (logging, pydantic, SQLAlchemy, Starlette names). Many are syntactic name matches (`StreamingResponse` text, `AsyncSession` text, variable names `request` / `app`) and should move to catalog data or be resolved by type.
- Warm runtime doubled (0.3 s to 0.68 s). Watch analysis-phase cost per fix, not only cold extraction; fixers measured cold time and called it noise.
- Fixer runs of 20-50 minutes each made 6 rounds the practical limit for this session. Smaller rounds (2-3 mechanisms) kept reviews manageable.
- Next time: run a performance round and a "move names to catalog" refactor before more recall work, and fix the harness limits listed in section 2.

## 6. Harness changes (after `7bd0d27`, harness only)

No hydroid code (`crates/`, `catalog.toml`), case labels or `split.json` changed.

**Commits:**

- `bbbcc19` oracle: calls that cannot block are not recorded. General rules, no library names:
  - A catalog method on an object whose class has `async def` methods is probed first, with every catalog call under it refused. If it only builds an awaitable, it is not recorded (awaited `redis.asyncio`).
  - A socket operation on a non-blocking socket (timeout 0) is not recorded (`httpx.AsyncClient`). Connections stay disabled except `AF_UNIX`.
  - ~~Blocking wholly inside library code under a library coroutine is not attributed to the case~~: too broad, it hid `await sdk.fetch()` blocking inside. Reverted in the follow-up commit below.
  - Catalog `unless_given` is honoured (`make_msgid(domain=...)`). This fixes the suite's only INVALID case.
  - Tests: `scripts/hillclimb/test_oracle.py`, cases in `scripts/hillclimb/tests/oracle/`. All 4 fail with the old oracle and pass now.
- `1884894` gate: the holdout ratchet is keyed by the holdout case set.
  - Each round records `holdout_set` (hash of the sorted names); `_state.json` `holdout_sets` lists the names.
  - When the set has no recorded round, the gate builds `--base` (cached in `target/gate-base/`) and scores the current set with it as the baseline.
  - New flags: `--require-holdout-gain`, and `--lock-base` (harness sessions only).
- `67def39` gate: `literals.py` fails on added Rust string literals that look like qualified names and prints the others. On `f02284a..7bd0d27` it fails with the 27 qualified names of section 3 and prints 160 other literals.
- Follow-up (commit after `eb4dd7b`): oracle attribution restored.
  - Blocking inside library code under a library coroutine the case awaits is again recorded at the case's `await` line (`before[-1]`).
  - Only FastAPI's own source read is ignored. `IGNORED_FRAMES` in `oracle_runtime.py` has one entry, `fastapi.routing._extract_endpoint_context`: one-time introspection of the endpoint that no case call causes. A general rule (no case frame below the library coroutine) could not tell it apart from an async SDK that blocks.
  - New oracle test `library_coroutine`: it awaits `fake_sdk.fetch()` (in `scripts/hillclimb/tests/oracle_lib/`, put on `PYTHONPATH` as library code), which calls `requests.get`; it must be recorded at the `await` line. It fails with the `bbbcc19` oracle. `framework_own_io` still passes, and fails without the ignore entry.
  - `check.py --suite`: no verdict or recorded-line change against `bbbcc19` (28 BREAKS, 71 PASSES). `test_oracle.py` 5/5, `test_gate.py` OK.
  - SKILL.md and fixer-prompt.md: a fixer never passes `--lock-base`; a gate run with it does not count.
- `b1ba25f` skill: the new policy (holdout FPs become fixer leads, no library rules in Rust, triage by analysis gap, no protocol rounds, `--polar` every round, branch rule).
  - `gate.py --polar` records the cold and warm medians.

**Numbers at the head (same as `7bd0d27`):**

- train 160/161, 1 FP, 68/70 clean
- holdout 4/34, 0 FP, 3/29 clean. The gate's baseline: 4/34 with the `7bd0d27` binary on set `48ec61bf2a43`.
- Polar: cold 10.8 s, warm 717 ms (medians of 3). Warm is above the 500 ms goal.

**Gate result:** `gate.py --base 7bd0d27 --lock-base HEAD --polar` passes the lock, literals (none added), build, clippy and tests. It fails at score only on the 2 known cases (`rc_route_hook_queue_override` FP, `mw_route_handler_per_request` miss). Polar was timed by hand, since the gate stops before that step.

**Verdict changes in `check.py --suite`:** only `std_email_make_msgid_fqdn`, INVALID → PASSES (line 37 `make_msgid(domain=...)` is no longer recorded). There are no candidates, so `check.py --candidates` had nothing to run.

**Labels to review:** none found wrong.

## 7. Final state for the PR to `main` (head `62c801d`)

**Rounds 16-18:**

| Round | Lever | Train | Holdout | FP | Polar warm (recorded) |
|---|---|---|---|---|---|
| 16 | phase B: library rules moved to catalog concepts | 161/161 | 4/34 | 0 | 378 ms |
| 17 | PEP 561 `-stubs` packages, logging level literals | 161/161 | 4/34 | 0 | 348 ms |
| 18 | deferred exit callbacks (`[[exit_stack]]`) | 162/166 | 4/38 | 0 | 762 ms (wrong, see below) |

- `literals.py f02284a`: 0 qualified library names in Rust (27 at `7bd0d27`). Section 3 is stale: those
  rules are now catalog data.
- Polar: 104 diagnostics; chains and unresolved calls identical from `f02284a`'s refactor baseline
  to `62c801d`.

**Round-18 speed record is wrong.** 762 ms warm / 15.7 s cold was measured on a busy machine. An
independent re-run on a quiet 8-vCPU box gave ~295 ms warm at `62c801d` (~770 ms before phase B).

**4 `open` train cases left unfixed (by decision):**
- `r18_abc_register_virtual`: an ABC with a virtual subclass (`Base.register(Impl)`).
- `r18_callback_protocol_global`: a function stored in a global typed as a callback Protocol.
- `r18_decorator_registry_dispatch`: callables put in a registry dict by a decorator, called by key.
- `r18_protocol_factory_sync_client`: a factory `-> Protocol` returning a sync HTTP client.

All are values behind Protocol/ABC types or callables through registries: the call resolves to the
abstract method, or not at all. These are the best next targets (one general mechanism: track the
concrete classes/callables that flow into a declared abstract type or a container).

**Ideas for next time:**
- Fix the 4 open cases above with one value-flow mechanism, not per-case rules.
- Time Polar only on a quiet machine, 3+ runs, and record the box; re-measure the baseline in
  the same run.
- The holdout misses of section 4 still apply; the framework-calls-a-dunder group is the biggest.
- Harness limits in section 2 (no `--cpu` mode in `check.py`, stdlib internal waits) are still open.

**Review of PR #8 (5 Claude reviewers + Codex gpt-6.1-sol), open follow-ups.** Each repro was run
on `main` (`f02284a`) and on the head. Only one was a regression against `main`: dict/list
literals with `**spread` / `*starred` elements in dispatch tables (fixed, case in
`tests/fixtures/cases/dispatch_tables`). The rest are gaps or false positives in the new
mechanisms, or older gaps; best targets first:
- Catalog merge: a user `[[entry]]`/`[[arguments]]`/`[[sink]]`/`[[observer]]` that lists a built-in
  name replaces the built-in group (its new flags go to defaults); user globs never override
  built-in globs; wildcard expansion beats exact rules (`catalog.rs` `Patterns::insert`, `hydroid_ty/src/lib.rs` expansion).
- Logging: one global threshold for all loggers (`analysis.rs` `logger_threshold`); any
  `QueueListener` built anywhere (also in tests) drops every handler hook; handler levels per class,
  test code counted; `debug` not in `dispatch`; `FileHandler`/`SMTPHandler`/`HTTPHandler` emit not sinks.
- ORM lazy loads: dropped after any awaited non-offload call (`extract.rs` `may_be_persistent_value`),
  hidden by an unrelated `AsyncSession` parameter or an unused eager option, and by awaited calls
  given a callable that never calls it (`preloaded_off_loop`).
- Pydantic: any project instance counts as "already a model" (`holds_instances`); response
  validation skipped when the return type is `Any` or a union with the model.
- Exit stacks: a stack built off the loop (startup, `__init__`, module level) and closed on the loop
  is not reported; `stack.push(cm)` and `@stack.callback` missed; `frame_access` treats every
  builtin (`len`) as frame access (false positive); nested `try/finally` is exponential in
  `stacks.rs`; `copies` lists `run_in_executor`/`Executor.submit`, which always run in a thread.
- Stubs: `--follow-libs` loses implementations re-exported from a private module; catalog names
  given by implementation path stop matching with stubs installed; stub-to-source calls lose `iterates`.
- Cache key: library contents (editable installs, in-place edits), `.gitignore`/`.ignore` files.
- Others: `lazy_arguments` skips methods (false positive for streamed genexps in methods) and is
  quadratic in names; `lifecycle` entries ignore `slot` (import-time callbacks reported);
  specialization skips parameter narrowing on receiver copies; loop-probe `try` bodies with other
  raising operations; `add_route` slot name is `route`, not `endpoint`.
- Harness: the gate ignores holdout `errors` (an erroring case raises holdout recall);
  `PROTECTED` misses `tests/fixtures/cases/` and other test files; `literals.py` only catches
  dotted names; `HANDOFF.md` §4 describes holdout mechanisms a fixer can read; the oracle is not a
  sandbox (cases write to `/tmp`, `/etc`); round 18's 762 ms stays in `_state.json`.
