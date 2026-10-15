# Audit: library-specific rules on `claude/eager-ramanujan-8bg26q` (phase A)

Scope: every rule added to Rust between `f02284a` (session start, round 5) and `7311394` (head).
Sources: HANDOFF section 3 and `literals.py f02284a HEAD` (27 qualified names, 160 other
literals). Line numbers are at `7311394`. No code was changed for this audit.

Classes:

- **(a)** a general analysis mechanism replaces it (named).
- **(b)** it moves to `catalog.toml` as a general concept (named, with 2 libraries).
- **(c)** it stays (reason given). Needs your approval.

## 1. Baseline (head `7311394`)

| Measure | Value |
|---|---|
| `check.py --suite` | 28 BREAKS, 71 PASSES, 0 INVALID |
| train (incl. 7 `open`) | 160/161 labels, **1 FP**, 68/70 clean, 0 errors |
| holdout | 4/34 labels, 0 FP, 3/29 clean |
| `gate.py --base HEAD --polar` | lock, literals (none added vs HEAD), build, clippy, tests pass; **fails at score** (train 1 FP). The Polar step is not reached, so Polar was timed by hand |
| Failures | `rc_route_hook_queue_override` FP at `app.py:28`; `mw_route_handler_per_request` miss at `app.py:24` (the 2 known ones) |

Polar: fresh clone of `main` at `062d49f`, so the numbers differ from HANDOFF. Both binaries
were timed on the same checkout, as medians of 3 runs.

| Binary | Cold (`--no-cache`) | Warm (cache hit) | Diagnostics |
|---|---|---|---|
| `f02284a` | 9.9 s | **295 ms** | 104 |
| `7311394` | 11.3 s (a first set right after setup: 15.3 s) | **769 ms** | 104 |

## 2. Where the warm time went

A warm run is cache load + analysis + output. I measured it with `HYDROID_TIMINGS` and the
report's `extract_ms` / `analyze_ms`. For the analysis steps I used throw-away timers in scratch
worktrees (not committed): median of 7 warm runs per binary, on a noisy 4 vCPU box.

| Step | `f02284a` | head | Change |
|---|---|---|---|
| cache load (`extract_ms`) | 78 | 126 | +48: bigger facts (new `Call` / `Function` fields) |
| `specialize` (copies per kind of `self`) | – | 144 | +144 |
| sinks (+ `inherited_sink`) | 13 | 20 | +7 |
| flags + used params (`unless_given`, `is_inspector` lookups) | 18 | 32 | +14 |
| params / stored / returned / called | 34 | 27 | ~0 |
| hierarchy + flows index | 7 | 9 | ~0 |
| `logging_hooks` (+ scopes, thresholds, levels) | – | 49 | +49 |
| `effective_targets` | 63 | 110 | +47 |
| `framework_hooks` | – | 13 | +13 |
| blocking distances, entry paths, diagnostics, unresolved | 34 | 37 | ~0 |
| **analysis total** | **~170** | **~440** | **+270** |

Turning one mechanism off (same 104 diagnostics in each case; medians of 5, noisy):

| Mechanism off | Analysis total | Saved |
|---|---|---|
| none | 561 | – |
| specialization | 392 | ~170 |
| construction / field / default-factory hooks per construction call | 450 | ~110 (part of it existed at `f02284a`) |
| logging hooks | 484 | ~75 |
| pydantic dump hooks (`DUMPS` string compare on every call) | 533 | ~28 |
| template hooks | ~561 | within noise |
| framework hooks | ~561 | within noise |

**Groups that cause the growth:**

1. **Specialization**: ~150 ms. This is a general mechanism with no library names. Almost all of
   its cost is `facts.clone()` of 115k calls to add 594 functions and 8k calls.
2. **Construction / validation hooks** (group P): ~65–110 ms. Each construction call runs
   breadth-first walks of the class hierarchy (`construction_hooks`, `default_factories`,
   `field_hooks`, `ancestors`) with no memoization.
3. **Logging** (group L): ~50–75 ms. Classes are looked up by qualname with linear scans, and
   every call target is matched against qualname strings (`handler_levels`,
   `logger_threshold`, `log_scopes`).
4. **Bigger facts**: +48 ms of cache load.
5. Per-call string compares: `DUMPS` (~25 ms) and catalog lookups per function or flow
   (~15 ms).

