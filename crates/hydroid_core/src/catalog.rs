//! What hydroid knows about functions it does not analyze: which block, which move work off the
//! event loop, which schedule callbacks on it, and which register framework entry points.

use anyhow::Context;
use rustc_hash::FxHashMap as HashMap;
use serde::Deserialize;

use crate::facts::Slot;
use crate::report::{Severity, Sink};

const BUILTIN: &str = include_str!("../catalog.toml");

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    #[serde(default)]
    sink: Vec<SinkGroup>,
    /// Decorators that make calls to the sync functions they decorate block (retry loops).
    #[serde(default)]
    blocking_decorator: Vec<SinkGroup>,
    #[serde(default)]
    offload: OffloadGroup,
    #[serde(default)]
    loop_callback: Group,
    /// Functions that only inspect the callables passed to them (`functools.wraps(func)`).
    #[serde(default)]
    inspects: Group,
    /// Registries running the observers attached to them on the calling thread.
    #[serde(default)]
    observer: Vec<Observer>,
    /// What library functions do with their arguments during the call.
    #[serde(default)]
    arguments: Vec<ArgumentGroup>,
    /// Functions raising when no event loop runs in the calling thread.
    #[serde(default)]
    loop_probe: Vec<LoopProbeGroup>,
    /// What makes a function a hook that validating or dumping a model runs.
    #[serde(default)]
    hook: Vec<HookGroup>,
    /// Operations running the hooks of the class they are called on.
    #[serde(default)]
    runs_hooks: Vec<RunsHooksGroup>,
    #[serde(default)]
    entry: Vec<EntryGroup>,
    /// Functions taking a class (or a factory) the library calls later, on the loop.
    #[serde(default)]
    builds_on_loop: Vec<BuildsOnLoopGroup>,
    /// Objects whose attributes hold what project code stores on them.
    #[serde(default)]
    state_namespace: Vec<StateNamespace>,
    /// Class attributes whose first read loads them (I/O).
    #[serde(default)]
    lazy_attribute: Vec<LazyAttribute>,
    /// Objects storing callables and context managers' exits that they run later.
    #[serde(default)]
    exit_stack: Vec<ExitStack>,
}

/// `[[exit_stack]]`: calls storing a callable, or the exit of a context manager, on the
/// instance they are called on (they do not run it), and the calls running what it stores:
/// `contextlib.ExitStack`, `unittest.TestCase` cleanups.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitStack {
    /// Calls storing the callable given first.
    #[serde(default)]
    pub callbacks: Vec<String>,
    /// Calls entering the context manager given first, and storing its `__exit__`.
    #[serde(default)]
    pub contexts: Vec<String>,
    /// Calls running everything stored (leaving a `with` block runs it when its `__exit__` or
    /// `__aexit__` is one).
    #[serde(default)]
    pub runs: Vec<String>,
    /// Calls returning a new instance holding everything stored, leaving the original empty.
    #[serde(default)]
    pub moves: Vec<String>,
}

/// `name = factory(...)` in a class body makes reading `instance.name` load it, running `loads`
/// (SQLAlchemy's `relationship()`), unless it is already loaded.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LazyAttribute {
    pub factories: Vec<String>,
    /// The factory's keyword choosing how the attribute loads.
    #[serde(default)]
    pub keyword: Option<String>,
    /// The literal values of `keyword` that load on read; without the keyword, it does.
    #[serde(default)]
    pub lazy: Vec<LiteralValue>,
    /// What a load runs.
    pub loads: String,
    /// Options loading the attributes they name up front (`selectinload(Author.books)`).
    #[serde(default)]
    pub eager: Vec<String>,
    /// Calls expiring every loaded attribute (`commit()`).
    #[serde(default)]
    pub expires_all: Vec<String>,
    /// Calls expiring the loaded attributes of the instance they are given first.
    #[serde(default)]
    pub expires: Vec<String>,
    /// Calls loading, on the instance given first, the attributes named (in a literal list,
    /// tuple or set) at `names`; given no names, they expire it.
    #[serde(default)]
    pub reload: Option<Reload>,
    /// Classes under which a load raises instead of blocking (async sessions): a function
    /// naming one (in an annotation, or a value of one) loads nothing.
    #[serde(default)]
    pub raises_under: Vec<String>,
}

/// A literal argument value.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum LiteralValue {
    Bool(bool),
    Str(String),
}

