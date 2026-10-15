# Using hydroid

hydroid finds code that blocks the asyncio event loop in FastAPI apps: a `time.sleep`, a
`requests.get`, a sync database query… anywhere below an `async def`, however deep.

## 1. Install

From PyPI with [uv](https://docs.astral.sh/uv/getting-started/installation/) (prebuilt wheels for
macOS, Linux, Windows; no Rust needed):

```sh
uvx --from hydroid-cli hydroid .
```

Install the `hydroid` command permanently:

```sh
uv tool install hydroid-cli
hydroid .
```

Without uv: `pipx install hydroid-cli` or `pip install hydroid-cli`.

**From source** (latest `main`; needs a Rust toolchain, `curl https://sh.rustup.rs -sSf | sh`;
the first build takes about a minute, then it is cached):

```sh
uvx --from git+https://github.com/agusmdev/hydroid hydroid .
uv tool install git+https://github.com/agusmdev/hydroid      # permanent
```

From a clone, `cargo install --path crates/hydroid` also works.

## 2. Prepare the project to check

hydroid resolves imports against the project's installed dependencies, like a type checker. With uv:

```sh
cd my-fastapi-app
uv sync            # creates .venv with your dependencies
```

hydroid finds the environment the same way ty does: `VIRTUAL_ENV`, an active Conda env,
`<project>/.venv`, then `python3` on `PATH`. Point it elsewhere with `--python path/to/.venv`.

## 3. Run it

```sh
hydroid .                      # the project root (where your package lives)
hydroid backend --python .venv # project in a subdirectory, venv elsewhere
```

Exit status: `0` nothing found, `1` blocking calls found, `2` hydroid could not run.

## 4. Read a finding

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

- `-->` is **where to fix it**: the call inside code running on the event loop (`async def`, or a
  sync callback the loop runs).
- `reached from` is the FastAPI entry point (route, dependency, middleware, …) that gets there.
- `blocking chain` walks from that call down to the function that blocks.
- `help` suggests the non-blocking alternative.

Typical fixes:

```python
# before
async def get_user(user_id: int):
    return services.load(user_id)

# 1. move the sync work to a thread
async def get_user(user_id: int):
    return await asyncio.to_thread(services.load, user_id)

# 2. or make the endpoint sync: FastAPI runs `def` endpoints in a threadpool
def get_user(user_id: int):
    return services.load(user_id)

# 3. or use an async library all the way down (httpx.AsyncClient, AsyncSession, redis.asyncio)
```

## 5. What counts as "on the event loop"

| Runs on the loop (checked) | Runs in a thread (not reported) |
|---|---|
| `async def` endpoints, dependencies, middleware, exception handlers, websockets | `def` endpoints and `def` dependencies |
| `lifespan`, startup/shutdown handlers (also `def` ones) | sync background tasks |
| async background tasks | `asyncio.to_thread`, `run_in_threadpool`, `loop.run_in_executor`, `anyio.to_thread.run_sync` |
| callbacks given to `loop.call_soon`, `call_later`, `add_done_callback` | `ThreadPoolExecutor.submit`, `threading.Thread(target=...)` |
| pydantic validators of request parameters and bodies (also for `def` endpoints and dependencies), nested models and `Annotated[T, AfterValidator(f)]` fields included | |
| `@computed_field`, `@field_serializer`, `@model_serializer` and `Annotated[T, PlainSerializer(f)]` fields of the returned model (every endpoint; not the fields in a literal `response_model_exclude`) | |
| validators of the `response_model` (or return annotation, unless `response_model=None`) of `async def` endpoints | validators of the `response_model` of `def` endpoints |
| validators of the models nested in the fields a construction (`Model(items=[{...}])`) or `Model.model_validate(...)` gives data to, and serializers run by `model.model_dump()` / `model_dump_json()` | nested validators of fields given model instances (`Model(items=[item])`, a variable declared `list[Item]`), which pydantic does not validate again; serializers of the fields a literal `exclude=` / `include=` leaves out; `when_used="json"` serializers in python-mode dumps (`model_dump()` without `mode="json"`) |
| the constructor of a middleware class (`app.add_middleware(Cls, ...)`, `Middleware(Cls, ...)`), which Starlette calls when the first lifespan or request event builds the stack | |
| `render`, `__init__`, `init_headers` of a `response_class=` / `default_response_class=` (every endpoint) | the response validation, serialization and `response_class=` of endpoints that always return a `Response` instance, which FastAPI sends as is |
| `filter`, `emit`, `format` of the project's logging filters, handlers and formatters, run by `logger.info(...)` and above on the logger they are attached to or below it (and rotating/watched file and socket handlers' I/O) | handlers attached to a `QueueListener` |