Cold time: the bodies phase grew from 6.1 to 7.2 s and referenced indexes by 0.2 s (one
`HYDROID_TIMINGS` run each). This comes from the new extraction work: ORM, logging names, lazy
arguments, field types.

**Speed plan for phase B:**

- Make copies an overlay instead of cloning the facts.
- Memoize class hooks per class.
- Turn every catalog role into a precomputed per-function flag (also needed for the catalog
  move).
- Expected warm time: ~450–500 ms before any rule removal. The rule removals should then be
  neutral or faster.

## 3. Rules by mechanism

Counted: **66 rules** (rows below), in 9 groups. Classification: (a) 9 (I6–I8 counted as (a), the
preferred route), (b) 55, (c) 2 (G2, G3: language, not library).

### Group P: model validation and serialization hooks (pydantic, dataclasses)

pydantic-core is a compiled extension, so validation and dumping cannot be followed in library
source. This group needs catalog data.

**New concept `[[model_hooks]]`** (one table per model family):

- the base class;
- the validate methods and whether construction validates;
- the dump methods, with which of them dump to JSON and the keywords for mode, exclude and
  include;
- the hook decorators (validator, serializer, the field names they take, JSON-only keyword
  values);
- `Annotated` metadata classes (validator / serializer) and their callable keyword;
- field options (default factory, aliases);
- annotations to skip.

Libraries: **pydantic** (`model_dump`, `field_serializer`, `AfterValidator`) and
**marshmallow** (`Schema.load` runs `@validates` / `@post_load`, `Schema.dump(only=, exclude=)`
runs `@pre_dump` / `@post_dump`). attrs fits too (`@x.validator`, `Factory`, `asdict`).

| ID | file:line | What it does | Class |
|---|---|---|---|
| P1 | `analysis.rs:26-31` | `DUMPS`: four `pydantic.main.BaseModel` dump qualnames; a dump runs the receiver's serialization hooks | (b) `model_hooks.dump` |
| P2 | `extract.rs:1681-1711` | `dump()`: `model_dump_json` / `json` = JSON, `dict` = python, `model_dump(mode=)` with `"python"`, `exclude=`, `include=` | (b) `model_hooks.dump` fields |
| P3 | `extract.rs:344` | `VALIDATOR_METADATA`: `AfterValidator`, `BeforeValidator`, `PlainValidator`, `WrapValidator` (bare names) | (b) `model_hooks.metadata` |
| P4 | `extract.rs:347` | `SERIALIZER_METADATA`: `PlainSerializer`, `WrapSerializer` | (b) same |
| P5 | `extract.rs:1944`, `:2400` | `func=` keyword of that metadata | (b) same |
| P6 | `index.rs:593-599` | `computed_field`, `field_serializer`, `model_serializer` decorators make serialization hooks | (b) `model_hooks.serializers` |
| P7 | `index.rs:601-606`, `:911` | `when_used="json"` / `"json-unless-none"` means JSON-only | (b) same |
| P8 | `index.rs:609-628` | field names of `field_serializer` (`"*"` = all), `computed_field` name | (b) same |
| P9 | `extract.rs:2294-2312` | `default_factory=` keyword, `alias` / `validation_alias` names | (b) `model_hooks.fields`; dataclasses `field(default_factory=)` is the second user |
| P10 | `extract.rs:2330` | aliases again in `model_field` | (b) same |
| P11 | `extract.rs:2319-2320`, `index.rs:991` | `ClassVar` annotations are not fields (text match) | (b) `model_hooks.skip_annotations = ["typing.ClassVar"]`, resolved through ty, not by text |
| P12 | `extract.rs:280-311` | `declares_instances`: `list`, `List`, `set`, `Optional`, `Union`, `dict`, `Mapping`, ... by name | (a) **element classes from ty's type** of the annotation (specialization arguments of the inferred type), with no container names |

Speed: memoize `construction_hooks` / `field_hooks` / `default_factories` per class, and flag
dump methods once per function (bool vector), not by string compare per call. Expected: −60 to
−100 ms warm.

### Group F: framework request lifecycle (FastAPI, Starlette)

What a framework runs on the loop around a registered callable: request validation of its
parameters, response validation and serialization, a response class built per request,
middleware built on the first event, dependencies.

**Concept: extend `[[entry]]`** with:

- `slot`: the argument holding the registered callable;
- `validates_params`, `serializes_return`, and the response-model and response-exclude
  keywords;
- `passthrough_return`: return classes the framework sends as is.

**New concept `[[builds_on_loop]]`**: class arguments the framework constructs on the loop, with
`functions` and `slots`.

**New concept `[[dependency_marker]]`**: callables marking a parameter as injected, with the
provider slot.

Libraries:

- `[[entry]]` fields: **FastAPI** routes and **pydantic `validate_call`** (both validate
  annotated parameters on each call). Litestar fits too.
- `[[builds_on_loop]]`: **FastAPI** `response_class` / `default_response_class` and **Starlette**
  `Middleware(cls)` / `add_middleware`. aiohttp middlewares fit too.
- `[[dependency_marker]]`: **FastAPI** `Depends` / `Security` and **fast-depends**
  (`fast_depends.Depends`). Litestar `Provide` fits too.

| ID | file:line | What it does | Class |
|---|---|---|---|
| F1 | `analysis.rs:1041`, `extract.rs:1529`, `:1671` | `response_model=` keyword flow is the declared response model | (b) `[[entry]]` response-model keyword |
| F2 | `extract.rs:1670-1678` | `response_model=None` = no model; `response_model_exclude=` literal names | (b) same |
| F3 | `analysis.rs:1046-1047`, `:1098-1101`, `:1160-1161` | `endpoint` / `dependency` slots, `route` / `websocket` / `dependency` kinds | (b) `[[entry]].slot`; kind names stay as hydroid's vocabulary |
| F4 | `analysis.rs:1065-1068` | `fastapi.` prefix + `response_class` / `default_response_class` slots | (b) `[[builds_on_loop]]` |
| F5 | `analysis.rs:1077` | `__init__`, `render`, `init_headers` of the response class | (a) **library self-calls of a constructor run on the loop**: Starlette's `Response.__init__` calls `self.render` and `self.init_headers` (visible in library source; `self_calls` already indexes them). Only `__init__` stays, and it is language |
| F6 | `analysis.rs:1084-1094` | `middleware` kind + `starlette.` prefix + `middleware_class` / `cls` slots: construction on the loop | (b) `[[builds_on_loop]]` |
| F7 | `analysis.rs:1098-1150` | route / dependency `fastapi.` check; request validation, response validation (async only), response serialization | (b) `[[entry]]` `validates_params` / `serializes_return` |
| F8 | `analysis.rs:1125`; `extract.rs:1983-2001` | `returns_response`: subclasses of `starlette.responses.Response` skip response hooks | (b) `[[entry]].passthrough_return` |
| F9 | `extract.rs:350`, `:1930-1940` | `DEPENDENCY_MARKERS` `Depends`, `Security` by bare name | (b) `[[dependency_marker]]`, resolved by qualname |
| F10 | `extract.rs:1180-1206` | provider = first positional arg or `dependency=` | (b) `[[dependency_marker]].slot` |
| F11 | `analysis.rs:1152-1176` | template hooks of callable instances handed to entries (slot names only) | (b) via F3's `slot`; the mechanism itself is generic |

**The 2 known failures**, fixed with (a): **exact class of instances a library constructs
from a class argument.** When a library function stores a class parameter on `self`
(`self.route_class = route_class`) and calls it (`self.route_class(...)`), its instances are
exactly the class the project passed. Their methods, and the library `async def` closures'
`self.<name>()` calls (`closure_self_calls`, `loop_self_calls`), dispatch on that class only.
This needs:

- a library index of "parameter stored to attribute, attribute called" pairs (cheap, per
  library class, like `self_calls`);
- class-valued flows into those parameters.

It fixes:

- `rc_route_hook_queue_override`: no dispatch to the base wrapper's other subclasses.
- `mw_route_handler_per_request`: `get_route_handler` is called on the loop for the exact
  route class.

It uses no FastAPI names. `wip/route-class-hooks` is read for reference only.

### Group L: logging dispatch (stdlib `logging`)

The stdlib is stub-only for ty, so `Logger.callHandlers` cannot be followed in source.

**New concept `[[observer]]`**: a registry where some calls attach objects or callables and
other calls run them on the calling thread. Fields:

- `attach`: functions, and the slot of the attached value;
- `dispatch`: functions;
- `runs`: the methods run on attached objects (in order, with "if overridden, else these").
  Plain callables are called.