/// See [`LazyAttribute::reload`].
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reload {
    pub functions: Vec<String>,
    pub names: Vec<ArgumentSlot>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildsOnLoopGroup {
    kind: String,
    slot: Vec<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    unless_passthrough: bool,
    functions: Vec<String>,
}

/// A class (or a factory) given to the function at `slot` is constructed (called) by the
/// library later, on the loop: its `__init__` runs there, with the project overrides of what
/// the library `__init__` it inherits calls on `self`.
#[derive(Clone, Debug)]
pub struct BuildsOnLoop {
    /// What running it is called in diagnostics (`construction`).
    pub kind: String,
    /// The parameters taking the class (or factory).
    pub slot: Vec<String>,
    /// How diagnostics name what reaches it; by default, the `[[entry]]` of the function.
    pub label: Option<String>,
    /// Not when every callable the same call registers always returns one of its entry's
    /// `passthrough` classes (sent as they are).
    pub unless_passthrough: bool,
}

/// A registry: calls attach observers (objects of `classes`, or callables) to it, and its
/// `dispatch` calls run them on the calling thread (stdlib `logging`: handlers, filters and
/// formatters attached to loggers; blinker: receivers connected to signals).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observer {
    /// Calls running the observers attached (to the registry they are called on, or to the
    /// root one for a module-level function).
    pub dispatch: Vec<String>,
    /// Calls attaching what they are given first to the registry (or observer) they are called
    /// on.
    #[serde(default)]
    pub attach: Vec<Attach>,
    /// Observer classes, and what a dispatch runs on their instances.
    #[serde(default)]
    pub classes: Vec<ObserverClass>,
    /// How registries are named: an observer runs for the records of the registries it is
    /// attached to (any, when not known).
    #[serde(default)]
    pub scope: Option<ObserverScope>,
    /// The level of what each dispatch (by its last name) dispatches; the others take it as
    /// their first argument (or at `level_keyword`).
    #[serde(default)]
    pub levels: HashMap<String, u32>,
    /// Dispatches (by last name) whose records carry an exception.
    #[serde(default)]
    pub exception: Vec<String>,
    /// Level names, as strings or module attributes (`logging.ERROR`).
    #[serde(default)]
    pub level_names: HashMap<String, u32>,
    /// The keyword giving a level.
    #[serde(default)]
    pub level_keyword: Option<String>,
    /// The level below which dispatches run nothing, unless the project sets a lower one.
    #[serde(default)]
    pub default_level: Option<u32>,
    /// Calls setting that level (their first argument, or `level_keyword`).
    #[serde(default)]
    pub threshold: Vec<String>,
    /// Calls that may set it to anything (configuration files).
    #[serde(default)]
    pub any_level: Vec<String>,
}

/// Calls attaching an observer.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attach {
    pub functions: Vec<String>,
    /// Attached to a registry, it also runs for the registries below it (see
    /// [`ObserverScope::separator`]).
    #[serde(default)]
    pub propagates: bool,
    /// A plain callable may be attached: dispatches call it.
    #[serde(default)]
    pub callable: bool,
}

/// Observers of a class (and its subclasses): what a dispatch runs on them.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserverClass {
    pub base: String,
    /// The method the registry calls.
    pub runs: String,
    /// What the library's methods call on `self` (the stdlib is stub-only): a method the
    /// project does not override runs these instead.
    #[serde(default)]
    pub calls: HashMap<String, Vec<String>>,
    /// Methods only run for records carrying an exception.
    #[serde(default)]
    pub exception_only: Vec<String>,
    /// Calls setting the level of an observer (on it, or in its own constructor), below which
    /// it is skipped.
    #[serde(default)]
    pub level: Vec<String>,
    /// Classes whose instances, once constructed, run these observers in their own thread.
    #[serde(default)]
    pub offloaded_by: Vec<String>,
}

/// How registries are named (`logging.getLogger("audit")`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserverScope {
    /// Calls returning the registry named by their first argument (or `keyword`); with no
    /// name, the root.
    pub naming: Vec<String>,
    #[serde(default)]
    pub keyword: Option<String>,
    /// A name (and a module attribute) standing for the root registry.
    #[serde(default)]
    pub root: Option<String>,
    /// Names are hierarchical: `a.b` is below `a`, and everything is below the root.
    #[serde(default)]
    pub separator: Option<String>,
    /// An attribute that, set `False` on a registry, keeps records from going up from it.
    #[serde(default)]
    pub stop: Option<String>,
}

/// An attribute namespace the library types as `Any` (`app.state`): reading `<namespace>.name`
/// gets the class of what the project stores under `name` there.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateNamespace {
    /// Classes whose instances (and their subclasses') own one.
    pub owners: Vec<String>,
    /// The owner's attribute holding it; without it, the owner itself.
    #[serde(default)]
    pub attribute: Option<String>,
    /// An attribute of the owners giving the application (`request.app`).
    #[serde(default)]
    pub app: Option<AppAttribute>,
    /// What a lifespan given to an application's constructor yields is copied into it.
    #[serde(default)]
    pub lifespan: Option<Lifespan>,
}

