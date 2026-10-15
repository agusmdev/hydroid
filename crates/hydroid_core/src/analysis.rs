//! From facts to diagnostics.
//!
//! Where code runs is a property of the *call*, not of the function: a sync function runs on the
//! event loop when called from code on the loop, and in a worker thread when handed to
//! `asyncio.to_thread`. So "does calling `f` block?" has one answer per function, computed once
//! by a breadth-first search backwards from blocking calls (shortest witness chains, O(V+E), no
//! recursion, any depth). Diagnostics are the call sites inside loop code (`async def` bodies and
//! callbacks scheduled on the loop) whose callee blocks.

mod specialize;

use std::cell::RefCell;
use std::collections::{BTreeSet, VecDeque};
use std::rc::Rc;

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::catalog::{Catalog, Entry, Observer, ObserverClass, Roles};
use crate::facts::{
    Call, ClassId, Dump, Facts, FnId, Flow, FunctionKind, Location, LoggerName, Origin, ResponseOptions, Signature, Slot,
    Value,
};
use crate::report::{Diagnostic, Frame, ReachedFrom, Report, Severity, Sink, Unresolved};

pub fn analyze(mut facts: Facts, catalog: &Catalog) -> Report {
    // Copies (see `specialize`) come after the extracted functions and calls.
    let functions_analyzed = facts.functions.iter().filter(|f| f.analyzed).count();
    let call_sites = facts.calls.len();
    specialize::specialize(&mut facts);
    let report = report(&facts, catalog, functions_analyzed, call_sites);
    // Freeing the facts of a large project takes tens of milliseconds: off the critical path.
    std::thread::spawn(move || drop(facts));
    report
}

fn report(facts: &Facts, catalog: &Catalog, functions_analyzed: usize, call_sites: usize) -> Report {
    let graph = Graph::new(facts, catalog);
    // The chain of a diagnostic leads to its worst sink: a call reaching a file check in one
    // step and an HTTP request in three is critical, and shows the request.
    let critical = graph.blocking_distances(Severity::Critical);
    let blocking = graph.blocking_distances(Severity::Warn);
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
        let runs_once = context.runs_once[root.0 as usize];
        for &call in &graph.calls_of[root.0 as usize] {
            let Some((chain, sink)) = graph
                .blocking_chain(call, &critical, Severity::Critical)
                .or_else(|| graph.blocking_chain(call, &blocking, Severity::Warn))
            else {
                continue;
            };
            let location = &facts.calls[entered_at.unwrap_or(call)].location;
            if suppressed.contains(&(location.path.as_str(), location.line)) {
                continue;
            }
            diagnostics.push(Diagnostic {
                location: location.clone(),
                function: function.clone(),
                severity: if runs_once { Severity::Warn } else { sink.severity },
                sink,
                chain: prefix.iter().cloned().chain(chain).collect(),
                reached_from: reached.clone(),
                runs_once,
            });
        }
    }
    // A call site reached from several roots keeps its worst severity.
    let key = |d: &Diagnostic| (d.location.clone(), d.sink.qualname.clone(), std::cmp::Reverse(d.severity));
    diagnostics.sort_by_cached_key(key);
    diagnostics.dedup_by(|a, b| a.location == b.location && a.sink.qualname == b.sink.qualname);

    let mut report = Report { diagnostics, unresolved: graph.unresolved(&roots), ..Report::default() };
    report.stats.entry_points = context.entries;
    report.stats.functions_analyzed = functions_analyzed;
    report.stats.call_sites = call_sites;
    report.stats.call_sites_resolved = (0..call_sites)
        .filter(|&i| !graph.targets[i].is_empty() || facts.calls[i].param.is_some() || graph.offloaded[i])
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
    /// Calls computing the elements of a lazy iterable handed to an offload API (see
    /// [`Call::lazy_in`]): they run in its worker, never here.
    offloaded: Vec<bool>,
    /// Sinks that only count when called directly from loop code, by function.
    direct: Vec<bool>,
    /// Blocking decorators (not sinks themselves), by function: calling the wrapper one returns
    /// when applied by hand blocks (see [`Graph::blocks_here`]), applying it does not.
    wrapper: Vec<bool>,
    /// Arguments that make a call to a sink not block (catalog `unless_given`), by function.
    unless_given: Vec<&'a [String]>,
    /// Parameters that are called, or handed to something other than an offload API or a
    /// function that only inspects them.
    used_params: HashSet<ParamKey>,
    /// See [`Graph::framework_hooks`].
    hooks: Vec<(FnId, String)>,
    /// See [`Graph::logging_hooks`].
    log_hooks: Vec<LogHook>,
    /// Catalog `[[observer]]` dispatches running them, by function (empty without hooks): the
    /// group, what they dispatch, and at which level (`None`: the level is an argument,
    /// `Logger.log`).
    log_dispatch: Vec<Option<(usize, LogCall, Option<u32>)>>,
    /// The lowest level the project sets on a registry, by group (see
    /// [`Graph::logger_threshold`]).
    log_threshold: Vec<u32>,
    /// See [`Graph::logger_stops`].
    log_stops: Vec<String>,
    /// Memoized [`Graph::validation_hooks`].
    validated: RefCell<HashMap<ClassId, Rc<[FnId]>>>,
    /// Memoized [`Graph::ancestors`].
    ancestry: RefCell<HashMap<ClassId, Rc<[ClassId]>>>,
    /// Memoized [`Graph::construction_hooks`].
    constructed: RefCell<HashMap<ClassId, Rc<[FnId]>>>,
    /// What the catalog says about each function.
    roles: Vec<Roles>,
    /// Classes that may have instances of their own, by class (see
    /// [`crate::facts::Class::referenced`]).
    instantiable: Vec<bool>,
    /// Memoized [`Graph::instantiable_below`].
    below: RefCell<HashMap<ClassId, Rc<[ClassId]>>>,
    /// Functions defined in test code (see [`is_test_path`]).
    in_tests: Vec<bool>,
    /// Memoized [`Graph::template_hooks`].
    templates: RefCell<HashMap<TemplateKey, Rc<[FnId]>>>,
}

type ParamKey = (FnId, String);

