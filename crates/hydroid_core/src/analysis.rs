//! From facts to diagnostics.
//!
//! Where code runs is a property of the *call*, not of the function: a sync function runs on the
//! event loop when called from code on the loop, and in a worker thread when handed to
//! `asyncio.to_thread`. So "does calling `f` block?" has one answer per function, computed once
//! by a breadth-first search backwards from blocking calls (shortest witness chains, O(V+E), no
//! recursion, any depth). Diagnostics are the call sites inside loop code (`async def` bodies and
//! callbacks scheduled on the loop) whose callee blocks.

use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::rc::Rc;

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::catalog::Catalog;
use crate::facts::{
    Call, ClassId, Facts, FnId, Flow, FunctionKind, Location, LoggerName, Origin, Signature, Slot, Value,
};
use crate::report::{Diagnostic, Frame, ReachedFrom, Report, Sink, Unresolved};

pub fn analyze(facts: &Facts, catalog: &Catalog) -> Report {
    let graph = Graph::new(facts, catalog);
    let blocking = graph.blocking_distances();
    let roots = graph.roots();
    let context = graph.entry_paths();

    let suppressed: HashSet<(&str, u32)> =
        facts.suppressed.iter().map(|(p, l)| (p.as_str(), *l)).collect();
    let mut diagnostics = Vec::new();
    for &root in &roots {
        let root_fn = facts.function(root);
        let steps = context.steps_to(root);
        // Diagnostics land in project code: blocking inside a library coroutine is reported where
        // project code enters the library, with the path into the library prepended to the chain.
        let (function, entered_at, prefix, reached) = if root_fn.origin == Origin::Project {
            let reached = steps.as_ref().map(|s| context.reached_from(s, s.steps.len()));
            (root_fn.qualname.clone(), None, Vec::new(), reached)
        } else {
            let Some(s) = steps else { continue };
            let Some(cut) = s.steps.iter().rposition(|(caller, _, _)| facts.function(*caller).origin == Origin::Project)
            else {
                continue;
            };
            let (caller, call, _) = s.steps[cut];
            let prefix = s.steps[cut..].iter().map(|&(_, call, target)| graph.frame(call, target)).collect();
            let reached = s.starts_at_entry_point().then(|| context.reached_from(&s, cut));
            (facts.function(caller).qualname.clone(), Some(call), prefix, reached)
        };
        for &call in &graph.calls_of[root.0 as usize] {
            let Some((chain, sink)) = graph.blocking_chain(call, &blocking) else {
                continue;
            };
            let location = &facts.calls[entered_at.unwrap_or(call)].location;
            if suppressed.contains(&(location.path.as_str(), location.line)) {
                continue;
            }
            diagnostics.push(Diagnostic {
                location: location.clone(),
                function: function.clone(),
                sink,
                chain: prefix.iter().cloned().chain(chain).collect(),
                reached_from: reached.clone(),
            });
        }
    }
    diagnostics.sort_by(|a, b| (&a.location, &a.sink.qualname).cmp(&(&b.location, &b.sink.qualname)));
    diagnostics.dedup_by(|a, b| a.location == b.location && a.sink.qualname == b.sink.qualname);

    let mut report = Report { diagnostics, unresolved: graph.unresolved(&roots), ..Report::default() };
    report.stats.entry_points = context.entries;
    report.stats.functions_analyzed = facts.functions.iter().filter(|f| f.analyzed).count();
    report.stats.call_sites = facts.calls.len();
    report.stats.call_sites_resolved = (0..facts.calls.len())
        .filter(|&i| !graph.targets[i].is_empty() || facts.calls[i].param.is_some())
        .count();
    report
}

struct Graph<'a> {
    facts: &'a Facts,
    catalog: &'a Catalog,
    /// Calls made by each function.
    calls_of: Vec<Vec<usize>>,
    /// Effective targets of each call: resolved targets, decorator wrappers, overrides in
    /// subclasses, callables flowing into called parameters. Offload APIs are cut.
    targets: Vec<Vec<FnId>>,
    /// What blocking means for each function, if it blocks by itself: a catalog sink, or a sync
    /// function wrapped in a blocking decorator (retry loops).
    sink: Vec<Option<Sink>>,
    /// See [`Graph::called_params`].
    called: HashSet<ParamKey>,
    /// Catalog offload APIs, by function.
    offload: Vec<bool>,
    /// Sinks that only count when called directly from loop code, by function.
    direct: Vec<bool>,
    /// Arguments that make a call to a sink not block (catalog `unless_given`), by function.
    unless_given: Vec<&'a [String]>,
    /// Parameters that are called, or handed to something other than an offload API or a
    /// function that only inspects them.
    used_params: HashSet<ParamKey>,
    /// See [`Graph::framework_hooks`].
    hooks: Vec<(FnId, String)>,
    /// See [`Graph::logging_hooks`].
    log_hooks: Vec<LogHook>,
    /// Catalog logging calls running them, by function (empty without hooks): what they log,
    /// and at which level (`None`: the level is an argument, `Logger.log`).
    log_dispatch: Vec<Option<(LogCall, Option<u32>)>>,
    /// The lowest level the project sets on a logger (see [`Graph::logger_threshold`]).
    log_threshold: u32,
    /// See [`Graph::logger_stops`].
    log_stops: Vec<String>,
}

type ParamKey = (FnId, String);

/// What [`Graph::effective_targets`] reads besides the call.
#[derive(Clone, Copy)]
struct Context<'g> {
    nested: &'g [Vec<FnId>],
    params: &'g HashMap<ParamKey, BTreeSet<FnId>>,
    called: &'g HashSet<ParamKey>,
    hierarchy: &'g Hierarchy,
    stored: &'g HashMap<(ClassId, String), BTreeSet<FnId>>,
    returned: &'g HashMap<FnId, BTreeSet<FnId>>,
    dispatches: &'g HashMap<FnId, Vec<FnId>>,
}

