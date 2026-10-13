//! What hydroid knows about functions it does not analyze: which block, which move work off the
//! event loop, which schedule callbacks on it, and which register FastAPI entry points.

use anyhow::Context;
use rustc_hash::FxHashMap as HashMap;
use serde::Deserialize;

use crate::report::Sink;

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
    offload: Group,
    #[serde(default)]
    loop_callback: Group,
    /// Functions that only inspect the callables passed to them (`functools.wraps(func)`).
    #[serde(default)]
    inspects: Group,
    /// Logging calls that run the attached filters and handlers on the calling thread.
    #[serde(default)]
    log_dispatch: Group,
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
    functions: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Group {
    #[serde(default)]
    functions: Vec<String>,
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
    functions: Vec<String>,
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
    pub log_dispatch: bool,
    /// The entry-point kind it registers.
    entry: Option<usize>,
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
    offload: Patterns<()>,
    loop_callbacks: Patterns<()>,
    inspectors: Patterns<()>,
    log_dispatch: Patterns<()>,
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
    entry_kinds: Vec<String>,
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
            let patterns = if blocking_decorator { &mut self.blocking_decorators } else { &mut self.sinks };
            for name in &group.functions {
                patterns.insert(name, index);
            }
        }
        for name in &file.offload.functions {
            self.offload.insert(name, ());
        }
        for name in &file.loop_callback.functions {
            self.loop_callbacks.insert(name, ());
        }
        for name in &file.inspects.functions {
            self.inspectors.insert(name, ());
        }
        for name in &file.log_dispatch.functions {
            self.log_dispatch.insert(name, ());
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
        for group in file.entry {
            let index = self.entry_kinds.len();
            self.entry_kinds.push(group.kind);
            for name in &group.functions {
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
            log_dispatch: self.log_dispatch.get(qualname).is_some(),
            entry: self.entries.get(qualname).copied(),
        }
    }

    /// [`Catalog::sink`] for a function whose roles are known.
    pub fn sink_of(&self, roles: Roles, qualname: &str) -> Option<Sink> {
        let index = roles.sink?;
        let (category, advice) = &self.groups[index];
        let qualname = self.blocks_in[index].as_deref().unwrap_or(qualname).to_string();
        Some(Sink { qualname, category: category.clone(), advice: advice.clone() })
    }

    /// [`Catalog::blocking_decorator`] for a function whose roles are known.
    pub fn blocking_decorator_of(&self, roles: Roles, qualname: &str) -> Option<Sink> {
        let (category, advice) = &self.groups[roles.blocking_decorator?];
        Some(Sink { qualname: qualname.to_string(), category: category.clone(), advice: advice.clone() })
    }

    /// [`Catalog::is_direct_sink`] for a function whose roles are known.
    pub fn is_direct_sink_of(&self, roles: Roles) -> bool {
        roles.sink.is_some_and(|index| self.direct[index])
    }

    /// [`Catalog::unless_given`] for a function whose roles are known.
    pub fn unless_given_of(&self, roles: Roles) -> &[String] {
        roles.sink.map_or(&[], |index| &self.unless_given[index])
    }

    /// [`Catalog::entry_kind`] for a function whose roles are known.
    pub fn entry_kind_of(&self, roles: Roles) -> Option<&str> {
        roles.entry.map(|i| self.entry_kinds[i].as_str())
    }

    pub fn sink(&self, qualname: &str) -> Option<Sink> {
        let &index = self.sinks.get(qualname)?;
        let (category, advice) = &self.groups[index];
        let qualname = self.blocks_in[index].as_deref().unwrap_or(qualname).to_string();
        Some(Sink { qualname, category: category.clone(), advice: advice.clone() })
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
        let (category, advice) = &self.groups[index];
        Some(Sink { qualname: qualname.to_string(), category: category.clone(), advice: advice.clone() })
    }

    pub fn is_offload(&self, qualname: &str) -> bool {
        self.offload.get(qualname).is_some()
    }

    pub fn is_loop_callback(&self, qualname: &str) -> bool {
        self.loop_callbacks.get(qualname).is_some()
    }

    /// Whether callables passed to `qualname` are only inspected, never called.
    pub fn is_inspector(&self, qualname: &str) -> bool {
        self.inspectors.get(qualname).is_some()
    }

    /// Whether calling `qualname` runs the filters and handlers attached to the loggers.
    pub fn is_log_dispatch(&self, qualname: &str) -> bool {
        self.log_dispatch.get(qualname).is_some()
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

    /// Every `[[loop_probe]]` name (or pattern), with the class it raises.
    pub fn loop_probes(&self) -> &[(String, String)] {
        &self.loop_probes
    }

    /// The entry-point kind registered by passing a callable to `qualname`.
    pub fn entry_kind(&self, qualname: &str) -> Option<&str> {
        self.entries.get(qualname).map(|&i| self.entry_kinds[i].as_str())
    }

    /// Every name and pattern in the catalog.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .sinks
            .all()
            .chain(self.blocking_decorators.all())
            .chain(self.offload.all())
            .chain(self.loop_callbacks.all())
            .chain(self.inspectors.all())
            .chain(self.log_dispatch.all())
            .chain(self.arguments.all())
            .chain(self.loop_probes.iter().flat_map(|(f, raises)| [f.clone(), raises.clone()]))
            .chain(self.hooks.iter().map(|(f, _)| f.clone()))
            .chain(self.operations.iter().map(|(f, _)| f.clone()))
            .chain(self.entries.all())
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