/// `<owner>.<attribute>` is the application: an instance of `owners`, typed as the one project
/// subclass of them, if there is one.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppAttribute {
    pub attribute: String,
    pub owners: Vec<String>,
}

/// The string keys of a dict literal yielded by the lifespan given at `keyword` to the
/// constructor of `owners` (or of a subclass) go into the namespace.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lifespan {
    pub owners: Vec<String>,
    pub keyword: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SinkGroup {
    category: String,
    advice: String,
    /// Only a call made directly from code on the loop counts: sync functions calling these do
    /// not block because of them.
    #[serde(default)]
    direct: bool,
    #[serde(default)]
    opt_in: bool,
    /// Arguments that make a call skip the blocking work (`make_msgid(domain=...)` resolves no
    /// host name): only calls known to give none of them block, see
    /// [`crate::facts::Call::given`].
    #[serde(default)]
    unless_given: Vec<String>,
    /// What these functions block in, internally (`make_msgid` in `socket.getfqdn`):
    /// diagnostics name it as the blocking function.
    #[serde(default)]
    blocks_in: Option<String>,
    /// How long these functions can stop the loop (default: `critical`).
    #[serde(default)]
    severity: Severity,
    functions: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Group {
    #[serde(default)]
    functions: Vec<String>,
}

/// `[offload]`: functions running the callables given to them off the loop.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct OffloadGroup {
    #[serde(default)]
    functions: Vec<String>,
    /// Those that may run it in another process, on a copy (pickled): what it changes there
    /// leaves the original as it was.
    #[serde(default)]
    copies: Vec<String>,
}

/// `[[arguments]]`: functions doing the same with their arguments (see [`Arguments`]).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArgumentGroup {
    #[serde(default)]
    calls: Vec<ArgumentSlot>,
    #[serde(default)]
    iterates: Vec<ArgumentSlot>,
    #[serde(default)]
    enters: Vec<ArgumentSlot>,
    #[serde(default)]
    methods: Vec<String>,
    #[serde(default)]
    lazy: bool,
    #[serde(default)]
    factory: Vec<ArgumentSlot>,
    #[serde(default)]
    names: Vec<ArgumentSlot>,
    functions: Vec<String>,
}

/// What a function does with its arguments, in the thread calling it. A class stands for
/// constructing it.
#[derive(Clone, Debug, Default)]
pub struct Arguments {
    /// Callables given there are called (`sorted(xs, key=f)`, `Context.run(f)`).
    pub calls: Vec<ArgumentSlot>,
    /// Iterables given there are iterated (`list(xs)`; `"*"`: every positional argument not in
    /// `calls`).
    pub iterates: Vec<ArgumentSlot>,
    /// Context managers given there are entered, and exited (`stack.enter_context(cm)`).
    pub enters: Vec<ArgumentSlot>,
    /// Methods called on the first positional argument (`len(x)`: `__len__`).
    pub methods: Vec<String>,
    /// The call returns a lazy iterator: what `calls` and `iterates` say happens when it is
    /// iterated (`map(f, xs)`), not during the call.
    pub lazy: bool,
    /// The call declares a field in a class body (`x: list = Field(default_factory=list)`):
    /// constructing the class without giving the field calls the callable (or constructs the
    /// class) given there.
    pub factory: Vec<ArgumentSlot>,
    /// The string literals given there are other names the declared field can be given by
    /// (aliases).
    pub names: Vec<ArgumentSlot>,
}

/// When a hook runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookPhase {
    /// Constructing or validating an instance of the class (from data).
    Validation,
    /// Dumping an instance of the class.
    Dump,
    /// Setting the attributes it names on an instance (`obj.name = value`, the constructor's
    /// keyword arguments included).
    Set,
}

/// Which fields a decorator hook is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookFields {
    /// Those its decorator's call names as string arguments (`"*"`: every field).
    Arguments,
    /// The one named like the decorated function.
    Name,
}

/// A keyword argument whose literal values make a dump hook JSON-only.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeywordValues {
    pub keyword: String,
    pub values: Vec<String>,
}