impl<'a> Graph<'a> {
    fn new(facts: &'a Facts, catalog: &'a Catalog) -> Self {
        let n = facts.functions.len();
        let mut calls_of = vec![Vec::new(); n];
        for (i, call) in facts.calls.iter().enumerate() {
            calls_of[call.caller.0 as usize].push(i);
        }
        let sink = facts
            .functions
            .iter()
            .map(|f| {
                catalog.sink(&f.qualname).or_else(|| {
                    let decorators = f.decorators.iter().map(|d| &facts.function(*d).qualname);
                    (!f.is_async).then(|| decorators.filter_map(|d| catalog.blocking_decorator(d)).next())?
                })
            })
            .collect();
        let offload = facts.functions.iter().map(|f| catalog.is_offload(&f.qualname)).collect();
        let direct = facts.functions.iter().map(|f| catalog.is_direct_sink(&f.qualname)).collect();
        let mut graph = Self {
            facts,
            catalog,
            calls_of,
            targets: Vec::new(),
            sink,
            called: HashSet::default(),
            offload,
            direct,
            unless_given: facts.functions.iter().map(|f| catalog.unless_given(&f.qualname)).collect(),
            used_params: HashSet::default(),
            hooks: Vec::new(),
            log_hooks: Vec::new(),
            log_dispatch: Vec::new(),
            log_threshold: 0,
            log_stops: Vec::new(),
        };
        let used_from_calls = facts.calls.iter().filter_map(|c| c.param.clone());
        let used_from_flows: Vec<ParamKey> = facts
            .flows
            .iter()
            .filter(|f| !graph.is_offload(f.into) && !catalog.is_inspector(&facts.function(f.into).qualname))
            .filter_map(|f| match &f.value {
                Value::Param(owner, name) => Some((*owner, name.clone())),
                _ => None,
            })
            .collect();
        graph.used_params = used_from_calls.chain(used_from_flows).collect();
        let nested = graph.nested_functions();
        let params = graph.parameter_values(&nested);
        let stored = graph.stored_values(&params);
        let returned = graph.returned_values(&params);
        let mut dispatches: HashMap<FnId, Vec<FnId>> = HashMap::default();
        for &(dispatcher, implementation) in &facts.dispatches {
            dispatches.entry(dispatcher).or_default().push(implementation);
        }
        graph.called = graph.called_params();
        let called = &graph.called;
        let hierarchy = Hierarchy::new(facts);
        // A caller's calls are all in one file: line and column identify the call.
        let mut flows_at: HashMap<(FnId, u32, u32), Vec<&Flow>> = HashMap::default();
        for flow in &facts.flows {
            flows_at.entry((flow.caller, flow.location.line, flow.location.column)).or_default().push(flow);
        }
        let cx = Context {
            nested: &nested,
            params: &params,
            called,
            hierarchy: &hierarchy,
            stored: &stored,
            returned: &returned,
            dispatches: &dispatches,
        };
        graph.log_hooks = graph.logging_hooks();
        if !graph.log_hooks.is_empty() {
            let kind = |q: &str| {
                let method = q.rsplit('.').next().unwrap_or_default();
                let call = if method == "exception" { LogCall::Exception } else { LogCall::Record };
                let level = match method {
                    "info" => Some(20),
                    "warning" | "warn" => Some(30),
                    "error" | "exception" => Some(40),
                    "critical" => Some(50),
                    _ => None,
                };
                (call, level)
            };
            graph.log_dispatch =
                facts.functions.iter().map(|f| catalog.is_log_dispatch(&f.qualname).then(|| kind(&f.qualname))).collect();
            graph.log_threshold = graph.logger_threshold();
            graph.log_stops = graph.logger_stops();
        }
        let mut runs = HashMap::default();
        graph.targets = facts
            .calls
            .iter()
            .map(|call| {
                let at = (call.caller, call.location.line, call.location.column);
                let flows = flows_at.get(&at).map_or(&[][..], |f| &f[..]);
                graph.effective_targets(call, flows, &cx, &mut runs)
            })
            .collect();
        graph.hooks = graph.framework_hooks();
        graph
    }

    fn is_offload(&self, f: FnId) -> bool {
        self.offload[f.0 as usize]
    }

    /// Functions defined (at any depth) inside each function.
    fn nested_functions(&self) -> Vec<Vec<FnId>> {
        let mut nested = vec![Vec::new(); self.facts.functions.len()];
        for (i, f) in self.facts.functions.iter().enumerate() {
            let mut parent = f.parent;
            while let Some(p) = parent {
                nested[p.0 as usize].push(FnId(i as u32));
                parent = self.facts.function(p).parent;
            }
        }
        nested
    }

    /// The parameters a flow into `into` at `slot` lands on. A decorated function lands on the
    /// first parameter of the decorator and of every function nested in it (decorator factories).
    fn slot_params(&self, into: FnId, slot: &Slot, nested: &[Vec<FnId>]) -> Vec<ParamKey> {
        match slot {
            Slot::Param(name) => vec![(into, name.clone())],
            Slot::Unknown => Vec::new(),
            Slot::Decorated => std::iter::once(into)
                .chain(nested[into.0 as usize].iter().copied())
                .filter_map(|owner| {
                    let f = self.facts.function(owner);
                    let first = f.params.iter().find(|p| *p != "self" && *p != "cls")?;
                    Some((owner, first.clone()))
                })
                .collect(),
        }
    }

    /// Which functions each parameter can hold, to a fixpoint over forwarded parameters.
    fn parameter_values(&self, nested: &[Vec<FnId>]) -> HashMap<ParamKey, BTreeSet<FnId>> {
        let mut values: HashMap<ParamKey, BTreeSet<FnId>> = HashMap::default();
        let mut forwards: HashMap<ParamKey, Vec<ParamKey>> = HashMap::default();
        for flow in &self.facts.flows {
            // A decorated function reaches its decorator's wrapper only when it is itself called,
            // and that call links to it directly (see `effective_targets`): letting it flow into
            // the decorator's parameter would make every wrapper call every decorated function.
            if self.is_offload(flow.into) || flow.slot == Slot::Decorated {
                continue;
            }
            for key in self.slot_params(flow.into, &flow.slot, nested) {
                match &flow.value {
                    Value::Function(f) => {
                        values.entry(key).or_default().insert(*f);
                    }
                    Value::Param(owner, name) => {
                        forwards.entry((*owner, name.clone())).or_default().push(key);
                    }
                    Value::Class(_) => {}
                }
            }
        }
        let mut queue: VecDeque<ParamKey> = values.keys().cloned().collect();
        while let Some(from) = queue.pop_front() {
            let Some(targets) = forwards.get(&from) else { continue };
            let incoming = values[&from].clone();
            for to in targets {
                let slot = values.entry(to.clone()).or_default();
                let before = slot.len();
                slot.extend(&incoming);
                if slot.len() != before {
                    queue.push_back(to.clone());
                }
            }
        }
        values
    }

    /// Parameters of sync functions that the function itself ends up calling during the call:
    /// directly, or by forwarding them to such a parameter of another function. Callables passed
    /// there run as part of that call, so they are attributed to the call site that passes them
    /// (keeping `apply(blocking, x)` and `apply(abs, x)` apart).
    fn called_params(&self) -> HashSet<ParamKey> {
        let direct = |owner: FnId, caller: FnId| owner == caller && !self.facts.function(owner).is_async;
        let mut called: HashSet<ParamKey> = self
            .facts
            .calls
            .iter()
            .filter_map(|c| c.param.as_ref().filter(|(owner, _)| direct(*owner, c.caller)).cloned())
            .collect();
        loop {
            let before = called.len();
            for flow in &self.facts.flows {
                if let (Value::Param(owner, name), Slot::Param(slot)) = (&flow.value, &flow.slot)
                    && direct(*owner, flow.caller)
                    && !self.is_offload(flow.into)
                    && called.contains(&(flow.into, slot.clone()))
                {
                    called.insert((*owner, name.clone()));
                }
            }
            if called.len() == before {
                return called;
            }
        }
    }