- optional `levels`: dispatch method → level; level setters with their argument slot; the
  default threshold; level names; config calls that mean "any level".
- optional `bases`: classes whose project subclasses supply the hooks.
- optional `offloaded_by`: a class whose presence moves dispatch to a thread
  (`QueueListener`).

Libraries:

- **stdlib logging**: `addHandler` / `addFilter` / `setFormatter`, then `info` / `error` / ...
- **blinker** (Flask signals): `Signal.connect(receiver)`, then `Signal.send()` runs receivers
  inline. Django `Signal.connect` / `send` is the same shape.
- `levels`: logging and **loguru** (`logger.add(sink, level=)`, `logger.info`).

For registries in project or pure-Python library source, the same behavior comes from (a)
**callables stored into a container attribute and called from it**. That is also the holdout
gap "handlers registered into an instance `defaultdict(list)`" (HANDOFF section 4).

| ID | file:line | What it does | Class |
|---|---|---|---|
| L1 | `analysis.rs:236-250` | log method names to levels (`info` 20, `warning` / `warn` 30, `error` / `exception` 40, `critical` 50); `exception` formats tracebacks | (b) `observer.levels` |
| L2 | `analysis.rs:727` | bases `logging.Handler`, `logging.Filter`, `logging.Formatter` | (b) `observer.bases` |
| L3 | `analysis.rs:752` | callables given to `logging.Filterer.addFilter` are filters | (b) `observer.attach` (callable slot) |
| L4 | `analysis.rs:767` | `logging.handlers.QueueListener` attached means handlers run off the loop | (b) `observer.offloaded_by` |
| L5 | `analysis.rs:779-784` | handler hooks `handle` → `filter` + `emit` | (b) `observer.runs` |
| L6 | `analysis.rs:790` | stdlib `emit` I/O methods `format`, `shouldRollover`, `reopenIfNeeded`, `send` | (b) `observer.runs` fallback list |
| L7 | `analysis.rs:797-808` | formatter hooks `format` / `usesTime` / `formatMessage` / `formatTime`, plus `formatException` / `formatStack` for `.exception()` | (b) same |
| L8 | `analysis.rs:902-912` | attachment scope by `logging.Logger.addHandler`, `logging.Filterer.addFilter`, `logging.Handler.setFormatter` | (b) `observer.attach` roles (on registry / on attached object) |
| L9 | `analysis.rs:945-967` | threshold from `Logger.setLevel`, `LoggerAdapter.setLevel`, `basicConfig(level=)`; `dictConfig` / `fileConfig` = any; default 30 | (b) `observer.levels` |
| L10 | `analysis.rs:984-1012` | handler level from `logging.Handler.setLevel`, `Handler.__init__(level)` | (b) same |
| L11 | `extract.rs:357` | `LOGS`: `debug` ... `log` (bare method names) | (b) `observer.dispatch`, resolved by qualname |
| L12 | `extract.rs:360`, `:2899-2990` | `ATTACHES`: `addHandler`, `addFilter`, `setFormatter`; `attach_only` scan | (b) `observer.attach` |
| L13 | `extract.rs:1713-1739` | `log_use`: `logging.info()` on a module = root logger | (b) `observer.scope` (module-level dispatch = root) |
| L14 | `extract.rs:1741-1826` | logger names: `getLogger` (+`name=`), `"root"`, `__name__`, f-string prefixes, `logging.root` | (b) `observer.scope`: the naming function, root aliases, dotted hierarchy. **Decision for you**, see section 4 |
| L15 | `extract.rs:1828-1839` | `propagate = False` stops records going up | (b) `observer.scope.stop_attribute` (same decision) |
| L16 | `extract.rs:1841-1854` | level argument: `level=`, `setLevel(x)`, `log(x)`, `__init__(x)` | (b) `observer.levels` setter slots |
| L17 | `extract.rs:1856-1878` | level names `NOTSET` ... `CRITICAL`, `logging.X`, `Literal[...]` parse | (b) `observer.levels.names`. The `Literal[` display parse is (a): use ty's literal type value, not its display string |

Speed:

- Precompute per-function role flags (attach / dispatch / setter) once, from the catalog.
- Look up base classes once by qualname in a map, not with 3 linear scans.
- Skip the whole pass when no dispatch function is reachable.