Awaiting an API is never blocking: `await redis.asyncio.Redis().get(k)` is fine,
`redis.Redis().get(k)` is reported.

## 6. Options

| Flag | `pyproject.toml` key | |
|---|---|---|
| `--python PATH` | `python` | virtualenv (or interpreter) to resolve imports |
| `--follow-libs` | `follow-libs` | also analyze installed libraries' code (finds blocking inside dependencies) |
| `--cpu` | `cpu` | also report CPU-heavy calls (bcrypt, pbkdf2, scrypt) |
| `--strict` | `strict` | list calls hydroid could not resolve, and fail on them |
| `--exclude GLOB` | `exclude` | skip files, e.g. `migrations/**` (repeatable) |
| `--no-cache` | | do not reuse or save extracted facts in `.hydroid_cache/` |
| `--format human\|json\|sarif` | | output format |

Flags win over `pyproject.toml`:

```toml
[tool.hydroid]
python = ".venv"
exclude = ["migrations/**", "scripts/**"]
follow-libs = false
strict = false
```

## 7. Teach it about your own blocking functions

Add sinks (by the **defining** module path of the function) in `pyproject.toml`:

```toml
[[tool.hydroid.sink]]
category = "sdk"
advice = "use AcmeAsyncClient"
functions = [
    "acme.client.Client.fetch",
    "acme.client.Client.*",       # `*` matches within one dotted segment
]

[[tool.hydroid.blocking_decorator]]   # sync functions decorated with these (or wrapped by hand) block when called
category = "retry"
advice = "decorate a coroutine instead"
functions = ["mylib.retry.retrying"]

[tool.hydroid.offload]                 # callables passed to these run in a thread
functions = ["mylib.jobs.run_in_worker"]
```

The builtin list lives in [`crates/hydroid_core/catalog.toml`](../crates/hydroid_core/catalog.toml).
Every table below can be added to from `pyproject.toml` as `[tool.hydroid.<table>]` (or
`[[tool.hydroid.<table>]]`). Names are the **defining** qualified names (`module.Class.method`);
a class name stands for constructing the class.