    /// `runs` memoizes, per target, what calling it runs (itself or decorator wrappers) and its
    /// overrides.
    fn effective_targets(
        &self,
        call: &Call,
        flows: &[&Flow],
        cx: &Context,
        runs: &mut HashMap<FnId, (Vec<FnId>, Vec<FnId>)>,
    ) -> Vec<FnId> {
        let Context { nested, params, called, hierarchy, stored, returned, dispatches } = *cx;
        let mut out = Vec::new();
        for &t in &call.targets {
            let (body, overrides) = runs.entry(t).or_insert_with(|| {
                // Calling a function decorated by project code runs the decorator's wrapper, and
                // the wrapper calls the original. Decorators wrapping both kinds define an async
                // and a sync wrapper: a coroutine function gets the async ones.
                let is_async = self.facts.function(t).is_async;
                let mut body = Vec::new();
                for d in self.facts.function(t).decorators.iter().filter(|d| self.facts.function(**d).analyzed) {
                    let wrappers = &nested[d.0 as usize];
                    let matching: Vec<FnId> =
                        wrappers.iter().copied().filter(|w| self.facts.function(*w).is_async == is_async).collect();
                    body.extend(if matching.is_empty() { wrappers.clone() } else { matching });
                }
                if self.facts.function(t).decorators.iter().any(|d| self.calls_original(*d, nested)) {
                    body.push(t);
                }
                if body.is_empty() {
                    body.push(t);
                }
                (body, hierarchy.overrides(self.facts, t))
            });
            out.extend(body.iter().copied());
            out.extend(dispatches.get(&t).into_iter().flatten());
            if call.virtual_dispatch {
                // An instance of a known class runs only the overrides of its subclasses.
                let receiver = call.receiver.filter(|r| !self.facts.class(*r).is_protocol);
                let below = |o: FnId| {
                    receiver.is_none_or(|r| self.facts.function(o).class.is_some_and(|c| self.inherits(c, r)))
                };
                out.extend(overrides.iter().copied().filter(|&o| below(o) && self.dispatches_into(call.caller, o)));
            }
            if let Some(receiver) = call.receiver {
                out.extend(hierarchy.super_chain(self.facts, receiver, t));
            }
        }
        // A logging call runs the attached filters and handlers.
        let logs = call.targets.iter().find_map(|t| self.log_dispatch.get(t.0 as usize).copied().flatten());
        if let Some((logs, fixed)) = logs {
            // A level that is not known counts as high enough.
            let level = fixed.or_else(|| call.level.as_ref()?.value).unwrap_or(u32::MAX);
            let in_tests = is_test_path(&self.facts.function(call.caller).location.path);
            let logger = call.logging.as_ref().and_then(|l| l.logger.as_ref());
            if level >= self.log_threshold {
                out.extend(
                    self.log_hooks
                        .iter()
                        .filter(|h| (h.outside_tests || in_tests) && h.runs_for <= logs && h.level <= level)
                        .filter(|h| self.log_reaches(&h.scope, logger))
                        .map(|h| h.function),
                );
            }
        }
        if let Some(class) = call.constructs {
            out.extend(self.construction_hooks(class));
            if let Some(given) = &call.given {
                out.extend(self.default_factories(class, given));
            }
        }
        if let Some((class, names)) = &call.sets {
            out.extend(self.attribute_hooks(*class, names));
        }
        for f in &call.returned_by {
            out.extend(returned.get(f).into_iter().flatten());
        }
        // The method, when ty could not type the receiver; else a callable stored in an
        // attribute of the class or of one of its bases.
        if let Some((class, name)) = &call.attribute {
            out.extend(Hierarchy::lookup(self.facts, *class, name));
            let mut seen = HashSet::from_iter([*class]);
            let mut queue = VecDeque::from([*class]);
            while let Some(c) = queue.pop_front() {
                if let Some(values) = stored.get(&(c, name.clone())) {
                    out.extend(values);
                }
                queue.extend(self.facts.class(c).bases.iter().filter(|b| seen.insert(**b)));
            }
        }
        // Callables passed here that the callee calls during this call.
        for flow in flows {
            if let (Value::Function(f), Slot::Param(slot)) = (&flow.value, &flow.slot)
                && called.contains(&(flow.into, slot.clone()))
                && !self.is_offload(flow.into)
            {
                out.push(*f);
            }
        }
        // A called parameter: closures (decorator wrappers) and `async def`s see every callable
        // flowing into it; sync functions calling their own parameter are handled above, at the
        // call sites passing the callable.
        if let Some(key) = &call.param
            && !called.contains(key)
            && let Some(values) = params.get(key)
        {
            out.extend(values);
        }
        out.sort_unstable();
        out.dedup();
        // Offload APIs run what they are given elsewhere; some also block themselves
        // (`Executor.map` results are waited for).
        out.retain(|&t| !self.is_offload(t) || self.sink[t.0 as usize].is_some());
        out
    }

    /// The callables each `(class, attribute)` can hold: stored directly, or through a parameter
    /// of the storing method (`self.fetch = fetch` in `__init__`).
    fn stored_values(
        &self,
        params: &HashMap<ParamKey, BTreeSet<FnId>>,
    ) -> HashMap<(ClassId, String), BTreeSet<FnId>> {
        let mut stored: HashMap<(ClassId, String), BTreeSet<FnId>> = HashMap::default();
        for store in &self.facts.stores {
            let values = stored.entry((store.class, store.name.clone())).or_default();
            match &store.value {
                Value::Function(f) => {
                    values.insert(*f);
                }
                Value::Param(owner, name) => values.extend(params.get(&(*owner, name.clone())).into_iter().flatten()),
                Value::Class(_) => {}
            }
        }
        stored
    }

    /// The callables each function can return: directly, or a parameter's values.
    fn returned_values(&self, params: &HashMap<ParamKey, BTreeSet<FnId>>) -> HashMap<FnId, BTreeSet<FnId>> {
        let mut returned: HashMap<FnId, BTreeSet<FnId>> = HashMap::default();
        for (function, value) in &self.facts.returns {
            let values = returned.entry(*function).or_default();
            match value {
                Value::Function(f) => {
                    values.insert(*f);
                }
                Value::Param(owner, name) => values.extend(params.get(&(*owner, name.clone())).into_iter().flatten()),
                Value::Class(_) => {}
            }
        }
        returned
    }