/// `[[hook]]`: decorators marking the method they decorate as a hook of its class, or (with
/// `callable`) classes or functions which, in the `Annotated[...]` metadata of a field, attach
/// the callable given at that slot to the field.
#[derive(Clone, Debug)]
pub struct Hook {
    pub runs: HookPhase,
    pub callable: Vec<ArgumentSlot>,
    /// Without it, a decorator hook is for every field.
    pub fields: Option<HookFields>,
    /// Dump hooks only used when dumping to JSON.
    pub json_only: Option<KeywordValues>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookGroup {
    runs: HookPhase,
    #[serde(default)]
    callable: Vec<ArgumentSlot>,
    #[serde(default)]
    fields: Option<HookFields>,
    #[serde(default)]
    json_only: Option<KeywordValues>,
    functions: Vec<String>,
}

/// `[[runs_hooks]]`: an operation running the hooks of the class it is called on (a class
/// object, or an instance).
#[derive(Clone, Debug)]
pub struct RunsHooks {
    pub runs: HookPhase,
    /// A dump to JSON (JSON-only hooks run too).
    pub json: bool,
    /// A keyword choosing the dump mode: when it is given, the dump is to JSON unless its literal
    /// value is one of `values`.
    pub json_unless: Option<KeywordValues>,
    /// Keyword arguments naming the top-level fields left out of the dump, and the only ones
    /// dumped.
    pub exclude: Option<String>,
    pub include: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunsHooksGroup {
    runs: HookPhase,
    #[serde(default)]
    json: bool,
    #[serde(default)]
    json_unless: Option<KeywordValues>,
    #[serde(default)]
    exclude: Option<String>,
    #[serde(default)]
    include: Option<String>,
    functions: Vec<String>,
}

/// `[[loop_probe]]`: functions raising `raises` (a class) when no event loop is running in the
/// calling thread. The handlers catching it of a `try` that only makes such calls never run on
/// the loop.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoopProbeGroup {
    raises: String,
    functions: Vec<String>,
}

/// A positional argument (from 0, not counting `self`), a keyword argument, or `"*"`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ArgumentSlot {
    Position(usize),
    Keyword(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryGroup {
    kind: String,
    /// The parameters taking the callable (or class) registered; a decorator registers what it
    /// decorates. Without it, any argument.
    #[serde(default)]
    slot: Vec<String>,
    #[serde(default)]
    validates_params: bool,
    #[serde(default)]
    serializes_return: bool,
    #[serde(default)]
    validates_return: bool,
    #[serde(default)]
    response_model: Option<String>,
    #[serde(default)]
    response_exclude: Option<String>,
    #[serde(default)]
    passthrough: Vec<String>,
    #[serde(default)]
    injects: bool,
    #[serde(default)]
    runs_once: RunsOnce,
    functions: Vec<String>,
}

/// Which callables an `[[entry]]` function registers run only while the server starts or stops
/// (lifespan, startup and shutdown handlers): `true` for all of them, or the parameters taking
/// them.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum RunsOnce {
    All(bool),
    Slots(Vec<String>),
}

impl Default for RunsOnce {
    fn default() -> Self {
        Self::All(false)
    }
}

/// What registering a callable (or a class) with an `[[entry]]` function means.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    /// What is registered (`route`, `middleware`, ...): labels which entry reaches a diagnostic.
    pub kind: String,
    /// The parameters taking what is registered; a decorator registers what it decorates.
    /// Empty: any argument.
    pub slot: Vec<String>,
    /// The framework validates, on the loop, the annotated parameters of what is registered
    /// (of its `__init__`, for a class) from the data it calls it with.
    pub validates_params: bool,
    /// The framework dumps what the callable returns (against its response model), on the loop,
    /// whether the callable itself runs on the loop or in a thread.
    pub serializes_return: bool,
    /// The framework validates what the callable returns into its response model in the thread
    /// the callable runs in (the loop, for an `async def`); an instance of the model is kept as
    /// is.
    pub validates_return: bool,
    /// The keyword giving the response model (given `None`: no model). The return annotation
    /// otherwise.
    pub response_model: Option<String>,
    /// The keyword naming, in a literal set or dict, top-level fields left out of the dump.
    pub response_exclude: Option<String>,
    /// Classes sent as they are: a callable always returning instances of them (or of
    /// subclasses) has its return value neither validated nor dumped.
    pub passthrough: Vec<String>,
    /// A parameter whose default (or `Annotated` metadata) is a call to the entry function is
    /// injected: the framework fills it with what the callable registered at `slot` provides
    /// (what it returns or yields; an instance, for a class), not from the request.
    pub injects: bool,
    /// See [`RunsOnce`].
    pub runs_once: RunsOnce,
}

impl Entry {
    /// Whether what is registered at `slot` runs only while the server starts or stops.
    pub fn runs_once(&self, slot: &Slot) -> bool {
        match (&self.runs_once, slot) {
            (RunsOnce::All(all), _) => *all,
            (RunsOnce::Slots(slots), Slot::Param(p)) => slots.contains(p),
            (RunsOnce::Slots(_), _) => false,
        }
    }
}

/// Qualified-name patterns: exact names in a map, `*` patterns scanned.
struct Patterns<T> {
    exact: HashMap<String, T>,
    globs: Vec<(Vec<String>, T)>,
}