/// A library method and the class of the instance it runs on.
type TemplateKey = (FnId, ClassId);

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
        let roles: Vec<Roles> = facts.functions.iter().map(|f| catalog.roles(&f.qualname)).collect();
        let sink = facts
            .functions
            .iter()
            .zip(&roles)
            .map(|(f, &r)| {
                catalog
                    .sink_of(r, &f.qualname)
                    .or_else(|| {
                        let mut decorators = f.decorators.iter();
                        (!f.is_async).then(|| {
                            decorators.find_map(|d| {
                                catalog.blocking_decorator_of(roles[d.0 as usize], &facts.function(*d).qualname)
                            })
                        })?
                    })
                    .or_else(|| inherited_sink(facts, catalog, &roles, f))
                    .or_else(|| catalog.blocking_decorator_of(r, &f.qualname))
            })
            .collect();
        let wrapper = roles.iter().map(|r| !r.is_sink() && r.is_blocking_decorator()).collect();
        let offload = roles.iter().map(|r| r.offload).collect();
        let direct = roles.iter().map(|&r| catalog.is_direct_sink_of(r)).collect();
        let mut graph = Self {
            facts,
            catalog,
            calls_of,
            targets: Vec::new(),
            sink,
            called: HashSet::default(),
            offload,
            offloaded: Vec::new(),
            direct,
            wrapper,
            unless_given: roles.iter().map(|&r| catalog.unless_given_of(r)).collect(),
            used_params: HashSet::default(),
            hooks: Vec::new(),
            log_hooks: Vec::new(),
            log_dispatch: Vec::new(),
            log_threshold: Vec::new(),
            log_stops: Vec::new(),
            validated: RefCell::default(),
            ancestry: RefCell::default(),
            constructed: RefCell::default(),
            roles,
            instantiable: Vec::new(),
            below: RefCell::default(),
            in_tests: facts.functions.iter().map(|f| is_test_path(&f.location.path)).collect(),
            templates: RefCell::default(),
        };
        // A class with no instances of its own may still have them through a registry its
        // ancestors keep (`__init_subclass__`).
        graph.instantiable = (0..facts.classes.len() as u32)
            .map(ClassId)
            .map(|c| {
                let class = facts.class(c);
                class.referenced
                    || class.is_protocol
                    || graph.ancestors(c).iter().any(|&a| facts.class(a).methods.iter().any(|(n, _)| n == "__init_subclass__"))
            })
            .collect();
        let used_from_calls = facts.calls.iter().filter_map(|c| c.param.clone());
        let used_from_flows: Vec<ParamKey> = facts
            .flows
            .iter()
            .filter(|f| !graph.is_offload(f.into) && !graph.roles[f.into.0 as usize].inspector)
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
            let observers = catalog.observers();
            // What a dispatch dispatches, by its last name (`levels`, `exception`).
            let kind = |group: usize, q: &str| {
                let observer = &observers[group];
                let method = q.rsplit('.').next().unwrap_or_default();
                let exception = observer.exception.iter().any(|e| e == method);
                let call = if exception { LogCall::Exception } else { LogCall::Record };
                (group, call, observer.levels.get(method).copied())
            };
            graph.log_dispatch = facts
                .functions
                .iter()
                .zip(&graph.roles)
                .map(|(f, &r)| catalog.dispatch_of(r).map(|group| kind(group, &f.qualname)))
                .collect();
            graph.log_threshold = observers.iter().map(|o| graph.logger_threshold(o)).collect();
            graph.log_stops = graph.logger_stops();
        }
        let offloading: HashSet<(FnId, u32, u32)> = facts
            .calls
            .iter()
            .filter(|c| c.targets.iter().any(|t| graph.is_offload(*t)))
            .map(|c| (c.caller, c.location.line, c.location.column))
            .collect();
        graph.offloaded = facts
            .calls
            .iter()
            .map(|c| c.lazy_in.is_some_and(|(line, column)| offloading.contains(&(c.caller, line, column))))
            .collect();
        let mut runs = HashMap::default();
        graph.targets = facts
            .calls
            .iter()
            .enumerate()
            .map(|(i, call)| {
                if graph.offloaded[i] {
                    return Vec::new();
                }
                let at = (call.caller, call.location.line, call.location.column);
                let flows = flows_at.get(&at).map_or(&[][..], |f| &f[..]);
                graph.effective_targets(call, flows, &cx, &mut runs)
            })
            .collect();
        graph.hooks = graph.framework_hooks();
        graph
    }

    fn entry_kind(&self, f: FnId) -> Option<&str> {
        self.catalog.entry_kind_of(self.roles[f.0 as usize])
    }

    fn entry(&self, f: FnId) -> Option<&'a Entry> {
        self.catalog.entry_of(self.roles[f.0 as usize])
    }

    /// Whether a flow into an entry function passes what it registers (catalog `[[entry]]`
    /// `slot`): what a decorator decorates, or an argument at one of the slots.
    fn registers(&self, flow: &Flow) -> bool {
        let slots = self.entry(flow.into).map_or(&[][..], |e| &e.slot[..]);
        match &flow.slot {
            Slot::Decorated => true,
            Slot::Param(p) => slots.is_empty() || slots.contains(p),
            Slot::Unknown => slots.is_empty(),
        }
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
                    // The first parameter after the receiver.
                    let first = f.params.get(usize::from(f.takes_receiver))?;
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
        let narrowed = if call.virtual_dispatch { self.narrowed_targets(call, hierarchy) } else { None };
        let (targets, virtual_dispatch) = match &narrowed {
            Some(narrowed) => (&narrowed[..], false),
            None => (&call.targets[..], call.virtual_dispatch),
        };
        for &t in targets {
            // A copy for one kind of `self` (see `specialize`) has the original's overrides.
            let original = self.facts.function(t).specializes.unwrap_or(t);
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
                (body, hierarchy.overrides(self.facts, original))
            });
            out.extend(body.iter().copied());
            out.extend(dispatches.get(&original).into_iter().flatten());
            // Awaiting a library coroutine method runs, on the loop, the project overrides of
            // the hooks it calls on the instance.
            if let Some(receiver) = call.receiver
                && self.facts.function(call.caller).is_async
            {
                out.extend(self.template_hooks(original, receiver).iter().copied());
            }
            if virtual_dispatch {
                // An instance of a known class runs only the overrides of its subclasses.
                let receiver = call.receiver.filter(|r| !self.facts.class(*r).is_protocol);
                let below = |o: FnId| {
                    receiver.is_none_or(|r| self.facts.function(o).class.is_some_and(|c| self.inherits(c, r)))
                };
                out.extend(overrides.iter().copied().filter(|&o| below(o) && self.dispatches_into(call.caller, o)));
            }
            if let Some(receiver) = call.receiver {
                out.extend(hierarchy.super_chain(self.facts, receiver, original));
            }
        }
        // A dispatch runs the observers attached (logging: the filters and handlers).
        let logs = call.targets.iter().find_map(|t| self.log_dispatch.get(t.0 as usize).copied().flatten());
        if let Some((group, logs, fixed)) = logs {
            // A level that is not known counts as high enough.
            let level = fixed.or_else(|| call.level.as_ref()?.value).unwrap_or(u32::MAX);
            let in_tests = self.in_tests[call.caller.0 as usize];
            let logger = call.logging.as_ref().and_then(|l| l.logger.as_ref());
            if level >= self.log_threshold[group] {
                out.extend(
                    self.log_hooks
                        .iter()
                        .filter(|h| h.group == group)
                        .filter(|h| (h.outside_tests || in_tests) && h.runs_for <= logs && h.level <= level)
                        .filter(|h| self.log_reaches(group, &h.scope, logger))
                        .map(|h| h.function),
                );
            }
        }
        if let Some(class) = call.constructs {
            out.extend(self.construction_hooks(class).iter());
            if let Some(given) = &call.given {
                out.extend(self.default_factories(class, given));
            }
            let given = if call.unpacks { None } else { call.given.as_deref() };
            out.extend(self.field_hooks(class, given, &call.instances));
        }
        // A dump operation (`model.model_dump()`) runs the serialization hooks of the model.
        if let Some(class) = call.receiver
            && call.super_name.is_none()
            && let Some(dump) = &call.dump
        {
            out.extend(self.serialization_hooks(class, dump));
        }
        if let Some((class, names)) = &call.sets {
            out.extend(self.attribute_hooks(*class, names));
        }
        for f in &call.returned_by {
            if call.targets.is_empty() {
                out.extend(returned.get(f).into_iter().flatten());
            }
            // Calling what a blocking decorator applied by hand returns runs its wrapper.
            if self.wrapper[f.0 as usize] {
                out.push(*f);
            }
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
    fn construction_hooks(&self, class: ClassId) -> Rc<[FnId]> {
        if let Some(hooks) = self.constructed.borrow().get(&class) {
            return hooks.clone();
        }
        let mut hooks: Vec<FnId> = ["__post_init__", "model_post_init"]
            .iter()
            .filter_map(|name| Hierarchy::lookup(self.facts, class, name))
            .collect();
        for &c in self.ancestors(class).iter() {
            let cls = self.facts.class(c);
            hooks.extend(cls.methods.iter().map(|(_, m)| *m).filter(|m| self.facts.function(*m).is_validator));
        }
        let hooks: Rc<[FnId]> = hooks.into();
        self.constructed.borrow_mut().insert(class, hooks.clone());
        hooks
    }

    /// The default factories constructing `class` calls when it is given only the `given` fields
    /// (of the class and its bases). A class as factory runs its `__init__`.
    fn default_factories(&self, class: ClassId, given: &[String]) -> Vec<FnId> {
        let mut factories = Vec::new();
        for &c in self.ancestors(class).iter() {
            for (names, factory) in &self.facts.class(c).default_factories {
                if names.iter().any(|n| given.contains(n)) {
                    continue;
                }
                match factory {
                    Value::Function(f) => factories.push(*f),
                    Value::Class(k) => factories.extend(Hierarchy::lookup(self.facts, *k, "__init__")),
                    Value::Param(..) => {}
                }
            }
        }
        factories
    }

    /// The SQLAlchemy validators setting `names` on an instance of `class` runs (of the class and
    /// its bases).
    fn attribute_hooks(&self, class: ClassId, names: &[String]) -> Vec<FnId> {
        let mut hooks = Vec::new();
        for &c in self.ancestors(class).iter() {
            hooks.extend(
                self.facts
                    .class(c)
                    .methods
                    .iter()
                    .map(|(_, m)| *m)
                    .filter(|m| self.facts.function(*m).validates.iter().any(|v| names.contains(v))),
            );
        }
        hooks
    }

    /// `class` and its bases, transitively (breadth-first).
    fn ancestors(&self, class: ClassId) -> Rc<[ClassId]> {
        if let Some(found) = self.ancestry.borrow().get(&class) {
            return found.clone();
        }
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([class]);
        let mut out = Vec::new();
        while let Some(c) = queue.pop_front() {
            out.push(c);
            queue.extend(self.facts.class(c).bases.iter().filter(|b| seen.insert(**b)));
        }
        let out: Rc<[ClassId]> = out.into();
        self.ancestry.borrow_mut().insert(class, out.clone());
        out
    }

    /// What constructing `class` runs for the fields it is given (`None`: any field): their
    /// `Annotated` validators, and the validation of the data given into the models nested in
    /// them, unless they are given model instances (`instances`), which pydantic keeps as is.
    fn field_hooks(&self, class: ClassId, given: Option<&[String]>, instances: &[String]) -> Vec<FnId> {
        let mut hooks = Vec::new();
        for &c in self.ancestors(class).iter() {
            for field in &self.facts.class(c).model_fields {
                if given.is_some_and(|g| !field.names.iter().any(|n| g.contains(n))) {
                    continue;
                }
                hooks.extend(&field.validators);
                if !field.names.iter().any(|n| instances.contains(n)) {
                    for &nested in &field.classes {
                        hooks.extend(self.validation_hooks(nested).iter());
                    }
                }
            }
        }
        hooks
    }

    /// What validating `class` from data runs: its construction hooks and default factories
    /// (the data may leave those fields out), the `Annotated` validators of its fields, and those
    /// of the models nested in its fields.
    fn validation_hooks(&self, class: ClassId) -> Rc<[FnId]> {
        if let Some(hooks) = self.validated.borrow().get(&class) {
            return hooks.clone();
        }
        let mut hooks = Vec::new();
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            hooks.extend(self.construction_hooks(c).iter());
            hooks.extend(self.default_factories(c, &[]));
            for &a in self.ancestors(c).iter() {
                let cls = self.facts.class(a);
                hooks.extend(cls.model_fields.iter().flat_map(|f| &f.validators));
                let nested = cls.fields.iter().chain(cls.model_fields.iter().flat_map(|f| &f.classes));
                queue.extend(nested.filter(|f| seen.insert(**f)));
            }
        }
        hooks.sort_unstable();
        hooks.dedup();
        let hooks: Rc<[FnId]> = hooks.into();
        self.validated.borrow_mut().insert(class, hooks.clone());
        hooks
    }

    /// What dumping an instance of `class` runs: serialization hooks of the class, its bases and
    /// the models nested in its fields, and the `Annotated` serializers of their fields. Fields
    /// of `class` that `dump` leaves out are not dumped; a python-mode dump skips JSON-only
    /// serializers.
    fn serialization_hooks(&self, class: ClassId, dump: &Dump) -> Vec<FnId> {
        let mut hooks = Vec::new();
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([(class, true)]);
        while let Some((c, top)) = queue.pop_front() {
            let dumped = |name: &String| {
                !top || (!dump.exclude.contains(name) && dump.include.as_ref().is_none_or(|i| i.contains(name)))
            };
            for &a in self.ancestors(c).iter() {
                let cls = self.facts.class(a);
                let runs = |m: &FnId| {
                    let f = self.facts.function(*m);
                    f.is_serializer
                        && (dump.json || !f.json_only)
                        && (f.serializes.is_empty() || f.serializes.iter().any(dumped))
                };
                hooks.extend(cls.methods.iter().map(|(_, m)| *m).filter(runs));
                let mut nested: Vec<ClassId> = Vec::new();
                for field in cls.model_fields.iter().filter(|f| dumped(&f.names[0])) {
                    let json_only = |s: &&FnId| dump.json || !field.json_serializers.contains(s);
                    hooks.extend(field.serializers.iter().filter(json_only));
                    nested.extend(&field.classes);
                }
                // Other attributes holding models are dumped unless the fields are filtered.
                if !top || (dump.exclude.is_empty() && dump.include.is_none()) {
                    nested.extend(&cls.fields);
                }
                queue.extend(nested.into_iter().filter(|f| seen.insert(*f)).map(|f| (f, false)));
            }
        }
        hooks
    }

    /// What the dispatches of each catalog `[[observer]]` run on the calling thread besides
    /// library code (logging: `Logger.handle` runs the logger's filters, then `Handler.handle` of
    /// every handler: its filters, then `emit`, which formats the record). Observers count once
    /// project code attaches them, which constructing them (or passing the class) stands for. Each
    /// hook comes with its group, whether it is attached outside test code (test doubles only
    /// stand in for test code's dispatches), and the registries it runs for (see
    /// [`Graph::log_scopes`]).
    fn logging_hooks(&self) -> Vec<LogHook> {
        let mut hooks: Vec<LogHook> = Vec::new();
        for (group, observer) in self.catalog.observers().iter().enumerate() {
            hooks.extend(self.observer_hooks(group, observer));
        }
        hooks
    }

    /// See [`Graph::logging_hooks`].
    fn observer_hooks(&self, group: usize, observer: &Observer) -> Vec<LogHook> {
        let find = |name: &str| self.facts.classes.iter().position(|c| c.qualname == name).map(|i| ClassId(i as u32));
        let classes: Vec<(ClassId, &ObserverClass)> =
            observer.classes.iter().filter_map(|c| Some((find(&c.base)?, c))).collect();
        let callable: HashSet<&str> =
            observer.attach.iter().filter(|a| a.callable).flat_map(|a| a.functions.iter().map(String::as_str)).collect();
        if classes.is_empty() && callable.is_empty() {
            return Vec::new();
        }
        let scopes = self.log_scopes(observer, &classes);
        let mut attached: HashMap<ClassId, bool> = HashMap::default();
        let mut hooks: Vec<LogHook> = Vec::new();
        let mut attach = |class: ClassId, caller: FnId| {
            let outside_tests = !self.in_tests[caller.0 as usize];
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
                // A plain callable attached (`handler.addFilter(drop_health_checks)`).
                Value::Function(f) if callable.contains(self.facts.function(flow.into).qualname.as_str()) => {
                    let outside_tests = !self.in_tests[flow.caller.0 as usize];
                    let call = self.facts.calls.iter().find(|c| {
                        c.caller == flow.caller && c.location == flow.location && c.targets.contains(&flow.into)
                    });
                    let scope = match call {
                        Some(call) => self.attachment_scope(call, &scopes, observer, &classes),
                        None => None,
                    };
                    hooks.push(LogHook { function: f, group, outside_tests, runs_for: LogCall::Record, level: 0, scope });
                }
                _ => {}
            }
        }
        let levels = self.handler_levels(observer);
        let is = |class: ClassId, base: ClassId| self.inherits(class, base);
        for (&class, &outside_tests) in &attached {
            let scope = scopes.get(&class).cloned().flatten();
            for &(base, config) in classes.iter().filter(|&&(base, _)| is(class, base)) {
                // Observers given to an `offloaded_by` instance (`QueueListener`) run in its thread;
                // which ones are is not tracked.
                if attached.keys().any(|&c| config.offloaded_by.contains(&self.facts.class(c).qualname)) {
                    continue;
                }
                // The registry skips an observer whose level is above the record's.
                let level = if config.level.is_empty() { 0 } else { levels.get(&class).copied().unwrap_or(0) };
                let _ = base;
                hooks.extend(
                    self.observer_methods(class, config)
                        .into_iter()
                        .filter(|&(m, _)| self.facts.function(m).analyzed || self.sink[m.0 as usize].is_some())
                        .map(|(function, runs_for)| LogHook {
                            function,
                            group,
                            outside_tests,
                            runs_for,
                            level,
                            scope: scope.clone(),
                        }),
                );
            }
        }
        hooks.sort_unstable_by_key(|h| (h.function, h.outside_tests, h.runs_for, h.level));
        // The same method attached through several classes runs for the registries of each.
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

    /// What a dispatch runs on an observer of `class` (catalog `[[observer]]` `classes`): its
    /// `runs` method if the project overrides it, else what the library's method calls on `self`
    /// (`calls`), recursively; methods with nothing listed run as they are (the library ones
    /// count only as sinks).
    fn observer_methods(&self, class: ClassId, config: &ObserverClass) -> Vec<(FnId, LogCall)> {
        let mut found = Vec::new();
        let mut stack = vec![config.runs.as_str()];
        let mut seen: HashSet<&str> = HashSet::default();
        while let Some(name) = stack.pop() {
            if !seen.insert(name) {
                continue;
            }
            let method = Hierarchy::lookup(self.facts, class, name);
            let overridden = method.is_some_and(|m| self.facts.function(m).analyzed);
            match config.calls.get(name) {
                Some(calls) if !overridden => stack.extend(calls.iter().rev().map(String::as_str)),
                _ => {
                    let runs_for =
                        if config.exception_only.iter().any(|e| e == name) { LogCall::Exception } else { LogCall::Record };
                    found.extend(method.map(|m| (m, runs_for)));
                }
            }
        }
        found
    }

    /// The registries the observers of each class attached in project code run for (`None`:
    /// any). A class counts as attached only where project code attaches it when every instance
    /// it constructs is only attached ([`crate::facts::LogUse::attach_only`]) and the class itself
    /// is not passed on (`dictConfig`, `handlers=[...]` lists and the like attach it anywhere).
    fn log_scopes(&self, observer: &Observer, classes: &[(ClassId, &ObserverClass)]) -> HashMap<ClassId, LogScope> {
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
        let mut ordered: Vec<ClassId> = attachments.keys().copied().collect();
        ordered.sort_unstable();
        // Observers attached to an observer (a formatter on a handler) run where that one runs:
        // a second pass sees the scopes of the first.
        for _ in 0..2 {
            for &class in &ordered {
                let scope = if anywhere.contains(&class) {
                    None
                } else {
                    let mut registries = Vec::new();
                    for call in &attachments[&class] {
                        match self.attachment_scope(call, &scopes, observer, classes) {
                            Some(on) => registries.extend(on),
                            None => {
                                registries.clear();
                                break;
                            }
                        }
                    }
                    (!registries.is_empty()).then_some(registries)
                };
                scopes.insert(class, scope);
            }
        }
        scopes
    }

    /// The registries what an attachment call (catalog `[[observer]]` `attach`) attaches runs
    /// for: attached to a registry, its records (and those of the registries below, when the
    /// attachment `propagates`); attached to an observer, where that observer runs.
    fn attachment_scope(
        &self,
        call: &Call,
        scopes: &HashMap<ClassId, LogScope>,
        observer: &Observer,
        classes: &[(ClassId, &ObserverClass)],
    ) -> LogScope {
        let logging = call.logging.as_ref()?;
        let qualname = call.targets.iter().map(|&t| self.facts.function(t).qualname.as_str()).next()?;
        let attach = observer.attach.iter().find(|a| a.functions.iter().any(|f| f == qualname))?;
        match &logging.logger {
            Some(registry) if registry.exact => Some(vec![(registry.name.clone(), attach.propagates)]),
            Some(_) => None,
            None => {
                let receiver = call.receiver.filter(|&r| classes.iter().any(|&(base, _)| self.inherits(r, base)))?;
                scopes.get(&receiver).cloned().flatten()
            }
        }
    }

    /// Whether a hook of `group` attached for `scope` runs for a record dispatched on `logger`
    /// (`None`: not known, any). With hierarchical names (`scope.separator`), as
    /// `Logger.callHandlers`: the observers of the registry and of its ancestors, up to the first
    /// one that does not propagate (`scope.stop`).
    fn log_reaches(&self, group: usize, scope: &LogScope, logger: Option<&LoggerName>) -> bool {
        let (Some(scope), Some(logger)) = (scope, logger) else { return true };
        let separator = self.catalog.observers()[group].scope.as_ref().and_then(|s| s.separator.as_deref());
        let below = |rest: &str| separator.is_some_and(|s| rest.starts_with(s));
        let under = |name: &str, ancestor: &str| {
            name == ancestor || (separator.is_some() && ancestor.is_empty()) || name.strip_prefix(ancestor).is_some_and(below)
        };
        // Whether the logger is (exactly or only maybe) `name` or below it.
        let surely_under = |name: &str| {
            if logger.exact {
                under(&logger.name, name)
            } else {
                (separator.is_some() && name.is_empty()) || logger.name.strip_prefix(name).is_some_and(below)
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

    /// The lowest level project code (outside tests) sets on a registry of `observer`: with its
    /// `threshold` calls (a level not known counts as any), or anything with its `any_level` ones.
    /// Without any, its `default_level`.
    fn logger_threshold(&self, observer: &Observer) -> u32 {
        let mut threshold: Option<u32> = None;
        for call in &self.facts.calls {
            if self.facts.function(call.caller).origin != Origin::Project || self.in_tests[call.caller.0 as usize] {
                continue;
            }
            for &t in &call.targets {
                let qualname = &self.facts.function(t).qualname;
                let level = if observer.any_level.contains(qualname) {
                    0
                } else if observer.threshold.contains(qualname) {
                    // A call not giving the level leaves it.
                    match &call.level {
                        Some(level) => level.value.unwrap_or(0),
                        None => continue,
                    }
                } else {
                    continue;
                };
                threshold = Some(threshold.map_or(level, |t| t.min(level)));
            }
        }
        threshold.unwrap_or(observer.default_level.unwrap_or(0))
    }

    /// The registries that do not propagate their records to their ancestors' observers: the
    /// `scope.stop` attribute is only ever set false on them.
    fn logger_stops(&self) -> Vec<String> {
        let mut stops: Vec<String> = Vec::new();
        for (name, _) in self.facts.stops_propagation.iter().filter(|(_, stops)| *stops) {
            if !self.facts.stops_propagation.iter().any(|(n, stops)| n == name && !stops) && !stops.contains(name) {
                stops.push(name.clone());
            }
        }
        stops
    }

    /// The level of the observers of each class, when the project gives it (catalog
    /// `[[observer]]` `classes.level`): a level call on an instance of the class
    /// (`self.setLevel(...)` included) or in its own constructor (`super().__init__(level)`),
    /// else the level it is constructed with; the lowest when several.
    fn handler_levels(&self, observer: &Observer) -> HashMap<ClassId, u32> {
        let setters: HashSet<&str> =
            observer.classes.iter().flat_map(|c| c.level.iter().map(String::as_str)).collect();
        let (mut set, mut constructed): (HashMap<ClassId, u32>, HashMap<ClassId, u32>) = Default::default();
        let lower = |map: &mut HashMap<ClassId, u32>, class: ClassId, level: u32| {
            map.entry(class).and_modify(|l| *l = (*l).min(level)).or_insert(level);
        };
        for call in &self.facts.calls {
            let value = call.level.as_ref().map_or(0, |l| l.value.unwrap_or(0));
            if let Some(class) = call.constructs {
                lower(&mut constructed, class, value);
            }
            if call.constructs.is_some()
                || call.level.is_none()
                || !call.targets.iter().any(|&t| setters.contains(self.facts.function(t).qualname.as_str()))
            {
                continue;
            }
            let receiver = call.level.as_ref().and_then(|l| l.receiver);
            if let Some(class) = receiver.or(self.facts.function(call.caller).class) {
                lower(&mut set, class, value);
            }
        }
        constructed.extend(set);
        constructed
    }

    /// Whether what `signature` returns is sent as is when its function is registered with
    /// `entry` (every value returned is an instance of one of the entry's `passthrough` classes).
    fn passes_through(&self, signature: Option<&Signature>, entry: &Entry) -> bool {
        signature.is_some_and(|s| s.always_returns.iter().any(|c| entry.passthrough.contains(c)))
    }

    /// Whether `class` is `base` or a subclass of it.
    fn inherits(&self, class: ClassId, base: ClassId) -> bool {
        class == base || self.ancestors(class).contains(&base)
    }

    /// Model (and response class) code a framework runs on the loop around what is registered
    /// with an `[[entry]]`, with what runs it: validating the parameters (`validates_params`),
    /// validating the return value into the response model (`validates_return`, `async def`
    /// only: a sync callable runs in a thread) and dumping it (`serializes_return`). Callables
    /// always returning a `passthrough` class skip the last two (and the response class).
    fn framework_hooks(&self) -> Vec<(FnId, String)> {
        let signatures: HashMap<FnId, &Signature> = self.facts.signatures.iter().map(|s| (s.function, s)).collect();
        let mut response_models: HashMap<(FnId, &Location), Vec<ClassId>> = HashMap::default();
        let mut endpoints: HashMap<(FnId, &Location), Vec<bool>> = HashMap::default();
        for flow in &self.facts.flows {
            if let (Value::Class(class), Slot::Param(slot)) = (&flow.value, &flow.slot)
                && self.entry(flow.into).and_then(|e| e.response_model.as_ref()) == Some(slot)
            {
                response_models.entry((flow.caller, &flow.location)).or_default().push(*class);
            }
            if let Value::Function(f) = flow.value
                && self.registers(flow)
                && let Some(entry) = self.entry(flow.into)
            {
                let passes = self.passes_through(signatures.get(&f).copied(), entry);
                endpoints.entry((flow.caller, &flow.location)).or_default().push(passes);
            }
        }
        let options: HashMap<(FnId, &Location), &ResponseOptions> =
            self.facts.response_options.iter().map(|o| ((o.caller, &o.location), o)).collect();
        let mut hooks = Vec::new();
        for flow in &self.facts.flows {
            let into = &self.facts.function(flow.into).qualname;
            // A class (or factory) the library constructs later, on the loop (catalog
            // `[[builds_on_loop]]`: FastAPI's `response_class`, Starlette's middleware).
            if let Some(builds) = self.catalog.builds_on_loop_of(self.roles[flow.into.0 as usize])
                && matches!(&flow.slot, Slot::Param(p) if builds.slot.contains(p))
            {
                if builds.unless_passthrough
                    && endpoints.get(&(flow.caller, &flow.location)).is_some_and(|e| e.iter().all(|&passes| passes))
                {
                    continue;
                }
                let label = match &builds.label {
                    Some(label) => label.clone(),
                    None => entry_label(self.entry_kind(flow.into).unwrap_or_default(), into, flow),
                };
                let label = format!("{label} ({})", builds.kind);
                let run = match flow.value {
                    Value::Class(class) => self.construction(class),
                    Value::Function(f) => vec![f],
                    Value::Param(..) => Vec::new(),
                };
                hooks.extend(run.into_iter().map(|h| (h, label.clone())));
                continue;
            }
            let Some(entry) = self.entry(flow.into) else { continue };
            if !(entry.validates_params || entry.serializes_return || entry.validates_return) || !self.registers(flow) {
                continue;
            }
            let function = match flow.value {
                Value::Function(f) => f,
                // A class: the framework fills the parameters of its `__init__`.
                Value::Class(class) => {
                    let Some(init) = Hierarchy::lookup(self.facts, class, "__init__") else { continue };
                    init
                }
                Value::Param(..) => continue,
            };
            let none =
                Signature { function, params: Vec::new(), annotation: Vec::new(), returned: Vec::new(), returns_unknown: false, always_returns: Vec::new() };
            let signature = signatures.get(&function).copied().unwrap_or(&none);
            let label = entry_label(&entry.kind, into, flow);
            let mut add = |found: Vec<FnId>, what: &str| hooks.extend(found.into_iter().map(|h| (h, format!("{label} ({what})"))));
            if entry.validates_params {
                for value in &signature.params {
                    match value {
                        Value::Class(class) => add(self.validation_hooks(*class).to_vec(), "request validation"),
                        Value::Function(f) => add(vec![*f], "request validation"),
                        Value::Param(..) => {}
                    }
                }
            }
            if matches!(flow.value, Value::Class(_)) || self.passes_through(Some(signature), entry) {
                continue;
            }
            let options = options.get(&(flow.caller, &flow.location));
            // `response_model=None` leaves the route without a response model.
            let models = match response_models.get(&(flow.caller, &flow.location)) {
                Some(models) => &models[..],
                None if options.is_some_and(|o| o.no_model) => &[],
                None => &signature.annotation[..],
            };
            // Without a response model, `jsonable_encoder` dumps whatever is returned (and
            // ignores `response_model_exclude`).
            // Both dump in JSON mode.
            let exclude = options.map(|o| o.exclude.clone()).filter(|_| !models.is_empty()).unwrap_or_default();
            let dump = Dump { json: true, exclude, include: None };
            let dumped = if models.is_empty() { &signature.returned[..] } else { models };
            for &class in dumped.iter().filter(|_| entry.serializes_return) {
                add(self.serialization_hooks(class, &dump), "response serialization");
            }
            // Validating an instance of the model (or of a subclass) keeps it as is.
            if entry.validates_return && self.facts.function(function).is_async && !signature.returns_unknown {
                let validated = models.iter().filter(|&&m| !signature.returned.iter().any(|&r| self.inherits(r, m)));
                for &class in validated {
                    add(self.validation_hooks(class).to_vec(), "response validation");
                }
            }
        }
        // A callable instance of a project subclass handed to FastAPI (`Security(scheme)`) whose
        // `__call__` is a library `async def`: the project overrides of the hooks it calls.
        for flow in &self.facts.flows {
            let (Value::Function(method), Some(receiver)) = (flow.value.clone(), flow.receiver) else { continue };
            let into = &self.facts.function(flow.into).qualname;
            let Some(kind) = self.entry_kind(flow.into) else { continue };
            if !self.registers(flow) {
                continue;
            }
            let found = self.template_hooks(method, receiver);
            if found.is_empty() {
                continue;
            }
            let name = self.facts.function(method).qualname.rsplit('.').next().unwrap_or_default().to_string();
            let label = format!("{} ({name})", entry_label(kind, into, flow));
            hooks.extend(found.iter().map(|&h| (h, label.clone())));
        }
        // A project callable assigned over a library method on an instance (`app.openapi =
        // custom_openapi`) runs wherever the library calls that method on the instance.
        let mut loop_calls: HashMap<ClassId, HashMap<String, FnId>> = HashMap::default();
        for store in &self.facts.stores {
            let Value::Function(f) = store.value else { continue };
            let replaced = Hierarchy::lookup(self.facts, store.class, &store.name);
            if self.facts.function(f).origin != Origin::Project
                || replaced.is_none_or(|m| self.facts.function(m).origin == Origin::Project)
            {
                continue;
            }
            let calls = loop_calls.entry(store.class).or_insert_with(|| self.loop_self_calls(store.class));
            if let Some(&caller) = calls.get(&store.name) {
                let caller = &self.facts.function(caller).qualname;
                hooks.push((f, format!("`{}` replaced on an instance, called by {caller}", store.name)));
            }
        }
        // Project classes library code instantiates (`APIRouter(route_class=AuditedRoute)` stores
        // the class, `add_api_route` calls it): the library `async def` methods of those instances
        // run on the loop, and with them the project overrides of what they call on `self`.
        for (class, by) in self.library_constructed() {
            let calls = loop_calls.entry(class).or_insert_with(|| self.loop_self_calls(class));
            for (name, &caller) in calls.iter() {
                let Some(method) = Hierarchy::lookup(self.facts, class, name) else { continue };
                if self.facts.function(method).origin != Origin::Project {
                    continue;
                }
                let (class, by, caller) = (&self.facts.class(class).qualname, &self.facts.function(by).qualname, &self.facts.function(caller).qualname);
                hooks.push((method, format!("{class} instances {by} constructs: `{name}`, called by {caller}")));
            }
        }
        hooks.retain(|(h, _)| self.facts.function(*h).analyzed);
        hooks
    }

    /// The classes library code instantiates from a class passed to it, with the library function
    /// it is passed to: one that calls the parameter, or stores it on its instance where a method
    /// of its class calls the attribute (see [`crate::facts::Function::called_params`]).
    fn library_constructed(&self) -> Vec<(ClassId, FnId)> {
        let mut found = Vec::new();
        for flow in &self.facts.flows {
            let (Value::Class(class), Slot::Param(slot)) = (&flow.value, &flow.slot) else { continue };
            let into = self.facts.function(flow.into);
            if into.origin == Origin::Project {
                continue;
            }
            let calls_attribute = |attribute: &String| {
                into.class.is_some_and(|k| {
                    self.ancestors(k).iter().any(|&a| {
                        let methods = &self.facts.class(a).methods;
                        methods.iter().any(|(_, m)| self.facts.function(*m).called_attributes.contains(attribute))
                    })
                })
            };
            if into.called_params.contains(slot)
                || into.stored_params.iter().any(|(p, attribute)| p == slot && calls_attribute(attribute))
            {
                found.push((*class, flow.into));
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }

    /// What a virtual call runs when the class it dispatches on has no instances of its own (see
    /// [`crate::facts::Class::referenced`]): the methods the MROs of its instantiable subclasses
    /// find. `None` when the class may have instances, a target is a copy (see `specialize`), or
    /// no instantiable subclass is known.
    fn narrowed_targets(&self, call: &Call, hierarchy: &Hierarchy) -> Option<Vec<FnId>> {
        let mut out = Vec::new();
        for &t in &call.targets {
            let f = self.facts.function(t);
            if f.specializes.is_some() {
                return None;
            }
            let owner = f.class?;
            let class = call.receiver.filter(|r| !self.facts.class(*r).is_protocol).unwrap_or(owner);
            if self.instantiable[class.0 as usize] {
                return None;
            }
            let name = &self.facts.class(owner).methods.iter().find(|(_, m)| *m == t)?.0;
            let below = self.instantiable_below(class, hierarchy);
            if below.is_empty() {
                return None;
            }
            for &c in below.iter() {
                out.extend(Hierarchy::lookup(self.facts, c, name).filter(|&m| self.dispatches_into(call.caller, m)));
            }
        }
        out.sort_unstable();
        out.dedup();
        (!out.is_empty()).then_some(out)
    }

    /// The subclasses of `class` (at any depth) that may have instances of their own.
    fn instantiable_below(&self, class: ClassId, hierarchy: &Hierarchy) -> Rc<[ClassId]> {
        if let Some(found) = self.below.borrow().get(&class) {
            return found.clone();
        }
        let mut out = Vec::new();
        let mut seen = HashSet::from_iter([class]);
        let mut queue = VecDeque::from([class]);
        while let Some(c) = queue.pop_front() {
            for &sub in &hierarchy.subclasses[c.0 as usize] {
                if seen.insert(sub) {
                    if self.instantiable[sub.0 as usize] {
                        out.push(sub);
                    }
                    queue.push_back(sub);
                }
            }
        }
        let out: Rc<[ClassId]> = out.into();
        self.below.borrow_mut().insert(class, out.clone());
        out
    }

    /// The methods library code calls on the loop on an instance of `class`, each with the
    /// library function calling it: what its `async def` methods (and the `async def` closures
    /// its methods define, such as the routes `FastAPI.setup` registers) call on `self`,
    /// directly or through other library methods.
    fn loop_self_calls(&self, class: ClassId) -> HashMap<String, FnId> {
        let mut found: HashMap<String, FnId> = HashMap::default();
        let mut queue = VecDeque::new();
        for &c in self.ancestors(class).iter() {
            for &(_, m) in &self.facts.class(c).methods {
                let f = self.facts.function(m);
                if f.origin == Origin::Project {
                    continue;
                }
                let direct = if f.is_async { &f.self_calls[..] } else { &[] };
                for name in direct.iter().chain(&f.closure_self_calls) {
                    if !found.contains_key(name) {
                        found.insert(name.clone(), m);
                        queue.push_back(name.clone());
                    }
                }
            }
        }
        while let Some(name) = queue.pop_front() {
            let Some(m) = Hierarchy::lookup(self.facts, class, &name) else { continue };
            let f = self.facts.function(m);
            if f.origin == Origin::Project {
                continue;
            }
            let caller = found[&name];
            for next in &f.self_calls {
                if !found.contains_key(next) {
                    found.insert(next.clone(), caller);
                    queue.push_back(next.clone());
                }
            }
        }
        found
    }

    /// What constructing `class` runs: its `__init__`, and the project overrides of what the
    /// library `__init__`s in its MRO call on `self` (Starlette's `Response.__init__`, reached
    /// through `JSONResponse.__init__`'s `super().__init__(...)`, calls `self.render(...)`).
    fn construction(&self, class: ClassId) -> Vec<FnId> {
        let mut run: Vec<FnId> = Hierarchy::lookup(self.facts, class, "__init__").into_iter().collect();
        for c in std::iter::once(class).chain(self.ancestors(class).iter().copied()) {
            let Some(&(_, init)) = self.facts.class(c).methods.iter().find(|(name, _)| name == "__init__") else {
                continue;
            };
            if self.facts.function(init).origin != Origin::Project {
                run.extend(self.overrides_called(init, class).iter().copied());
            }
        }
        run.sort_unstable();
        run.dedup();
        run
    }

    /// What a library `async def` method runs of the project when it runs on an instance of
    /// `receiver` (template methods: `APIKeyHeader.__call__` calls `self.check_api_key(...)`):
    /// see [`Graph::overrides_called`].
    fn template_hooks(&self, method: FnId, receiver: ClassId) -> Rc<[FnId]> {
        if !self.facts.function(method).is_async {
            return Rc::from([]);
        }
        self.overrides_called(method, receiver)
    }

    /// The project overrides, in `receiver`'s MRO, of the library methods a library method calls
    /// on `self`, directly or through other library methods it calls on `self`.
    fn overrides_called(&self, method: FnId, receiver: ClassId) -> Rc<[FnId]> {
        let f = self.facts.function(method);
        let Some(owner) = f.class.filter(|_| !f.analyzed && f.origin != Origin::Project) else {
            return Rc::from([]);
        };
        if let Some(found) = self.templates.borrow().get(&(method, receiver)) {
            return found.clone();
        }
        let mut hooks = Vec::new();
        if receiver != owner && self.inherits(receiver, owner) {
            let mut seen = HashSet::from_iter([method]);
            let mut queue = VecDeque::from([method]);
            while let Some(m) = queue.pop_front() {
                let caller = self.facts.function(m);
                for name in &caller.self_calls {
                    let Some(found) = Hierarchy::lookup(self.facts, receiver, name) else { continue };
                    if !seen.insert(found) {
                        continue;
                    }
                    let target = self.facts.function(found);
                    if target.origin != Origin::Project {
                        if !target.analyzed {
                            queue.push_back(found);
                        }
                    } else if target.analyzed
                        && caller.class.is_some_and(|c| Hierarchy::lookup(self.facts, c, name).is_some())
                    {
                        // An override of a method the library defines.
                        hooks.push(found);
                    }
                }
            }
        }
        let hooks: Rc<[FnId]> = Rc::from(hooks);
        self.templates.borrow_mut().insert((method, receiver), hooks.clone());
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
        let tests = !self.in_tests[method.0 as usize] || self.in_tests[caller.0 as usize];
        let (caller, method) = (self.facts.function(caller), self.facts.function(method));
        tests
            && (!method.qualname.contains(".<locals>.") || method.location.path == caller.location.path)
    }

    /// Whether the sink `target` blocks when called at `call`: calls giving one of its
    /// `unless_given` arguments, or not known to leave them out, skip the blocking work.
    fn blocks_here(&self, call: usize, target: FnId) -> bool {
        if self.wrapper[target.0 as usize] {
            return self.facts.calls[call].returned_by.contains(&target);
        }
        let unless = self.unless_given[target.0 as usize];
        unless.is_empty() || self.facts.calls[call].given.as_ref().is_some_and(|g| !g.iter().any(|a| unless.contains(a)))
    }

    /// Whether calling `target` at `call` runs its body now: calling a generator function only
    /// creates the generator (or the context manager its decorator wraps it in), unless the call
    /// site iterates (enters) it right away.
    fn runs_body(&self, call: usize, target: FnId) -> bool {
        let call = &self.facts.calls[call];
        !(self.facts.function(target).is_generator || call.deferred_body) || call.iterates
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
            let on_loop = self.roles[flow.into.0 as usize].loop_callback || self.entry_kind(flow.into) == Some("lifecycle");
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
    /// on a shortest chain to a blocking function of severity `at_least` or worse.
    fn blocking_distances(&self, at_least: Severity) -> Vec<Option<(u32, usize, FnId)>> {
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
                if let Some(sink) = &self.sink[t.0 as usize] {
                    if sink.severity >= at_least
                        && !call.awaited
                        && self.blocks_here(i, t)
                        && next[call.caller.0 as usize].is_none()
                    {
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

    /// The shortest blocking chain starting at `call` to a blocking function of severity
    /// `at_least` or worse (`next` from [`Graph::blocking_distances`] for the same severity), and
    /// what blocks at its end.
    fn blocking_chain(
        &self,
        call: usize,
        next: &[Option<(u32, usize, FnId)>],
        at_least: Severity,
    ) -> Option<(Vec<Frame>, Sink)> {
        let c = &self.facts.calls[call];
        let best = self.targets[call]
            .iter()
            .filter_map(|&t| {
                if let Some(sink) = &self.sink[t.0 as usize] {
                    (sink.severity >= at_least && !c.awaited && self.blocks_here(call, t)).then_some((0, t))
                } else if self.facts.function(t).is_async || !self.runs_body(call, t) {
                    None
                } else {
                    next[t.0 as usize].map(|(d, _, _)| (d, t))
                }
            })
            // Copies (see `specialize`) rank as their original.
            .min_by_key(|&(d, t)| (d, self.facts.function(t).specializes.unwrap_or(t), t))?;
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
        // Entry points that only run while the server starts or stops, and the others.
        let (mut once, mut traffic): (Vec<FnId>, Vec<FnId>) = (Vec::new(), Vec::new());
        // Copies (see `specialize`) register what their original does.
        for flow in self.facts.flows.iter().filter(|f| self.facts.function(f.caller).specializes.is_none()) {
            if self.roles[flow.into.0 as usize].loop_callback
                && let Value::Function(f) = flow.value
            {
                // Scheduled on the loop: it may run at any time.
                traffic.push(f);
            }
            let into = &self.facts.function(flow.into).qualname;
            let Some(entry) = self.entry(flow.into) else { continue };
            let label = entry_label(&entry.kind, into, flow);
            let added = entries.len();
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
            let registered = entries[added..].iter().map(|(f, _)| *f);
            if entry.runs_once(&flow.slot) { once.extend(registered) } else { traffic.extend(registered) }
        }
        entries.extend(self.hooks.iter().cloned());
        traffic.extend(self.hooks.iter().map(|(h, _)| *h));
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
        let runs_once = self.runs_once(once, traffic);
        EntryPaths { graph: self, from_entries, from_project, entries: count, runs_once }
    }

    /// The functions that only run while the server starts or stops: reached from `once`
    /// entry points, and not from `traffic` ones. A coroutine created there without being
    /// awaited (a task, or a coroutine given to something else) may run later, so it counts as
    /// traffic.
    fn runs_once(&self, once: Vec<FnId>, mut traffic: Vec<FnId>) -> Vec<bool> {
        let mut at_start = vec![false; self.facts.functions.len()];
        let mut queue: VecDeque<FnId> = VecDeque::new();
        for f in once {
            if !std::mem::replace(&mut at_start[f.0 as usize], true) {
                queue.push_back(f);
            }
        }
        while let Some(f) = queue.pop_front() {
            for &call in &self.calls_of[f.0 as usize] {
                for &t in &self.targets[call] {
                    if !self.runs_inline(f, t) || !self.runs_body(call, t) {
                        continue;
                    }
                    if self.facts.function(t).is_async && !self.facts.calls[call].awaited {
                        traffic.push(t);
                        continue;
                    }
                    // Reaching a copy (see `specialize`) reaches the code of its original.
                    for g in std::iter::once(t).chain(self.facts.function(t).specializes) {
                        if !std::mem::replace(&mut at_start[g.0 as usize], true) {
                            queue.push_back(g);
                        }
                    }
                }
            }
        }
        let serving = self.bfs(traffic.into_iter().map(|f| (f, String::new())).collect());
        at_start.iter().zip(&serving).map(|(&start, serving)| start && serving.is_none()).collect()
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
                        // Reaching a copy (see `specialize`) reaches the code of its original.
                        if let Some(original) = self.facts.function(t).specializes
                            && reach[original.0 as usize].is_none()
                        {
                            reach[original.0 as usize] = Some(Reach::Via(f, call));
                        }
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
                        // Copies (see `specialize`) report as their original.
                        if let Some(original) = target.specializes {
                            on_loop[original.0 as usize] = true;
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        for (i, call) in self.facts.calls.iter().enumerate() {
            let caller = self.facts.function(call.caller);
            if !on_loop[call.caller.0 as usize]
                || caller.origin != Origin::Project
                || caller.specializes.is_some()
                || !self.targets[i].is_empty()
                || self.offloaded[i]
            {
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
    /// The `[[observer]]` group.
    group: usize,
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
    /// See [`Graph::runs_once`], by function.
    runs_once: Vec<bool>,
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

/// The sink a library method overrides: `sqlmodel`'s `Session.execute` overrides SQLAlchemy's
/// `Session.execute` and delegates to it through `super()`. Library bodies are not analyzed (or,
/// with `--follow-libs`, are, and need no help), and a sync override of a blocking method keeps
/// its base's contract: it blocks like it.
fn inherited_sink(facts: &Facts, catalog: &Catalog, roles: &[Roles], f: &crate::facts::Function) -> Option<Sink> {
    if f.origin != Origin::Library || f.analyzed || f.is_async || f.property.is_some() {
        return None;
    }
    let class = facts.class(f.class?);
    let name = f.qualname.rsplit('.').next()?;
    let base = class.bases.iter().find_map(|&b| Hierarchy::lookup(facts, b, name))?;
    let found = catalog.sink_of(roles[base.0 as usize], &facts.function(base).qualname);
    found.or_else(|| inherited_sink(facts, catalog, roles, facts.function(base)))
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