    /// What constructing `class` runs besides `__init__`: dataclass `__post_init__`, pydantic
    /// `model_post_init` and validators (of the class and its bases).
    fn construction_hooks(&self, class: ClassId) -> Vec<FnId> {
        let mut hooks: Vec<FnId> = ["__post_init__", "model_post_init"]
            .iter()
            .filter_map(|name| Hierarchy::lookup(self.facts, class, name))
            .collect();
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            let cls = self.facts.class(c);
            hooks.extend(cls.methods.iter().map(|(_, m)| *m).filter(|m| self.facts.function(*m).is_validator));
            queue.extend(cls.bases.iter().filter(|b| seen.insert(**b)));
        }
        hooks
    }

    /// The default factories constructing `class` calls when it is given only the `given` fields
    /// (of the class and its bases). A class as factory runs its `__init__`.
    fn default_factories(&self, class: ClassId, given: &[String]) -> Vec<FnId> {
        let mut factories = Vec::new();
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            let cls = self.facts.class(c);
            for (names, factory) in &cls.default_factories {
                if names.iter().any(|n| given.contains(n)) {
                    continue;
                }
                match factory {
                    Value::Function(f) => factories.push(*f),
                    Value::Class(k) => factories.extend(Hierarchy::lookup(self.facts, *k, "__init__")),
                    Value::Param(..) => {}
                }
            }
            queue.extend(cls.bases.iter().filter(|b| seen.insert(**b)));
        }
        factories
    }

    /// The SQLAlchemy validators setting `names` on an instance of `class` runs (of the class and
    /// its bases).
    fn attribute_hooks(&self, class: ClassId, names: &[String]) -> Vec<FnId> {
        let mut hooks = Vec::new();
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            let cls = self.facts.class(c);
            hooks.extend(
                cls.methods
                    .iter()
                    .map(|(_, m)| *m)
                    .filter(|m| self.facts.function(*m).validates.iter().any(|v| names.contains(v))),
            );
            queue.extend(cls.bases.iter().filter(|b| seen.insert(**b)));
        }
        hooks
    }

    /// What validating `class` from request data runs: its construction hooks and default
    /// factories (the request may leave those fields out), and those of the models nested in its
    /// fields.
    fn validation_hooks(&self, class: ClassId) -> Vec<FnId> {
        let mut hooks = Vec::new();
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            hooks.extend(self.construction_hooks(c));
            hooks.extend(self.default_factories(c, &[]));
            queue.extend(self.facts.class(c).fields.iter().filter(|f| seen.insert(**f)));
        }
        hooks
    }

    /// What dumping an instance of `class` runs: serialization hooks of the class, its bases and
    /// the models nested in its fields.
    fn serialization_hooks(&self, class: ClassId) -> Vec<FnId> {
        let mut hooks = Vec::new();
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            let cls = self.facts.class(c);
            hooks.extend(cls.methods.iter().map(|(_, m)| *m).filter(|m| self.facts.function(*m).is_serializer));
            queue.extend(cls.bases.iter().chain(&cls.fields).filter(|b| seen.insert(**b)));
        }
        hooks
    }

    /// What a logging call (catalog `log_dispatch`) runs on the calling thread besides stdlib code:
    /// `Logger.handle` runs the logger's filters, then `Handler.handle` of every handler (its
    /// filters, then `emit`, which formats the record). Handlers, filters and formatters count once
    /// project code attaches them, which constructing them (or passing the class) stands for. Each
    /// hook comes with whether it is attached outside test code (test doubles only stand in for
    /// test code's logging), and the loggers it runs for (see [`Graph::log_scopes`]).
    fn logging_hooks(&self) -> Vec<LogHook> {
        let find = |name: &str| self.facts.classes.iter().position(|c| c.qualname == name).map(|i| ClassId(i as u32));
        let (handler, filter, formatter) = (find("logging.Handler"), find("logging.Filter"), find("logging.Formatter"));
        if handler.is_none() && filter.is_none() && formatter.is_none() {
            return Vec::new();
        }
        let scopes = self.log_scopes(handler);
        let mut attached: HashMap<ClassId, bool> = HashMap::default();
        let mut hooks: Vec<LogHook> = Vec::new();
        let mut attach = |class: ClassId, caller: FnId| {
            let outside_tests = !is_test_path(&self.facts.function(caller).location.path);
            *attached.entry(class).or_default() |= outside_tests;
        };
        for call in &self.facts.calls {
            if let Some(class) = call.constructs
                && self.facts.function(call.caller).origin == Origin::Project
            {
                attach(class, call.caller);
            }
        }
        for flow in &self.facts.flows {
            if self.facts.function(flow.caller).origin != Origin::Project {
                continue;
            }
            match flow.value {
                Value::Class(class) => attach(class, flow.caller),
                // A plain callable as a filter (`handler.addFilter(drop_health_checks)`).
                Value::Function(f) if self.facts.function(flow.into).qualname == "logging.Filterer.addFilter" => {
                    let outside_tests = !is_test_path(&self.facts.function(flow.caller).location.path);
                    let call = self.facts.calls.iter().find(|c| {
                        c.caller == flow.caller && c.location == flow.location && c.targets.contains(&flow.into)
                    });
                    let scope = match call {
                        Some(call) => self.attachment_scope(call, &scopes, handler),
                        None => None,
                    };
                    hooks.push(LogHook { function: f, outside_tests, runs_for: LogCall::Record, level: 0, scope });
                }
                _ => {}
            }
        }
        // Handlers given to a `QueueListener` emit from its thread; which ones are is not tracked.
        let queued = attached.keys().any(|&c| self.facts.class(c).qualname == "logging.handlers.QueueListener");
        let handler_levels = self.handler_levels();
        let is = |class: ClassId, base: Option<ClassId>| base.is_some_and(|b| self.inherits(class, b));
        let lookup = |class: ClassId, name: &str| Hierarchy::lookup(self.facts, class, name);
        let analyzed = |f: &Option<FnId>| f.is_some_and(|f| self.facts.function(f).analyzed);
        for (&class, &outside_tests) in &attached {
            let scope = scopes.get(&class).cloned().flatten();
            let mut methods: Vec<(Option<FnId>, LogCall, u32)> = Vec::new();
            // `Logger.callHandlers` skips handlers whose level is above the record's.
            let level = handler_levels.get(&class).copied().unwrap_or(0);
            let mut push = |m: Option<FnId>| methods.push((m, LogCall::Record, level));
            if is(class, handler) && !queued {
                let handle = lookup(class, "handle");
                if analyzed(&handle) {
                    push(handle);
                } else {
                    push(lookup(class, "filter"));
                    let emit = lookup(class, "emit");
                    if analyzed(&emit) {
                        push(emit);
                    } else {
                        // The stdlib `emit`s format the record, and the file and socket ones do
                        // I/O through these (catalog sinks).
                        for m in ["format", "shouldRollover", "reopenIfNeeded", "send"] {
                            push(lookup(class, m));
                        }
                    }
                }
            }
            if is(class, filter) {
                methods.push((lookup(class, "filter"), LogCall::Record, 0));
            }
            if is(class, formatter) {
                let format = lookup(class, "format");
                if analyzed(&format) {
                    methods.push((format, LogCall::Record, 0));
                } else {
                    for m in ["usesTime", "formatMessage", "formatTime"] {
                        methods.push((lookup(class, m), LogCall::Record, 0));
                    }
                    // `Formatter.format` formats a traceback only for records carrying one.
                    for m in ["formatException", "formatStack"] {
                        methods.push((lookup(class, m), LogCall::Exception, 0));
                    }
                }
            }
            hooks.extend(
                methods
                    .into_iter()
                    .filter_map(|(m, runs_for, level)| Some((m?, runs_for, level)))
                    .filter(|&(m, _, _)| self.facts.function(m).analyzed || self.sink[m.0 as usize].is_some())
                    .map(|(function, runs_for, level)| LogHook {
                        function,
                        outside_tests,
                        runs_for,
                        level,
                        scope: scope.clone(),
                    }),
            );
        }
        hooks.sort_unstable_by_key(|h| (h.function, h.outside_tests, h.runs_for, h.level));
        // The same method attached through several classes runs for the loggers of each.
        hooks.dedup_by(|h, kept| {
            let same = (h.function, h.outside_tests, h.runs_for, h.level)
                == (kept.function, kept.outside_tests, kept.runs_for, kept.level);
            if same {
                kept.scope = match (kept.scope.take(), h.scope.take()) {
                    (Some(mut a), Some(b)) => {
                        a.extend(b);
                        Some(a)
                    }
                    _ => None,
                };
            }
            same
        });
        hooks
    }

    /// The loggers the hooks of each class attached in project code run for (`None`: any). A
    /// class counts as attached only where project code attaches it when every instance it
    /// constructs is only attached ([`crate::facts::LogUse::attach_only`]) and the class itself is not passed
    /// on (`dictConfig`, `handlers=[...]` lists and the like attach it anywhere).
    fn log_scopes(&self, handler: Option<ClassId>) -> HashMap<ClassId, LogScope> {
        let project = |call: &Call| self.facts.function(call.caller).origin == Origin::Project;
        let mut anywhere: HashSet<ClassId> = HashSet::default();
        for call in self.facts.calls.iter().filter(|c| project(c)) {
            if let Some(class) = call.constructs
                && !call.logging.as_ref().is_some_and(|l| l.attach_only)
            {
                anywhere.insert(class);
            }
        }
        for flow in &self.facts.flows {
            if let Value::Class(class) = flow.value
                && self.facts.function(flow.caller).origin == Origin::Project
            {
                anywhere.insert(class);
            }
        }
        let mut attachments: HashMap<ClassId, Vec<&Call>> = HashMap::default();
        for call in self.facts.calls.iter().filter(|c| project(c)) {
            if let Some(class) = call.logging.as_ref().and_then(|l| l.attaches) {
                attachments.entry(class).or_default().push(call);
            }
        }
        let mut scopes: HashMap<ClassId, LogScope> = HashMap::default();
        // Formatters and filters on a handler run for the loggers of the handler: handlers first.
        let mut classes: Vec<ClassId> = attachments.keys().copied().collect();
        classes.sort_unstable_by_key(|&c| (!handler.is_some_and(|h| self.inherits(c, h)), c));
        for class in classes {
            let scope = if anywhere.contains(&class) {
                None
            } else {
                let mut loggers = Vec::new();
                for call in &attachments[&class] {
                    match self.attachment_scope(call, &scopes, handler) {
                        Some(on) => loggers.extend(on),
                        None => {
                            loggers.clear();
                            break;
                        }
                    }
                }
                (!loggers.is_empty()).then_some(loggers)
            };
            scopes.insert(class, scope);
        }
        scopes
    }

    /// The loggers what an `addHandler(...)`, `addFilter(...)` or `setFormatter(...)` call
    /// attaches runs for: a handler runs for its logger's records and those propagating to it, a
    /// logger's filter only for records logged on that logger, a handler's filter or formatter
    /// where the handler runs.
    fn attachment_scope(&self, call: &Call, scopes: &HashMap<ClassId, LogScope>, handler: Option<ClassId>) -> LogScope {
        let logging = call.logging.as_ref()?;
        let method = call.targets.iter().map(|&t| self.facts.function(t).qualname.as_str()).next()?;
        match (method, &logging.logger) {
            ("logging.Logger.addHandler", Some(logger)) if logger.exact => Some(vec![(logger.name.clone(), true)]),
            ("logging.Filterer.addFilter", Some(logger)) if logger.exact => Some(vec![(logger.name.clone(), false)]),
            ("logging.Filterer.addFilter" | "logging.Handler.setFormatter", None) => {
                let receiver = call.receiver.filter(|&r| handler.is_some_and(|h| self.inherits(r, h)))?;
                scopes.get(&receiver).cloned().flatten()
            }
            _ => None,
        }
    }

    /// Whether a hook attached for `scope` runs for a record logged on `logger` (`None`: not
    /// known, any logger). See `Logger.callHandlers`: the handlers of the logger and of its
    /// ancestors, up to the first logger that does not propagate.
    fn log_reaches(&self, scope: &LogScope, logger: Option<&LoggerName>) -> bool {
        let (Some(scope), Some(logger)) = (scope, logger) else { return true };
        let under = |name: &str, ancestor: &str| {
            ancestor.is_empty() || name == ancestor || name.strip_prefix(ancestor).is_some_and(|r| r.starts_with('.'))
        };
        // Whether the logger is (exactly or only maybe) `name` or below it.
        let surely_under = |name: &str| {
            if logger.exact {
                under(&logger.name, name)
            } else {
                name.is_empty() || logger.name.strip_prefix(name).is_some_and(|r| r.starts_with('.'))
            }
        };
        let maybe_under = |name: &str| surely_under(name) || (!logger.exact && name.starts_with(&logger.name));
        scope.iter().any(|(on, propagated)| {
            if !propagated {
                return if logger.exact { logger.name == *on } else { on.starts_with(&logger.name) };
            }
            maybe_under(on)
                && !self.log_stops.iter().any(|stop| stop != on && under(stop, on) && surely_under(stop))
        })
    }

    /// The lowest level project code (outside tests) sets on a logger: `setLevel(...)`,
    /// `basicConfig(level=...)`, anything for a level that is not known or for `dictConfig` /
    /// `fileConfig`. Without any, the root logger's default, `WARNING`.
    fn logger_threshold(&self) -> u32 {
        let mut threshold: Option<u32> = None;
        for call in &self.facts.calls {
            let caller = self.facts.function(call.caller);
            if caller.origin != Origin::Project || is_test_path(&caller.location.path) {
                continue;
            }
            for &t in &call.targets {
                let given = call.level.as_ref().map(|l| l.value.unwrap_or(0));
                let level = match self.facts.function(t).qualname.as_str() {
                    "logging.Logger.setLevel" | "logging.LoggerAdapter.setLevel" => given.unwrap_or(0),
                    "logging.basicConfig" => match given {
                        Some(level) => level,
                        None => continue,
                    },
                    "logging.config.dictConfig" | "logging.config.fileConfig" => 0,
                    _ => continue,
                };
                threshold = Some(threshold.map_or(level, |t| t.min(level)));
            }
        }
        threshold.unwrap_or(30)
    }

    /// The loggers that do not propagate their records to their ancestors' handlers: `propagate`
    /// is only ever set false on them.
    fn logger_stops(&self) -> Vec<String> {
        let mut stops: Vec<String> = Vec::new();
        for (name, _) in self.facts.stops_propagation.iter().filter(|(_, stops)| *stops) {
            if !self.facts.stops_propagation.iter().any(|(n, stops)| n == name && !stops) && !stops.contains(name) {
                stops.push(name.clone());
            }
        }
        stops
    }

    /// The level of the handlers of each class, when the project gives it: `setLevel(...)` on an
    /// instance of the class (`self.setLevel(...)` included) or the level its constructor passes
    /// to `Handler.__init__`, else the `level=` it is constructed with; the lowest when several.
    fn handler_levels(&self) -> HashMap<ClassId, u32> {
        let (mut set, mut constructed): (HashMap<ClassId, u32>, HashMap<ClassId, u32>) = Default::default();
        let lower = |map: &mut HashMap<ClassId, u32>, class: ClassId, level: u32| {
            map.entry(class).and_modify(|l| *l = (*l).min(level)).or_insert(level);
        };
        for call in &self.facts.calls {
            let value = call.level.as_ref().map_or(0, |l| l.value.unwrap_or(0));
            if let Some(class) = call.constructs {
                lower(&mut constructed, class, value);
            }
            for &t in &call.targets {
                match self.facts.function(t).qualname.as_str() {
                    "logging.Handler.setLevel" => {
                        if let Some(class) = call.level.as_ref().and_then(|l| l.receiver) {
                            lower(&mut set, class, value);
                        }
                    }
                    // `super().__init__(level=...)` in the handler's own constructor.
                    "logging.Handler.__init__" if call.constructs.is_none() && call.level.is_some() => {
                        if let Some(class) = self.facts.function(call.caller).class {
                            lower(&mut set, class, value);
                        }
                    }
                    _ => {}
                }
            }
        }
        constructed.extend(set);
        constructed
    }

    /// Whether `class` is `base` or a subclass of it.
    fn inherits(&self, class: ClassId, base: ClassId) -> bool {
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            if c == base {
                return true;
            }
            queue.extend(self.facts.class(c).bases.iter().filter(|b| seen.insert(**b)));
        }
        false
    }

    /// Pydantic (and response class) code FastAPI runs on the loop around endpoints and dependencies, with what runs
    /// it. `solve_dependencies` validates the request parameters of every endpoint and
    /// dependency on the loop, sync ones included (only their bodies go to the threadpool).
    /// `serialize_response` validates the return value against the response model on the loop
    /// for `async def` endpoints only, then dumps it on the loop for every endpoint.
    /// The response class is constructed on the loop for every endpoint.
    fn framework_hooks(&self) -> Vec<(FnId, String)> {
        let signatures: HashMap<FnId, &Signature> = self.facts.signatures.iter().map(|s| (s.function, s)).collect();
        let mut response_models: HashMap<(FnId, &Location), Vec<ClassId>> = HashMap::default();
        for flow in &self.facts.flows {
            if let (Value::Class(class), Slot::Param(slot)) = (&flow.value, &flow.slot)
                && slot == "response_model"
            {
                response_models.entry((flow.caller, &flow.location)).or_default().push(*class);
            }
        }
        let mut hooks = Vec::new();
        for flow in &self.facts.flows {
            let into = &self.facts.function(flow.into).qualname;
            let kind = self.catalog.entry_kind(into);
            let slot = |name: &str| matches!(&flow.slot, Slot::Param(p) if p == name);
            // `response_class=` on a route (or `default_response_class=` on the app, a router or
            // `include_router`): `get_request_handler` constructs it on the loop after the
            // endpoint returns, sync endpoints included, and Starlette's `Response.__init__`
            // calls `render` and `init_headers` synchronously.
            if let Value::Class(class) = flow.value
                && into.starts_with("fastapi.")
                && (slot("response_class") || slot("default_response_class"))
            {
                let label = match kind {
                    Some(kind @ "route") => entry_label(kind, into, flow),
                    _ => "default response class".to_string(),
                };
                let methods = ["__init__", "render", "init_headers"].map(|m| Hierarchy::lookup(self.facts, class, m));
                hooks.extend(methods.into_iter().flatten().map(|h| (h, format!("{label} (response rendering)"))));
                continue;
            }
            // `add_middleware(cls, ...)` and `Middleware(cls, ...)` only record the factory:
            // `build_middleware_stack` calls `cls(app, ...)` lazily, in `Starlette.__call__` on
            // the first lifespan or request event, on the loop.
            if kind == Some("middleware")
                && into.starts_with("starlette.")
                && (slot("middleware_class") || slot("cls"))
            {
                let factory = match flow.value {
                    Value::Class(class) => Hierarchy::lookup(self.facts, class, "__init__"),
                    Value::Function(f) => Some(f),
                    Value::Param(..) => None,
                };
                let label = entry_label("middleware", into, flow);
                hooks.extend(factory.map(|f| (f, format!("{label} (construction)"))));
                continue;
            }
            let is_route =
                matches!(kind, Some("route" | "websocket")) && (flow.slot == Slot::Decorated || slot("endpoint"));
            let is_dependency = kind == Some("dependency") && slot("dependency");
            // Starlette routes neither validate nor serialize.
            if !into.starts_with("fastapi.") || (!is_route && !is_dependency) {
                continue;
            }
            let function = match flow.value {
                Value::Function(f) => f,
                // A class dependency: FastAPI fills the parameters of its `__init__`.
                Value::Class(class) if is_dependency => {
                    let Some(init) = Hierarchy::lookup(self.facts, class, "__init__") else { continue };
                    init
                }
                _ => continue,
            };
            let none =
                Signature { function, params: Vec::new(), annotation: Vec::new(), returned: Vec::new(), returns_unknown: false };
            let signature = signatures.get(&function).copied().unwrap_or(&none);
            let label = entry_label(kind.unwrap_or_default(), into, flow);
            let mut add = |found: Vec<FnId>, what: &str| hooks.extend(found.into_iter().map(|h| (h, format!("{label} ({what})"))));
            for value in &signature.params {
                match value {
                    Value::Class(class) => add(self.validation_hooks(*class), "request validation"),
                    Value::Function(f) => add(vec![*f], "request validation"),
                    Value::Param(..) => {}
                }
            }
            if kind != Some("route") {
                continue;
            }
            let declared = response_models.get(&(flow.caller, &flow.location));
            let models = declared.map_or(&signature.annotation[..], |m| &m[..]);
            // Without a response model, `jsonable_encoder` dumps whatever is returned.
            let dumped = if models.is_empty() { &signature.returned[..] } else { models };
            for &class in dumped {
                add(self.serialization_hooks(class), "response serialization");
            }
            // Validating an instance of the model (or of a subclass) keeps it as is.
            if self.facts.function(function).is_async && !signature.returns_unknown {
                let validated = models.iter().filter(|&&m| !signature.returned.iter().any(|&r| self.inherits(r, m)));
                for &class in validated {
                    add(self.construction_hooks(class), "response validation");
                }
            }
        }
        hooks.retain(|(h, _)| self.facts.function(*h).analyzed);
        hooks
    }

    /// Whether decorating with `decorator` keeps calling the original on the loop: one of its
    /// wrappers calls the decorated parameter, or hands it to something other than an offload API
    /// (`asyncio.to_thread(func)` does not run it here) or an inspector (`functools.wraps(func)`
    /// copies its metadata). Undecorated library decorators count as calling it.
    fn calls_original(&self, decorator: FnId, nested: &[Vec<FnId>]) -> bool {
        if !self.facts.function(decorator).analyzed {
            return true;
        }
        self.slot_params(decorator, &Slot::Decorated, nested).iter().any(|k| self.used_params.contains(k))
    }

    /// Whether a virtual call made in `caller` may dispatch to the override `method`: test doubles
    /// (overrides defined in test code) only stand in for real classes in test code, and classes
    /// defined inside a function only in their own file.
    fn dispatches_into(&self, caller: FnId, method: FnId) -> bool {
        let (caller, method) = (self.facts.function(caller), self.facts.function(method));
        (!is_test_path(&method.location.path) || is_test_path(&caller.location.path))
            && (!method.qualname.contains(".<locals>.") || method.location.path == caller.location.path)
    }

    /// Whether the sink `target` blocks when called at `call`: calls giving one of its
    /// `unless_given` arguments, or not known to leave them out, skip the blocking work.
    fn blocks_here(&self, call: usize, target: FnId) -> bool {
        let unless = self.unless_given[target.0 as usize];
        unless.is_empty() || self.facts.calls[call].given.as_ref().is_some_and(|g| !g.iter().any(|a| unless.contains(a)))
    }

    /// Whether calling `target` at `call` runs its body now: calling a generator function only
    /// creates the generator, unless the call site iterates it right away.
    fn runs_body(&self, call: usize, target: FnId) -> bool {
        !self.facts.function(target).is_generator || self.facts.calls[call].iterates
    }

    /// Loop code: every analyzed `async def`, plus sync callables scheduled on the loop and
    /// pydantic code FastAPI runs on the loop.
    fn roots(&self) -> Vec<FnId> {
        let mut roots: BTreeSet<FnId> = self
            .facts
            .functions
            .iter()
            .enumerate()
            .filter(|(_, f)| f.analyzed && f.is_async)
            .map(|(i, _)| FnId(i as u32))
            .collect();
        for flow in &self.facts.flows {
            // Starlette calls sync startup/shutdown handlers directly on the loop.
            let into = &self.facts.function(flow.into).qualname;
            let on_loop = self.catalog.is_loop_callback(into) || self.catalog.entry_kind(into) == Some("lifecycle");
            if let Value::Function(f) = flow.value
                && on_loop
                && self.facts.function(f).analyzed
            {
                roots.insert(f);
            }
        }
        roots.extend(self.hooks.iter().map(|(h, _)| *h));
        roots.into_iter().collect()
    }

    /// For each sync function that blocks when called: `(distance, next call, next function)`
    /// on a shortest chain to a blocking function.
    fn blocking_distances(&self) -> Vec<Option<(u32, usize, FnId)>> {
        let n = self.facts.functions.len();
        let mut next: Vec<Option<(u32, usize, FnId)>> = vec![None; n];
        let mut callers_of: Vec<Vec<(FnId, usize)>> = vec![Vec::new(); n];
        let mut queue = VecDeque::new();
        for (i, call) in self.facts.calls.iter().enumerate() {
            let caller = self.facts.function(call.caller);
            if caller.is_async || self.sink[call.caller.0 as usize].is_some() {
                continue;
            }
            for &t in &self.targets[i] {
                if self.direct[t.0 as usize] {
                    continue;
                }
                if self.sink[t.0 as usize].is_some() {
                    if !call.awaited && self.blocks_here(i, t) && next[call.caller.0 as usize].is_none() {
                        next[call.caller.0 as usize] = Some((1, i, t));
                        queue.push_back(call.caller);
                    }
                } else if !self.facts.function(t).is_async && self.runs_body(i, t) {
                    callers_of[t.0 as usize].push((call.caller, i));
                }
            }
        }
        while let Some(f) = queue.pop_front() {
            let (d, _, _) = next[f.0 as usize].expect("queued functions have a distance");
            for &(caller, call) in &callers_of[f.0 as usize] {
                if next[caller.0 as usize].is_none() {
                    next[caller.0 as usize] = Some((d + 1, call, f));
                    queue.push_back(caller);
                }
            }
        }
        next
    }

    /// The shortest blocking chain starting at `call`, and what blocks at its end.
    fn blocking_chain(&self, call: usize, next: &[Option<(u32, usize, FnId)>]) -> Option<(Vec<Frame>, Sink)> {
        let c = &self.facts.calls[call];
        let best = self.targets[call]
            .iter()
            .filter_map(|&t| {
                if self.sink[t.0 as usize].is_some() {
                    (!c.awaited && self.blocks_here(call, t)).then_some((0, t))
                } else if self.facts.function(t).is_async || !self.runs_body(call, t) {
                    None
                } else {
                    next[t.0 as usize].map(|(d, _, _)| (d, t))
                }
            })
            .min()?;
        let mut chain = vec![self.frame(call, best.1)];
        let mut current = best.1;
        loop {
            if let Some(sink) = &self.sink[current.0 as usize] {
                return Some((chain, sink.clone()));
            }
            let (_, call, target) = next[current.0 as usize].expect("blocking functions chain to a sink");
            chain.push(self.frame(call, target));
            current = target;
        }
    }

    fn frame(&self, call: usize, target: FnId) -> Frame {
        let c = &self.facts.calls[call];
        Frame {
            location: c.location.clone(),
            text: c.text.clone(),
            callee: self.facts.function(target).qualname.clone(),
        }
    }

    /// Whether `target` runs on the same thread as `caller` when called (a sync function calling
    /// an `async def` only creates a coroutine).
    fn runs_inline(&self, caller: FnId, target: FnId) -> bool {
        self.facts.function(caller).is_async || !self.facts.function(target).is_async
    }

    /// Shortest paths from FastAPI entry points (and, as a fallback, from project `async def`s)
    /// to every function they reach.
    fn entry_paths(&self) -> EntryPaths<'_> {
        let mut entries: Vec<(FnId, String)> = Vec::new();
        for flow in &self.facts.flows {
            let into = &self.facts.function(flow.into).qualname;
            let Some(kind) = self.catalog.entry_kind(into) else { continue };
            let label = entry_label(kind, into, flow);
            match flow.value {
                Value::Function(f) => entries.push((f, label)),
                Value::Class(class) => {
                    for method in ["dispatch", "__call__"] {
                        if let Some(m) = Hierarchy::lookup(self.facts, class, method) {
                            entries.push((m, format!("{label} ({method})")));
                        }
                    }
                }
                Value::Param(..) => {}
            }
        }
        entries.extend(self.hooks.iter().cloned());
        let project_async = self
            .facts
            .functions
            .iter()
            .enumerate()
            .filter(|(_, f)| f.analyzed && f.is_async && f.origin == Origin::Project)
            .map(|(i, _)| (FnId(i as u32), "project code".to_string()));
        let count = entries.len();
        let from_entries = self.bfs(entries);
        let from_project = self.bfs(project_async.collect());
        EntryPaths { graph: self, from_entries, from_project, entries: count }
    }

    fn bfs(&self, sources: Vec<(FnId, String)>) -> Vec<Option<Reach>> {
        let mut reach: Vec<Option<Reach>> = vec![None; self.facts.functions.len()];
        let mut queue = VecDeque::new();
        for (f, label) in sources {
            if reach[f.0 as usize].is_none() {
                reach[f.0 as usize] = Some(Reach::Entry(label));
                queue.push_back(f);
            }
        }
        while let Some(f) = queue.pop_front() {
            for &call in &self.calls_of[f.0 as usize] {
                for &t in &self.targets[call] {
                    if reach[t.0 as usize].is_none() && self.runs_inline(f, t) && self.runs_body(call, t) {
                        reach[t.0 as usize] = Some(Reach::Via(f, call));
                        queue.push_back(t);
                    }
                }
            }
        }
        reach
    }

    /// Calls on the event loop whose target is unknown, in project code.
    fn unresolved(&self, roots: &[FnId]) -> Vec<Unresolved> {
        let mut on_loop: Vec<bool> = vec![false; self.facts.functions.len()];
        let mut queue: VecDeque<FnId> = roots.iter().copied().collect();
        for r in roots {
            on_loop[r.0 as usize] = true;
        }
        while let Some(f) = queue.pop_front() {
            for &call in &self.calls_of[f.0 as usize] {
                for &t in &self.targets[call] {
                    let target = self.facts.function(t);
                    if !on_loop[t.0 as usize]
                        && !target.is_async
                        && self.sink[t.0 as usize].is_none()
                        && self.runs_body(call, t)
                    {
                        on_loop[t.0 as usize] = true;
                        queue.push_back(t);
                    }
                }
            }
        }
        let mut out = Vec::new();
        for (i, call) in self.facts.calls.iter().enumerate() {
            let caller = self.facts.function(call.caller);
            if !on_loop[call.caller.0 as usize] || caller.origin != Origin::Project || !self.targets[i].is_empty() {
                continue;
            }
            // Resolved at the call sites passing the callable.
            if call.param.as_ref().is_some_and(|k| self.called.contains(k)) {
                continue;
            }
            // Awaited calls never block; constructing a known class whose `__init__` is
            // synthesized (dataclasses, pydantic models) runs no code of its own.
            if call.awaited || call.constructs.is_some() {
                continue;
            }
            let reason = match (&call.param, &call.unresolved) {
                (Some((_, name)), _) => format!("parameter `{name}` receives no known callable"),
                (None, Some(reason)) => reason.clone(),
                // Resolved, but only to offload APIs.
                (None, None) => continue,
            };
            out.push(Unresolved {
                location: call.location.clone(),
                function: caller.qualname.clone(),
                text: call.text.clone(),
                reason,
            });
        }
        out
    }
}