impl<T: Clone> Default for Patterns<T> {
    fn default() -> Self {
        Self { exact: HashMap::default(), globs: Vec::new() }
    }
}

impl<T: Clone> Patterns<T> {
    fn insert(&mut self, pattern: &str, value: T) {
        if pattern.contains('*') {
            self.globs.push((pattern.split('.').map(str::to_string).collect(), value));
        } else {
            self.exact.insert(pattern.to_string(), value);
        }
    }

    fn get(&self, qualname: &str) -> Option<&T> {
        if let Some(value) = self.exact.get(qualname) {
            return Some(value);
        }
        if self.globs.is_empty() {
            return None;
        }
        let segments = qualname.split('.').count();
        self.globs.iter().find_map(|(pattern, value)| {
            (pattern.len() == segments && pattern.iter().zip(qualname.split('.')).all(|(p, s)| segment_matches(p, s)))
                .then_some(value)
        })
    }

    fn all(&self) -> impl Iterator<Item = String> + '_ {
        self.entries().map(|(name, _)| name)
    }

    fn entries(&self) -> impl Iterator<Item = (String, &T)> + '_ {
        self.exact.iter().map(|(n, v)| (n.clone(), v)).chain(self.globs.iter().map(|(p, v)| (p.join("."), v)))
    }
}

/// The last segment of every name, `None` when one ends in a pattern (`*`).
fn last_segments(names: impl Iterator<Item = String>) -> Option<rustc_hash::FxHashSet<String>> {
    names.map(|n| n.rsplit('.').next().map(str::to_string)).try_fold(rustc_hash::FxHashSet::default(), |mut set, last| {
        set.insert(last.filter(|l| !l.contains('*'))?);
        Some(set)
    })
}

/// Whether `qualname` matches a catalog name (`*` matches within one dotted segment).
pub fn qualname_matches(pattern: &str, qualname: &str) -> bool {
    let (mut p, mut q) = (pattern.split('.'), qualname.split('.'));
    loop {
        match (p.next(), q.next()) {
            (None, None) => return true,
            (Some(p), Some(q)) if segment_matches(p, q) => {}
            _ => return false,
        }
    }
}

