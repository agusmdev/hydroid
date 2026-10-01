//! From facts to diagnostics.
//!
//! Where code runs is a property of the *call*, not of the function: a sync function runs on the
//! event loop when called from code on the loop, and in a worker thread when handed to
//! `asyncio.to_thread`. So "does calling `f` block?" has one answer per function, computed once
//! by a breadth-first search backwards from blocking calls (shortest witness chains, O(V+E), no
//! recursion, any depth). Diagnostics are the call sites inside loop code (`async def` bodies and
//! callbacks scheduled on the loop) whose callee blocks.

use std::collections::{BTreeSet, VecDeque};

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::catalog::Catalog;
use crate::facts::{Call, ClassId, Facts, FnId, Flow, FunctionKind, Origin, Slot, Value};
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
        let mut graph =
            Self { facts, catalog, calls_of, targets: Vec::new(), sink, called: HashSet::default(), offload };
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
            if self.is_offload(flow.into) {
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
                // Calling a function decorated by project code runs the decorator's wrapper,
                // which reaches the original through the decorator's parameter.
                let wrappers: Vec<FnId> = self
                    .facts
                    .function(t)
                    .decorators
                    .iter()
                    .filter(|d| self.facts.function(**d).analyzed)
                    .flat_map(|d| nested[d.0 as usize].iter().copied())
                    .collect();
                let body = if wrappers.is_empty() { vec![t] } else { wrappers };
                (body, hierarchy.overrides(self.facts, t))
            });
            out.extend(body.iter().copied());
            out.extend(dispatches.get(&t).into_iter().flatten());
            if call.virtual_dispatch {
                out.extend(overrides.iter().copied());
            }
        }
        if let Some(class) = call.constructs {
            out.extend(self.construction_hooks(class));
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

    /// Whether calling `target` at `call` runs its body now: calling a generator function only
    /// creates the generator, unless the call site iterates it right away.
    fn runs_body(&self, call: usize, target: FnId) -> bool {
        !self.facts.function(target).is_generator || self.facts.calls[call].iterates
    }

    /// Loop code: every analyzed `async def`, plus sync callables scheduled on the loop.
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
                if self.sink[t.0 as usize].is_some() {
                    if !call.awaited && next[call.caller.0 as usize].is_none() {
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
                    (!c.awaited).then_some((0, t))
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
}

impl Hierarchy {
    fn new(facts: &Facts) -> Self {
        let mut subclasses = vec![Vec::new(); facts.classes.len()];
        for (i, class) in facts.classes.iter().enumerate() {
            for base in &class.bases {
                subclasses[base.0 as usize].push(ClassId(i as u32));
            }
        }
        Self { subclasses }
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