/// Test code by the usual conventions: a `tests`/`test` directory, `test_*.py`, `*_test.py`,
/// `conftest.py`.
fn is_test_path(path: &str) -> bool {
    let mut parts = path.split('/').rev();
    let file = parts.next().unwrap_or_default();
    let test_file = file.starts_with("test_") || file.ends_with("_test.py") || file == "conftest.py";
    test_file || parts.any(|d| d == "tests" || d == "test")
}

fn entry_label(kind: &str, into: &str, flow: &crate::facts::Flow) -> String {
    let method = into.rsplit('.').next().unwrap_or_default();
    let detail = match (kind, method) {
        ("route", "get" | "put" | "post" | "delete" | "options" | "head" | "patch" | "trace") => {
            Some(method.to_uppercase())
        }
        ("lifecycle", _) => match &flow.slot {
            Slot::Param(p) => Some(p.clone()),
            _ => None,
        },
        _ => None,
    };
    [Some(kind.to_string()), detail, flow.literal.clone()].into_iter().flatten().collect::<Vec<_>>().join(" ")
}

/// Which logging calls run a hook: every record, or only records carrying an exception
/// (`logger.exception(...)`; `exc_info=` is not tracked).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum LogCall {
    Record,
    Exception,
}

/// See [`Graph::logging_hooks`].
struct LogHook {
    function: FnId,
    /// Attached outside test code.
    outside_tests: bool,
    runs_for: LogCall,
    /// The lowest record level it runs for (a handler's level).
    level: u32,
    scope: LogScope,
}