Expected: −40 to −60 ms warm.

### Group O: ORM implicit I/O (SQLAlchemy)

**New concept `[[lazy_attribute]]`**: a class attribute built by a factory call is a descriptor
whose read does I/O. Fields:

- the factory and the option keyword with its lazy values and default;
- the function the load runs (the sink);
- eager-loading options;
- methods that expire loaded attributes;
- the reload method and its names argument;
- session classes under which a load raises instead of blocking (async sessions).

Libraries: **SQLAlchemy** `relationship(lazy=)`, `selectinload`, `Session.commit` expires,
`refresh(attribute_names=)`. **Django** `ForeignKey` reads query lazily; `select_related` /
`prefetch_related` are eager; `refresh_from_db(fields=)`.

**New concept `[[attribute_set_hooks]]`**: decorators registering a method run when attributes
are set. Libraries: **SQLAlchemy** `@validates("a")` and **attrs** `@x.validator` with
`on_setattr`.

| ID | file:line | What it does | Class |
|---|---|---|---|
| O1 | `index.rs:283-305` | `relationship(...)`; `lazy` absent / `"select"` / `True` is lazy | (b) `lazy_attribute` factory + option |
| O2 | `index.rs:530-532` | `EAGER_LOADERS`: `selectinload` ... `noload` | (b) `lazy_attribute.eager_options` |
| O3 | `index.rs:553-560` | `defined_in_sqlalchemy`: path contains `/sqlalchemy/` | (a) **resolve the callee's qualname** (ty) against the catalog; no path test |
| O4 | `index.rs:562-576` | `validates` decorator names attributes | (b) `[[attribute_set_hooks]]` |
| O5 | `extract.rs:2158-2182` | function text contains `AsyncSession` / `async_sessionmaker` means async ORM | (b) `lazy_attribute.raises_under` (async session classes), checked by the **type** of the session the instance came from, not by text |
| O6 | `extract.rs:3033-3078` | `expires`: `commit`, `rollback`, `expire_all`, `expire`, `refresh` without names; `refresh(attribute_names=)` loads | (b) `lazy_attribute.expires` / `reload` |
| O7 | `lib.rs:385-393` | the load runs `sqlalchemy.orm.session.Session.execute` | (b) `lazy_attribute.loads_via` |

Speed: extraction only (cold); nothing measurable warm.

### Group S: application and connection state (Starlette)

**New concept `[[state_namespace]]`**: an attribute bag owned by a framework object, typed by
what project code stores in it. Fields:

- the bag attribute;
- owner classes (app / connection);
- the attribute of a connection that returns the app (typed as the project's app subclass);
- the lifespan slot whose yielded dict is copied into connection state.

Libraries: **Starlette** (`app.state.x`, `request.state.x`, `request.app`, lifespan dict) and
**Flask** (`g.x`, `current_app` returning the app; `app.extensions`). aiohttp `app["key"]` is
close.

| ID | file:line | What it does | Class |
|---|---|---|---|
| S1 | `index.rs:253-264`, `extract.rs:1102-1140` | `X.state.name = v` stores, `X.state.name` / `X.app` reads | (b) `state_namespace.attribute` |
| S2 | `index.rs:480-515`, `lib.rs:318` | owners `starlette.applications.Starlette`, `starlette.requests.HTTPConnection`; app subclass for `request.app` | (b) `state_namespace.owners` / `app_attribute` |
| S3 | `index.rs:506-514` | untyped owners by **variable name**: `app`, `application` / `request`, `req`, `websocket`, `ws`, `conn`, `connection` | (a) **types only**: drop the names. An untyped owner gets no state typing (recall cost: to measure on the `life_*` cases) |
| S4 | `index.rs:266-281` | `@asynccontextmanager` lifespan's yielded dict goes into connection state | (b) `state_namespace.lifespan` (with group C's decorator concept) |

### Group I: iteration and argument effects of builtins, and streaming

Builtins and stdlib helpers are stub-only.

**New concept `[[argument_effects]]`**: what a function does with its arguments, by slot:

- calls it now;
- iterates it now;
- returns a lazy iterator over it (calling or iterating later);
- `__iter__` only.

Libraries:

- **builtins** (`map`, `list`, `sorted`, `iter`, `next`);
- **itertools** / **toolz** (`itertools.starmap`, `toolz.map`);
- **contextvars** / **contextlib** (`Context.run(f)`, `ExitStack.callback(f)` call now).

**Concept: extend `[offload]`** with `iterates = slot`: an iterable given there is consumed in a
worker thread. Libraries: **Starlette** `StreamingResponse(content)` / `iterate_in_threadpool`,
and **Django** `StreamingHttpResponse` (under ASGI it consumes sync iterators with
`sync_to_async`). Preferred over the catalog, as (a): **parameters a library function hands to
an offload API** are offloaded. Starlette's `StreamingResponse.__init__` passes `content` to
`iterate_in_threadpool`, which is visible in library source. I'll try (a) first and fall back to
(b) if the library index cost is too high.

| ID | file:line | What it does | Class |
|---|---|---|---|
| I1 | `extract.rs:335-339` | `CONSUMERS` (branch added `next`, `join`...) | (b) `argument_effects` |
| I2 | `extract.rs:341`, `:841-869`, `:2813-2833` | `LAZY` `map` / `filter` / `zip` / `enumerate` / `iter` | (b) same |
| I3 | `extract.rs:815-834` | builtin stub files `builtins.pyi`, `functools.pyi`, `contextlib.pyi`, `_contextvars.pyi` | (b) replaced by `argument_effects` qualnames |
| I4 | `extract.rs:1354-1359` | `callback` / `run` call their first argument | (b) same |
| I5 | `extract.rs:884`, `:1360-1366` | `iter(x)` is `x.__iter__()` only | (b) same (`__iter__` is language) |
| I6 | `extract.rs:257` | file must mention `StreamingResponse` for the streaming pre-pass | (a) or (b), as above; the text gate goes either way |
| I7 | `extract.rs:1974-1980` | callee is exactly `starlette.responses.StreamingResponse` | (a) / (b) as above |
| I8 | `extract.rs:2802` | `content=` keyword is the streamed iterable | (a) / (b) as above |

### Group A: running-loop probes (asyncio)

**New concept `[[loop_probe]]`**: functions that raise when no event loop runs in the thread,
with the exception class. Handlers catching it (or a base, checked through ty's class
hierarchy) never run on the loop. Libraries: **asyncio** `get_running_loop` (`RuntimeError`)
and **sniffio** `current_async_library` (`AsyncLibraryNotFoundError`).

| ID | file:line | What it does | Class |
|---|---|---|---|
| A1 | `extract.rs:2436-2452` | `get_running_loop` resolved in `_asyncio` / `asyncio.events` | (b) `[[loop_probe]]` |
| A2 | `extract.rs:3086-3096` | `RuntimeError`, `Exception`, `BaseException` handlers | (a) **catches = the probe's exception or one of its bases**, from ty's MRO; no names |

### Group C: context-manager decorators (contextlib)

**New concept `[[generator_context]]`**: decorators that turn a generator function into a
context-manager factory. Calling it runs nothing; entering runs the body up to `yield`.
Libraries: **contextlib** `contextmanager` / `asynccontextmanager` and **async_generator**
`asynccontextmanager` (`contextlib2` too).

| ID | file:line | What it does | Class |
|---|---|---|---|
| C1 | `index.rs:543-551` | `contextmanager` resolved to `contextlib` makes a generator | (b) `[[generator_context]]` |
| C2 | `index.rs:267` | `asynccontextmanager` lifespans (S4) | (b) same |

### Group G: generic code that only names the language

These are not library rules. They are listed because their literals show up in `literals.py`.

| ID | file:line | What it does | Class |
|---|---|---|---|
| G1 | `extract.rs:540`, `:1933`, `:2214`; `index.rs:714`, `:849` | `self` / `cls` as receiver names | (a) **the method's first parameter** (`receiver_name` already exists): no names |
| G2 | `index.rs:772` | `staticmethod` / `classmethod` | (c) builtins decorators that define the receiver: language |
| G3 | `extract.rs:1048-1100`, `:1341`, `:2513` | `__enter__` / `__iter__` / `__init__` / `__name__` dunders | (c) Python data model, not a library |
| G4 | `analysis.rs:1666-1687` (`inherited_sink`), `:1200-1279` (`loop_self_calls`, `template_hooks`), `index.rs:727-800` (`self_calls`, `closure_self_calls`) | override and template-method dispatch into library classes | general, no names; kept. Only the doc comment names sqlmodel |
| G5 | `analysis.rs:1075`, `:1078`, `:1094`, `:1117`, `:1120`, `:1142`, `:1148`, `:1172`, `:1189`; `extract.rs:2021`; `index.rs:982`, `:987` | diagnostic labels and `expect` messages | not rules |
| G6 | `analysis/specialize.rs` | per-instance copies | general, kept; to be made cheap (section 2) |