| Table | Says | Fields |
|---|---|---|
| `[[sink]]` | these functions block | `category`, `advice`, `functions`; `direct` (only calls made directly from loop code count), `opt_in` (only with `--cpu`), `unless_given` (keyword arguments that make a call skip the blocking work), `blocks_in` (the function to name in diagnostics) |
| `[[blocking_decorator]]` | sync functions decorated with these block | as `[[sink]]` |
| `[offload]` | callables passed to these run in a worker thread | `functions`; `copies` (those that may run it in another process, on a pickled copy: an exit stack's `close` handed there leaves the original full) |
| `[loop_callback]` | callables passed to these run on the event loop later | `functions` |
| `[inspects]` | callables passed to these are only inspected, never called | `functions` |
| `[[arguments]]` | what these functions do with their arguments, in the calling thread | `functions`, and any of: `calls` (callables given there are called), `iterates` (iterables given there are iterated), `enters` (context managers given there are entered and exited), `methods` (methods called on the first positional argument), `lazy` (the call returns a lazy iterator: iterating it iterates what `iterates` names, and what `calls` names runs where it is consumed); for calls declaring a field in a class body, `factory` (constructing the class without the field calls the callable given there) and `names` (string literals giving other names of the field) |
| `[[hook]]` | these decorators make the method they decorate a hook its class runs when validated (`runs = "validation"`: constructing it, or validating data into it), dumped (`runs = "dump"`), or when the fields it names are set on an instance (`runs = "set"`: `obj.email = ...`, constructor keywords included); with `callable`, these classes or functions, called in the `Annotated[...]` metadata of a field, attach the callable given at that slot to the field | `runs`, `functions`; `callable`, `fields` (`"arguments"`: the fields the decorator's call names, `"name"`: the field named like the decorated function), `json_only` (`{ keyword, values }`: a dump hook only runs for JSON dumps when its call gives the keyword one of these values) |
| `[[runs_hooks]]` | these operations run the `runs` hooks of the class they are called on (a class object or an instance) | `runs`, `functions`; for dumps `json`, `json_unless` (`{ keyword, values }`: given, the dump is to JSON unless the value is one of these), `exclude` and `include` (keywords naming top-level fields left out, or the only ones dumped) |
| `[[loop_probe]]` | these functions raise `raises` (a class) when no event loop runs in the calling thread: the handlers catching it (or a base) of a `try` doing nothing but call them never run on the loop | `raises`, `functions` |
| `[[entry]]` | callables passed to these are framework entry points | `kind`, `functions`; `slot` (the parameters taking what is registered; a decorator registers what it decorates; default: any argument); what the framework runs on the loop around it: `validates_params` (validates its annotated parameters, or those of a class's `__init__`), `serializes_return` (dumps what it returns, sync callables included), `validates_return` (validates what it returns into the response model, in the thread it runs in: only `async def` ones on the loop); `response_model` (the keyword giving the response model, `None` for none; default: the return annotation), `response_exclude` (the keyword naming top-level fields left out of the dump), `passthrough` (classes sent as is: a callable always returning them skips the return hooks); `injects` (a parameter whose default, or `Annotated` metadata, is a call to these is filled with what the callable at the first `slot` provides, and not validated from the request) |
| `[[builds_on_loop]]` | a class (or a factory) given to these at `slot` is constructed (called) by the library later, on the loop: its `__init__` runs there, with the project overrides of what the library `__init__` it inherits calls on `self` (`Response.__init__` calls `self.render`) | `kind` (what running it is called in diagnostics), `slot`, `functions`; `label` (how diagnostics name what reaches it; default: the function's `[[entry]]`), `unless_passthrough` (not when every callable the same call registers always returns one of its entry's `passthrough` classes) |
| `[[lazy_attribute]]` | `name = factory(...)` in a class body makes reading `instance.name` load it, running `loads`, unless it is already loaded (SQLAlchemy's `relationship()`) | `factories`, `loads`; `keyword` and `lazy` (the factory's keyword choosing how it loads, and its literal values that load on read; without the keyword, it does), `eager` (options loading the attributes they name up front), `expires_all` and `expires` (calls expiring every loaded attribute, or those of the instance given first), `reload` (`{ functions, names }`: calls loading the attributes named at `names` on the instance given first; given none, they expire it), `raises_under` (classes under which a load raises instead of blocking: a function naming one loads nothing) |
| `[[observer]]` | a registry: `attach` calls attach observers (instances of `classes`, or callables) to the registry they are called on, and its `dispatch` calls run them on the calling thread (stdlib logging handlers, filters and formatters; blinker receivers) | `dispatch`; `attach` (`{ functions, propagates, callable }`), `classes` (`{ base, runs, calls, exception_only, level, offloaded_by }`: what runs on an attached instance: `runs`, or what the library's methods call on `self` where the project does not override them), `scope` (`{ naming, keyword, root, separator, stop }`: how registries are named; observers only run for the registries they are attached to); logging only: `levels`, `exception`, `level_names`, `level_keyword`, `default_level`, `threshold`, `any_level` |
| `[[exit_stack]]` | calls storing a callable (`callbacks`), or a context manager's exit (`contexts`: entered now), on the instance they are called on, which runs what it stores later, where `runs` calls run it (and where a `with` block on it ends, when its `__exit__` is one); `moves` calls move what it stores to a new instance (`pop_all()`). Where hydroid cannot follow the instance to where it runs, the call does what `[[arguments]]` says | `callbacks`, `contexts`, `runs`, `moves` |
| `[[state_namespace]]` | an attribute namespace the library types as `Any` (`app.state`, Flask's `g`): reading `<namespace>.name` gets the class of what the project stores under `name` there, when every store (and every key of a lifespan's yielded dict) agrees | `owners` (classes whose instances own one); `attribute` (the owners' attribute holding it; without it, the owner itself); `app` (`{ attribute, owners }`: `<owner>.<attribute>` is the application, typed as the one project subclass of `owners`); `lifespan` (`{ owners, keyword }`: the dicts yielded by the lifespan given at `keyword` to the constructor of `owners` go into the namespace) |

Argument slots (`calls`, `iterates`, `enters`) are positions (`0` is the first argument after
`self`), keyword names, or `"*"` (every positional argument not in `calls`). For example, a
library whose sorted collections consume the iterable they are given and call their `key`:

```toml
[[tool.hydroid.arguments]]
iterates = [0, "iterable"]
calls = [1, "key"]
functions = ["sortedcontainers.sortedlist.SortedList", "sortedcontainers.sortedset.SortedSet"]
```

## 8. Silence a finding

```python
time.sleep(0.01)  # hydroid: ignore
```

The comment goes on the line hydroid reports (the `-->` line).

## 9. Unresolved calls

Python is dynamic; some calls cannot be resolved statically (`getattr(obj, name)()`, untyped
parameters, callables stored in dicts). hydroid never drops them silently: the summary line counts
them, and `--strict` lists each one reachable on the loop and exits `1`. Adding type annotations
usually resolves them.

## 10. CI (GitHub code scanning)

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
      - run: uv sync
      # Optional: pin a version for reproducible builds, e.g. hydroid-cli==0.2.0
      - run: >-
          uvx --from hydroid-cli
          hydroid . --format sarif > hydroid.sarif || [ $? -eq 1 ]
      - uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: hydroid.sarif
```

Each blocking chain shows up as a code flow on the pull request. Drop `|| [ $? -eq 1 ]` and the
upload step to simply fail the build instead.

GitLab, pre-commit, monorepos, adopting it on a project with existing findings, and a prompt that
has a coding agent set all of this up for you: [AGENT_SETUP.md](AGENT_SETUP.md).

## 11. JSON output

`--format json` prints the full report: `diagnostics` (location, function, chain, sink,
reached_from), `unresolved`, and `stats` (files, functions, call sites resolved, timings).

## 12. How it works

### Facts

Every function body is walked in parallel, and each call is resolved through ty (types, then
go-to-definition) to the functions it can run. Implicit calls are included:
- `with`, `for` and comprehensions;
- properties and their setters, descriptors, `__getattr__`;
- operators like `x[k]` and `k in x` on project classes, and iterators' `__next__`;
- constructors, with `__post_init__`, model validators and the default factories of fields the
  call does not give (`[[arguments]]` `factory`).

The catalog adds what library code does with its arguments (`[[arguments]]`), such as
`sorted(xs, key=f)` or `copy_context().run(f)`. Callables passed as
arguments are recorded as flows (`to_thread(f)`, `Depends(dep)`, `on_startup=[f]`,
decorators), and so are callables stored on instances and returned by functions. Calls through
dispatch tables (`TABLE[key](...)` on a dict, list or tuple literal) resolve to the callables
stored there, only the one under `key` when the key is a literal. When ty does not know a
value's type because it comes from an unannotated function, the class that function returns is
used.

Storing a callable or a context manager's exit on an exit stack (`[[exit_stack]]`:
`stack.callback(f)`, `stack.enter_context(cm)`) does not run it: it runs where the stack is run,
`stack.close()` or the end of a `with` block on it, or wherever `stack.close` is handed to
(`await asyncio.to_thread(stack.close)` runs it in a worker thread), on the stack `pop_all()`
moved it to. The stack is followed only in a local variable of the function creating it, and
only when every statement of that function touching it has one of the shapes modeled exactly:
storing, closing or moving it as a statement of its own, handing its sync `close` to a function
the catalog says calls what it is given later (`[offload]`, `[loop_callback]`, `[[entry]]`),
`if` statements, a sync `with` block on the stack alone, and `try` ...
`finally` without handlers (the `finally` runs on every way out: normal end, `return`,
exception). Exceptions and `return` leave through the `with` blocks and `finally` clauses
around them. Every statement may raise, but for literals, names bound on every path, reads of
an attribute a plain project class always assigns in its `__init__`, and f-strings formatting
those as `str`, `int` or `float` without a spec. An awaited hand-off,
`await asyncio.to_thread(stack.close)` (the awaited call itself, not one nested in it), has run
the stack once the await returns. Any other use
(a loop, `except` handlers, a `with` on another manager, an async stack, rebinding the stack's
name while it holds stores, a builtin function such as `locals()` that can reach the frame,
handing it to another function, returning or storing it) leaves the stack unfollowed: its store
calls do what their `[[arguments]]` entry says, so a new library declares both,
`[[exit_stack]]` for where its stores run and `[[arguments]]` (`calls`, `enters`) for reporting
them where they are stored when its stacks are not followed; without `[[arguments]]`, those
stores run nothing. A stack that is a project subclass of a stack class is not followed either,
and neither is a function calling a builtin that can reach its frame (`locals()`, `exec`).

Objects read back from a namespace the library types as `Any` (`[[state_namespace]]`:
`app.state.x`, `request.state.x`, a lifespan's `yield {"x": ...}`) get the class the project
stores under that name, when every store agrees. Signatures record what a framework validates
and serializes around an entry point (`[[entry]]`): the models in parameter annotations, the
response model or return annotation, the classes returned, and whether every return gives a
`passthrough` class. Model fields record their nested models and their `Annotated` validators
and serializers (`[[hook]]`).

The facts of an unchanged project are reused from `.hydroid_cache/`, keyed by the binary, the
options, the installed packages and every source file's contents.

### Analysis

Where code runs is a property of the call, so "does calling `f` block?" is computed once per
function by a backwards search from blocking functions: shortest witness chains, no recursion,
any depth. Diagnostics are the call sites inside loop code whose callee blocks. Loop code is:
- `async def` bodies;
- sync callbacks scheduled on the loop (`[loop_callback]`);
- what a framework runs on the loop around what is registered with an `[[entry]]`: validating
  the parameters (sync endpoints included, only their bodies go to the threadpool), validating
  and dumping the return value, and constructing the classes it builds later
  (`[[builds_on_loop]]`, like a `response_class` and the self-calls of its `__init__`).

Offload APIs cut the graph, and callbacks are attributed to the call site passing them. A lazy
iterable handed to a library function that passes it on to an offload API is consumed in the
worker (Starlette's `StreamingResponse(content)`), as is one given to an offload API with its
consumer (`to_thread(list, map(f, xs))`).

Overrides are found by class-hierarchy analysis (test doubles only stand in for real classes in
test code), narrowed by what is known:
- a construction (`Model().refresh()`, or a variable only ever assigned constructions) runs the
  methods its class's MRO finds;
- a function given such an instance as an argument (`checksum(io.BytesIO(data))`) is analyzed
  for it, so `stream.read()` in it is `BytesIO.read`;
- the type arguments of a generic class (`TagService(BaseService[TagRepo])`) pick the
  implementation behind its type-variable attributes.
Methods are analyzed once per such instance.

A library `async def` method running on an instance of a project subclass
(`Security(scheme)`) runs the project's overrides of the library methods it calls on `self`,
without `--follow-libs`. So does a project callable assigned over a library method on an
instance (`app.openapi = custom_openapi`).

Calling a generator function (or a decorated one whose call ty types as a context manager or an
iterator) runs nothing until it is iterated (entered). A decorated function is reached through
its decorator's wrappers only when they call it.

Some library code is described in the catalog, because its source does not show it:
- Observers (`[[observer]]`): a logging call runs the handlers, filters and formatters the
  project attaches, only for the loggers they are attached to and the records their levels let
  through.
- Lazily loaded attributes (`[[lazy_attribute]]`): reading a lazy SQLAlchemy relationship
  queries, unless an eager option loaded it, an async session holds it, or it was already
  loaded off the loop.
- Hooks run on attribute set (`[[hook]]`, `runs = "set"`): SQLAlchemy's `@validates`.

Code in the `except RuntimeError:` of a `try` that only probes for a running loop
(`[[loop_probe]]`) runs only when no loop is running: it is skipped. Starting an event loop
(`asyncio.run`) is only reported when loop code does it directly.

### Known limits

Python is dynamic: calls through `getattr`, untyped parameters, or containers of callables cannot
always be resolved. They are never silently dropped: `--strict` lists every unresolved call
reachable on the loop. Not modeled yet:
- `getattr(obj, "name")` and containers of callables (`HANDLERS[kind]()`);
- metaclasses;
- `.pyi`-only libraries under `--follow-libs`.
- Exit stacks: an exit that runs on the loop only when an unexpected exception is raised between
  storing it and an offloaded close (a property or `__format__` raising, an attribute deleted
  where hydroid does not see it) may go unreported; so may a stack reached through
  `sys._getframe()` or `inspect`; `stack.push(cm)` with a context manager object stores its
  `__exit__`, which hydroid does not model.

Classes never instantiated: a virtual call on a base class only dispatches to the overrides of
subclasses that can have instances. A class can have instances if:
- project code names it;
- it is decorated or given class keywords (a registry may create it);
- it is a `Protocol`;
- an ancestor defines `__init_subclass__`.
A class created only by name lookup (`getattr(module, name)()`, `importlib`, a string registry)
looks never instantiated, and its overrides are missed: a recall limit, not a false positive.

A change to any source file re-extracts the whole project (no per-file incremental cache yet).