/// The loggers a logging hook runs for, `None` for any: `(logger, propagated)`, where a hook
/// that is not `propagated` (a logger's own filter) only runs for records logged on that logger.
type LogScope = Option<Vec<(String, bool)>>;

#[derive(Clone)]
enum Reach {
    Entry(String),
    Via(FnId, usize),
}

struct EntryPaths<'a> {
    graph: &'a Graph<'a>,
    from_entries: Vec<Option<Reach>>,
    from_project: Vec<Option<Reach>>,
    entries: usize,
}

/// A shortest path from an entry point (or project code) to a function.
struct Steps {
    label: Option<String>,
    entry: FnId,
    /// `(caller, call, callee)`, from the entry onwards.
    steps: Vec<(FnId, usize, FnId)>,
}

impl Steps {
    /// Whether the path starts at a FastAPI entry point (not merely at project code).
    fn starts_at_entry_point(&self) -> bool {
        self.label.is_some()
    }
}

impl EntryPaths<'_> {
    /// How entry points reach `f`. Library functions no entry point reaches are attributed to
    /// the project code that reaches them (`label` is then `None`).
    fn steps_to(&self, f: FnId) -> Option<Steps> {
        let (reach, from_entry) = if self.from_entries[f.0 as usize].is_some() {
            (&self.from_entries, true)
        } else if self.graph.facts.function(f).origin != Origin::Project
            && self.from_project[f.0 as usize].is_some()
        {
            (&self.from_project, false)
        } else {
            return None;
        };
        let mut steps = Vec::new();
        let mut current = f;
        loop {
            match reach[current.0 as usize].as_ref().expect("reached") {
                Reach::Via(prev, call) => {
                    steps.push((*prev, *call, current));
                    current = *prev;
                }
                Reach::Entry(label) => {
                    steps.reverse();
                    return Some(Steps { label: from_entry.then(|| label.clone()), entry: current, steps });
                }
            }
        }
    }

    /// The first `upto` steps of a path from an entry point.
    fn reached_from(&self, steps: &Steps, upto: usize) -> ReachedFrom {
        let entry = self.graph.facts.function(steps.entry);
        let label = steps.label.clone().unwrap_or_default();
        ReachedFrom {
            entry: if entry.kind == FunctionKind::Module { label } else { format!("{label}: {}", entry.qualname) },
            location: entry.location.clone(),
            frames: steps.steps[..upto].iter().map(|&(_, call, target)| self.graph.frame(call, target)).collect(),
        }
    }
}