Count: G1 is counted as (a); G2 and G3 as (c). G4–G6 are not counted.

### Catalog concepts the branch already added

| Concept | Verdict |
|---|---|
| `unless_given` / `blocks_in` (sink fields) | General. `email.utils.make_msgid(domain=)`; the second user is any sink with a bypass argument, for example `socket.create_connection` with a numeric host skipping DNS. Keep |
| `[inspects]` | General: functools, inspect, asyncio. Keep |
| `[log_dispatch]` | Logging only: becomes `[[observer]].dispatch` |

## 4. Items for your decision

1. **(c) G2, G3 and the dunders**: language, not library. Please approve.
2. **Logger names and hierarchy (L14, L15)**: the dotted-name hierarchy with a stop flag
   (`propagate`) is used only by stdlib logging. My recommendation is (b): optional
   `observer.scope` fields (naming function, root aliases, separator, stop attribute), with
   generic Rust (hierarchical channel names). It is needed for 0 FP:
   `fp3_log_handler_other_logger` (train) and the fixtures `logging_logger_hierarchy` /
   `logging_levels` depend on it. The alternative, (c), would keep it in Rust.
3. **S3 (variable-name owners)**: removing it may cost recall on train cases with untyped
   `request` / `app`. If a train case needs it, I will stop and ask, not keep the names.
4. **`file_bytesio_binaryio` (known FP)**: plan (a) **exact argument classes**. When every call
   site passes an exact class (`io.BytesIO()`), a call on the parameter dispatches on that
   class, the same way specialization narrows `self`. No allow-list, and no catalog entry
   unless (a) fails.

Rules that existed before `f02284a` are out of scope unless you say otherwise:

- `VALIDATES` (`model_validate`, ...);
- pydantic validator decorators;
- `model_post_init`;
- `Protocol`;
- `property` / `cached_property`;
- `enter_context` / `closing` / `callback`;
- the `register` attribute;
- the `.venv` / `site-packages` path names.

## 5. Phase B order (one mechanism per commit; push after each)

| # | Commit | Rules | Expected warm |
|---|---|---|---|
| 1 | perf: copies as an overlay, not `facts.clone()` | – | −120 to −150 ms |
| 2 | perf: memoized class hooks; per-function catalog role flags | – | −60 to −100 ms |
| 3 | `[[argument_effects]]` | I1–I5 | ~0 |
| 4 | `[[loop_probe]]` + catches by MRO | A1–A2 | ~0 |
| 5 | `[[generator_context]]` | C1–C2 | ~0 |
| 6 | `[[model_hooks]]` + element classes from ty types | P1–P12 | ~0 / faster |
| 7 | `[[entry]]` slots / `[[builds_on_loop]]` / `[[dependency_marker]]` + constructor self-calls | F1–F11 | ~0 |
| 8 | exact class of library-constructed class arguments (fixes the 2 known failures) | – | small + |
| 9 | `[[state_namespace]]`, types only for owners | S1–S4 | ~0 |
| 10 | offload through library parameter forwarding (or `[offload].iterates`) | I6–I8 | ~0 |
| 11 | `[[lazy_attribute]]` / `[[attribute_set_hooks]]` | O1–O7 | ~0 |
| 12 | `[[observer]]` | L1–L17 | −20 to −40 ms |
| 13 | receivers by first parameter; exact argument classes (`file_bytesio_binaryio`) | G1 | ~0 |
| 14 | README "how it works" short (mechanisms), detail in `docs/GUIDE.md`; record the round | – | – |

After each commit:

- train 100% except the 2 known failures (until commit 8);
- 0 other FP;
- `cargo test --workspace` green;
- warm Polar no worse than +10%.

Target: warm < 500 ms after commit 2, and it stays there. Done when `literals.py f02284a HEAD`
passes and the "other literals" list has no library names except the approved (c) items.