/// `*` matches any run of characters inside one segment.
fn segment_matches(pattern: &str, segment: &str) -> bool {
    let Some((first, parts)) = pattern.split_once('*') else {
        return pattern == segment;
    };
    let Some(mut rest) = segment.strip_prefix(first) else {
        return false;
    };
    let mut parts = parts.split('*').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// What the catalog says about one function, looked up once (see [`Catalog::roles`]).
#[derive(Clone, Copy, Default)]
pub struct Roles {
    /// The sink group it belongs to.
    sink: Option<usize>,
    /// The blocking-decorator group it belongs to.
    blocking_decorator: Option<usize>,
    pub offload: bool,
    pub loop_callback: bool,
    pub inspector: bool,
    /// The `[[observer]]` group it dispatches for.
    dispatch: Option<usize>,
    /// The entry-point kind it registers.
    entry: Option<usize>,
    /// The `[[builds_on_loop]]` group it belongs to.
    builds: Option<usize>,
}

impl Roles {
    pub fn is_sink(&self) -> bool {
        self.sink.is_some()
    }

    pub fn is_blocking_decorator(&self) -> bool {
        self.blocking_decorator.is_some()
    }
}

#[derive(Default)]
pub struct Catalog {
    sinks: Patterns<usize>,
    blocking_decorators: Patterns<usize>,
    groups: Vec<(String, String)>,
    /// Groups marked `direct`, by index.
    direct: Vec<bool>,
    /// The `unless_given` arguments of each group, by index.
    unless_given: Vec<Vec<String>>,
    /// The `blocks_in` function of each group, by index.
    blocks_in: Vec<Option<String>>,
    /// The severity of each group, by index.
    severity: Vec<Severity>,
    offload: Patterns<()>,
    /// `[offload]` `copies`.
    copies: Vec<String>,
    loop_callbacks: Patterns<()>,
    inspectors: Patterns<()>,
    dispatches: Patterns<usize>,
    observers: Vec<Observer>,
    arguments: Patterns<usize>,
    argument_effects: Vec<Arguments>,
    /// The last segment of every `arguments` name (`None`: a pattern ends in `*`).
    argument_names: Option<rustc_hash::FxHashSet<String>>,
    /// `[[loop_probe]]` names (or patterns), with the class they raise.
    loop_probes: Vec<(String, String)>,
    /// `[[hook]]` names (or patterns).
    hooks: Vec<(String, Hook)>,
    /// `[[runs_hooks]]` names (or patterns).
    operations: Vec<(String, RunsHooks)>,
    entries: Patterns<usize>,
    entry_groups: Vec<Entry>,
    builds_on_loop: Patterns<usize>,
    builders: Vec<BuildsOnLoop>,
    lazy_attributes: Vec<LazyAttribute>,
    state_namespaces: Vec<StateNamespace>,
    exit_stacks: Vec<ExitStack>,
}

impl Catalog {
    /// The builtin catalog. `opt_in` enables groups marked `opt_in` (CPU-bound work).
    pub fn builtin(opt_in: bool) -> Self {
        let mut catalog = Self::default();
        catalog.extend(BUILTIN, opt_in).expect("builtin catalog is valid");
        catalog
    }

    /// Adds the groups of a catalog document (same schema as the builtin one).
    pub fn extend(&mut self, toml_text: &str, opt_in: bool) -> anyhow::Result<()> {
        let file: CatalogFile = toml::from_str(toml_text).context("invalid catalog")?;
        for (group, blocking_decorator) in
            file.sink.into_iter().map(|g| (g, false)).chain(file.blocking_decorator.into_iter().map(|g| (g, true)))
        {
            if group.opt_in && !opt_in {
                continue;
            }
            let index = self.groups.len();
            self.groups.push((group.category, group.advice));
            self.direct.push(group.direct);
            self.unless_given.push(group.unless_given);
            self.blocks_in.push(group.blocks_in);
            self.severity.push(group.severity);
            let patterns = if blocking_decorator { &mut self.blocking_decorators } else { &mut self.sinks };
            for name in &group.functions {
                patterns.insert(name, index);
            }
        }
        for name in &file.offload.functions {
            self.offload.insert(name, ());
        }
        self.copies.extend(file.offload.copies);
        for name in &file.loop_callback.functions {
            self.loop_callbacks.insert(name, ());
        }
        for name in &file.inspects.functions {
            self.inspectors.insert(name, ());
        }
        for observer in file.observer {
            let index = self.observers.len();
            for name in &observer.dispatch {
                self.dispatches.insert(name, index);
            }
            self.observers.push(observer);
        }
        for ArgumentGroup { calls, iterates, enters, methods, lazy, factory, names, functions } in file.arguments {
            let index = self.argument_effects.len();
            self.argument_effects.push(Arguments { calls, iterates, enters, methods, lazy, factory, names });
            for name in &functions {
                self.arguments.insert(name, index);
            }
        }
        self.argument_names = last_segments(self.arguments.all());
        for HookGroup { runs, callable, fields, json_only, functions } in file.hook {
            let hook = Hook { runs, callable, fields, json_only };
            self.hooks.extend(functions.into_iter().map(|f| (f, hook.clone())));
        }
        for RunsHooksGroup { runs, json, json_unless, exclude, include, functions } in file.runs_hooks {
            let operation = RunsHooks { runs, json, json_unless, exclude, include };
            self.operations.extend(functions.into_iter().map(|f| (f, operation.clone())));
        }
        for group in file.loop_probe {
            self.loop_probes.extend(group.functions.into_iter().map(|f| (f, group.raises.clone())));
        }
        self.state_namespaces.extend(file.state_namespace);
        self.lazy_attributes.extend(file.lazy_attribute);
        self.exit_stacks.extend(file.exit_stack);
        for BuildsOnLoopGroup { kind, slot, label, unless_passthrough, functions } in file.builds_on_loop {
            let index = self.builders.len();
            self.builders.push(BuildsOnLoop { kind, slot, label, unless_passthrough });
            for name in &functions {
                self.builds_on_loop.insert(name, index);
            }
        }
        for group in file.entry {
            let EntryGroup {
                kind,
                slot,
                validates_params,
                serializes_return,
                validates_return,
                response_model,
                response_exclude,
                passthrough,
                injects,
                runs_once,
                functions,
            } = group;
            let index = self.entry_groups.len();
            self.entry_groups.push(Entry {
                kind,
                slot,
                validates_params,
                serializes_return,
                validates_return,
                response_model,
                response_exclude,
                passthrough,
                injects,
                runs_once,
            });
            for name in &functions {
                self.entries.insert(name, index);
            }
        }
        Ok(())
    }

    /// Every role of `qualname`, for the lookups below that take [`Roles`].
    pub fn roles(&self, qualname: &str) -> Roles {
        Roles {
            sink: self.sinks.get(qualname).copied(),
            blocking_decorator: self.blocking_decorators.get(qualname).copied(),
            offload: self.offload.get(qualname).is_some(),
            loop_callback: self.loop_callbacks.get(qualname).is_some(),
            inspector: self.inspectors.get(qualname).is_some(),
            dispatch: self.dispatches.get(qualname).copied(),
            entry: self.entries.get(qualname).copied(),
            builds: self.builds_on_loop.get(qualname).copied(),
        }
    }

    /// [`Catalog::sink`] for a function whose roles are known.
    pub fn sink_of(&self, roles: Roles, qualname: &str) -> Option<Sink> {
        let index = roles.sink?;
        Some(self.group_sink(index, self.blocks_in[index].as_deref().unwrap_or(qualname)))
    }

    /// What blocking in `qualname`, a function of group `index`, means.
    fn group_sink(&self, index: usize, qualname: &str) -> Sink {
        let (category, advice) = &self.groups[index];
        Sink {
            qualname: qualname.to_string(),
            category: category.clone(),
            advice: advice.clone(),
            severity: self.severity[index],
        }
    }

    /// [`Catalog::blocking_decorator`] for a function whose roles are known.
    pub fn blocking_decorator_of(&self, roles: Roles, qualname: &str) -> Option<Sink> {
        Some(self.group_sink(roles.blocking_decorator?, qualname))
    }

    /// [`Catalog::is_direct_sink`] for a function whose roles are known.
    pub fn is_direct_sink_of(&self, roles: Roles) -> bool {
        roles.sink.is_some_and(|index| self.direct[index])
    }

    /// [`Catalog::unless_given`] for a function whose roles are known.
    pub fn unless_given_of(&self, roles: Roles) -> &[String] {
        roles.sink.map_or(&[], |index| &self.unless_given[index])
    }

    /// What registering with an entry function means, for a function whose roles are known.
    pub fn entry_of(&self, roles: Roles) -> Option<&Entry> {
        roles.entry.map(|i| &self.entry_groups[i])
    }

    /// What the function (with known roles) constructs on the loop later (`[[builds_on_loop]]`).
    pub fn builds_on_loop_of(&self, roles: Roles) -> Option<&BuildsOnLoop> {
        roles.builds.map(|i| &self.builders[i])
    }

    /// [`Catalog::entry_kind`] for a function whose roles are known.
    pub fn entry_kind_of(&self, roles: Roles) -> Option<&str> {
        roles.entry.map(|i| self.entry_groups[i].kind.as_str())
    }

    /// Every `[[entry]]` name (or pattern), with what registering there means.
    pub fn entry_entries(&self) -> impl Iterator<Item = (String, &Entry)> + '_ {
        self.entries.entries().map(|(name, &i)| (name, &self.entry_groups[i]))
    }

    /// The `passthrough` classes of every `[[entry]]`.
    pub fn passthrough(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.entry_groups.iter().flat_map(|e| e.passthrough.iter().map(String::as_str)).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    pub fn sink(&self, qualname: &str) -> Option<Sink> {
        let &index = self.sinks.get(qualname)?;
        Some(self.group_sink(index, self.blocks_in[index].as_deref().unwrap_or(qualname)))
    }

    /// Whether `qualname` is a sink only when called directly from loop code.
    pub fn is_direct_sink(&self, qualname: &str) -> bool {
        self.sinks.get(qualname).is_some_and(|&index| self.direct[index])
    }

    /// Arguments whose presence makes a call to the sink `qualname` not block.
    pub fn unless_given(&self, qualname: &str) -> &[String] {
        self.sinks.get(qualname).map_or(&[], |&index| &self.unless_given[index])
    }

    /// The blocking behavior a decorator gives the sync functions it decorates.
    pub fn blocking_decorator(&self, qualname: &str) -> Option<Sink> {
        let &index = self.blocking_decorators.get(qualname)?;
        Some(self.group_sink(index, qualname))
    }

    pub fn is_offload(&self, qualname: &str) -> bool {
        self.offload.get(qualname).is_some()
    }

    /// Every `[offload]` name (or pattern).
    pub fn offloads(&self) -> impl Iterator<Item = String> + '_ {
        self.offload.all()
    }

    /// The `[offload]` functions that may run what they are given on a copy, in another process.
    pub fn copies(&self) -> &[String] {
        &self.copies
    }

    /// Every `[loop_callback]` name (or pattern).
    pub fn loop_callbacks(&self) -> impl Iterator<Item = String> + '_ {
        self.loop_callbacks.all()
    }

    pub fn is_loop_callback(&self, qualname: &str) -> bool {
        self.loop_callbacks.get(qualname).is_some()
    }

    /// Whether callables passed to `qualname` are only inspected, never called.
    pub fn is_inspector(&self, qualname: &str) -> bool {
        self.inspectors.get(qualname).is_some()
    }

    /// The `[[observer]]` groups, in order (their index identifies them).
    pub fn observers(&self) -> &[Observer] {
        &self.observers
    }

    /// The `[[observer]]` a function (with known roles) dispatches for, by index.
    pub fn dispatch_of(&self, roles: Roles) -> Option<usize> {
        roles.dispatch
    }

    /// What `qualname` does with its arguments.
    pub fn arguments(&self, qualname: &str) -> Option<&Arguments> {
        self.arguments.get(qualname).map(|&i| &self.argument_effects[i])
    }

    /// Every `[[arguments]]` name (or pattern), with what it does with its arguments.
    pub fn argument_entries(&self) -> impl Iterator<Item = (String, &Arguments)> + '_ {
        self.arguments.entries().map(|(name, &i)| (name, &self.argument_effects[i]))
    }

    /// Whether a function (or class) named `name` (its last segment) may be in `[[arguments]]`.
    pub fn may_take_arguments(&self, name: &str) -> bool {
        self.argument_names.as_ref().is_none_or(|names| names.contains(name))
    }

    /// Every `[[hook]]` name (or pattern).
    pub fn hooks(&self) -> &[(String, Hook)] {
        &self.hooks
    }

    /// Every `[[runs_hooks]]` name (or pattern).
    pub fn operations(&self) -> &[(String, RunsHooks)] {
        &self.operations
    }

    /// The `[[lazy_attribute]]` groups.
    pub fn lazy_attributes(&self) -> &[LazyAttribute] {
        &self.lazy_attributes
    }

    /// The `[[exit_stack]]` groups.
    pub fn exit_stacks(&self) -> &[ExitStack] {
        &self.exit_stacks
    }

    /// The `[[state_namespace]]` groups, in order (their index identifies them).
    pub fn state_namespaces(&self) -> &[StateNamespace] {
        &self.state_namespaces
    }

    /// Every `[[loop_probe]]` name (or pattern), with the class it raises.
    pub fn loop_probes(&self) -> &[(String, String)] {
        &self.loop_probes
    }

    /// The entry-point kind registered by passing a callable to `qualname`.
    pub fn entry_kind(&self, qualname: &str) -> Option<&str> {
        self.entries.get(qualname).map(|&i| self.entry_groups[i].kind.as_str())
    }

    /// Every name and pattern in the catalog.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .sinks
            .all()
            .chain(self.blocking_decorators.all())
            .chain(self.offload.all())
            .chain(self.copies.iter().cloned())
            .chain(self.loop_callbacks.all())
            .chain(self.inspectors.all())
            .chain(self.observers.iter().flat_map(|o| {
                let attach = o.attach.iter().flat_map(|a| a.functions.iter());
                let classes = o.classes.iter().flat_map(|c| {
                    std::iter::once(&c.base).chain(&c.level).chain(&c.offloaded_by)
                });
                let naming = o.scope.iter().flat_map(|s| s.naming.iter());
                o.dispatch.iter().chain(attach).chain(classes).chain(naming).chain(&o.threshold).chain(&o.any_level).cloned()
            }))
            .chain(self.arguments.all())
            .chain(self.loop_probes.iter().flat_map(|(f, raises)| [f.clone(), raises.clone()]))
            .chain(self.hooks.iter().map(|(f, _)| f.clone()))
            .chain(self.operations.iter().map(|(f, _)| f.clone()))
            .chain(self.entries.all())
            .chain(self.builds_on_loop.all())
            .chain(self.entry_groups.iter().flat_map(|e| e.passthrough.iter().cloned()))
            .chain(self.lazy_attributes.iter().flat_map(|l| {
                let reload = l.reload.iter().flat_map(|r| r.functions.iter());
                let lists = [&l.factories, &l.eager, &l.expires_all, &l.expires, &l.raises_under];
                lists.into_iter().flatten().chain(reload).chain(std::iter::once(&l.loads)).cloned()
            }))
            .chain(self.state_namespaces.iter().flat_map(|n| {
                let app = n.app.iter().flat_map(|a| a.owners.iter());
                let lifespan = n.lifespan.iter().flat_map(|l| l.owners.iter());
                n.owners.iter().chain(app).chain(lifespan).cloned()
            }))
            .chain(self.exit_stacks.iter().flat_map(|e| {
                [&e.callbacks, &e.contexts, &e.runs, &e.moves].into_iter().flatten().cloned()
            }))
            .chain(self.blocks_in.iter().flatten().cloned())
            .collect();
        names.sort();
        names.dedup();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::segment_matches;

    #[test]
    fn glob_segments() {
        assert!(segment_matches("*Commands", "BasicKeyCommands"));
        assert!(segment_matches("*", "anything"));
        assert!(segment_matches("get*", "get_many"));
        assert!(segment_matches("a*c*e", "abcde"));
        assert!(!segment_matches("*Commands", "CommandsParser"));
        assert!(!segment_matches("get*", "set"));
        assert!(!segment_matches("a*c*e", "abcd"));
    }
}