/// Class hierarchy analysis: which methods may run for `obj.method()`.
struct Hierarchy {
    subclasses: Vec<Vec<ClassId>>,
    /// Classes with more than one base somewhere in their ancestry: the only ones whose MRO can
    /// send a `super()` call somewhere the calling class's own bases do not lead.
    cooperative: Vec<bool>,
    /// The names each function calls through `super().name(...)`.
    supers: HashMap<FnId, Vec<String>>,
    /// Memoized [`Hierarchy::mro`].
    mros: RefCell<HashMap<ClassId, Rc<[ClassId]>>>,
}

impl Hierarchy {
    fn new(facts: &Facts) -> Self {
        let mut subclasses = vec![Vec::new(); facts.classes.len()];
        for (i, class) in facts.classes.iter().enumerate() {
            for base in &class.bases {
                subclasses[base.0 as usize].push(ClassId(i as u32));
            }
        }
        // Propagated down from the classes with several bases.
        let mut cooperative: Vec<bool> = facts.classes.iter().map(|c| c.bases.len() > 1).collect();
        let mut queue: VecDeque<ClassId> =
            (0..facts.classes.len()).filter(|&i| cooperative[i]).map(|i| ClassId(i as u32)).collect();
        while let Some(c) = queue.pop_front() {
            for &sub in &subclasses[c.0 as usize] {
                if !cooperative[sub.0 as usize] {
                    cooperative[sub.0 as usize] = true;
                    queue.push_back(sub);
                }
            }
        }
        let mut supers: HashMap<FnId, Vec<String>> = HashMap::default();
        for call in &facts.calls {
            if let Some(name) = &call.super_name {
                supers.entry(call.caller).or_default().push(name.clone());
            }
        }
        Self { subclasses, cooperative, supers, mros: RefCell::default() }
    }

    /// What else calling `method` on an instance of `receiver` runs through cooperative
    /// `super()` calls: while the method running calls `super().name(...)` (its own name), the
    /// next `name` in the MRO of `receiver`. `class ItemRepository(AuditMixin, Repository)`:
    /// `AuditMixin.save` calling `super().save(...)` runs `Repository.save`, though `AuditMixin`
    /// itself has no base defining `save`.
    fn super_chain(&self, facts: &Facts, receiver: ClassId, method: FnId) -> Vec<FnId> {
        let calls_super = |m: FnId, name: &str| self.supers.get(&m).is_some_and(|n| n.iter().any(|n| n == name));
        let Some(class) = facts.function(method).class else { return Vec::new() };
        let Some((name, _)) = facts.class(class).methods.iter().find(|(_, m)| *m == method) else {
            return Vec::new();
        };
        if !self.cooperative[receiver.0 as usize] || !calls_super(method, name) {
            return Vec::new();
        }
        let mro = self.mro(facts, receiver);
        let Some(start) = mro.iter().position(|&c| c == class) else { return Vec::new() };
        let mut chain = Vec::new();
        for &c in &mro[start + 1..] {
            if let Some((_, next)) = facts.class(c).methods.iter().find(|(n, _)| n == name) {
                chain.push(*next);
                if !calls_super(*next, name) {
                    break;
                }
            }
        }
        chain
    }

    /// The C3 linearization of `class` over the indexed bases (a depth-first order when the
    /// bases admit none).
    fn mro(&self, facts: &Facts, class: ClassId) -> Rc<[ClassId]> {
        if let Some(mro) = self.mros.borrow().get(&class) {
            return mro.clone();
        }
        // Cycles cannot happen in valid code; this keeps a broken hierarchy finite.
        self.mros.borrow_mut().insert(class, Rc::from([class]));
        let bases = &facts.class(class).bases;
        let mut sequences: Vec<Vec<ClassId>> = bases.iter().map(|&b| self.mro(facts, b).to_vec()).collect();
        sequences.push(bases.clone());
        let mut mro = vec![class];
        loop {
            sequences.retain(|s| !s.is_empty());
            if sequences.is_empty() {
                break;
            }
            let head = sequences
                .iter()
                .map(|s| s[0])
                .find(|h| !sequences.iter().any(|s| s[1..].contains(h)))
                .unwrap_or(sequences[0][0]);
            if !mro.contains(&head) {
                mro.push(head);
            }
            for s in &mut sequences {
                s.retain(|c| *c != head);
            }
        }
        let mro: Rc<[ClassId]> = mro.into();
        self.mros.borrow_mut().insert(class, mro.clone());
        mro
    }

    /// Methods overriding `method` in subclasses of its class; for protocols defined in the
    /// project, the same-named methods of every class that defines all of the protocol's methods.
    /// Only analyzed methods count: stub-only classes (stdlib) are never dispatch targets, which
    /// keeps generic protocols like `AbstractContextManager` from matching every lock.
    fn overrides(&self, facts: &Facts, method: FnId) -> Vec<FnId> {
        let Some(class) = facts.function(method).class else { return Vec::new() };
        let Some(name) = facts.class(class).methods.iter().find(|(_, m)| *m == method).map(|(n, _)| n)
        else {
            return Vec::new();
        };
        let named = |c: &crate::facts::Class| -> Vec<FnId> {
            c.methods.iter().filter(|(n, m)| n == name && facts.function(*m).analyzed).map(|(_, m)| *m).collect()
        };
        let mut out = Vec::new();
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            for &sub in &self.subclasses[c.0 as usize] {
                if seen.insert(sub) {
                    out.extend(named(facts.class(sub)));
                    queue.push_back(sub);
                }
            }
        }
        let protocol = facts.class(class);
        if protocol.is_protocol && facts.function(method).origin == Origin::Project {
            let required: Vec<&String> = protocol.methods.iter().map(|(n, _)| n).collect();
            for implementer in facts.classes.iter().filter(|c| !c.is_protocol) {
                let defines = |n: &String| implementer.methods.iter().any(|(m, _)| m == n);
                if required.iter().all(|n| defines(n)) {
                    out.extend(named(implementer));
                }
            }
        }
        out
    }

    /// `class.name` through the bases (breadth-first approximation of the MRO).
    fn lookup(facts: &Facts, class: ClassId, name: &str) -> Option<FnId> {
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            let cls = facts.class(c);
            if let Some((_, m)) = cls.methods.iter().find(|(n, _)| n == name) {
                return Some(*m);
            }
            queue.extend(cls.bases.iter().filter(|b| seen.insert(**b)));
        }
        None
    }
}
