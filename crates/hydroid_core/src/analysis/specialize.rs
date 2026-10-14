//! Copies of functions for one kind of `self`, or of arguments.
//!
//! Which `self.name(...)` runs inside a method depends on the instance the method was called on,
//! and the call graph has one node per function. Where the caller knows more about the instance
//! than its declared class (see [`Instance`]: it is exactly a `Model()`, or its type specializes
//! a generic class), the methods it calls are copied for that instance, and the copies' calls
//! dispatch by what is known: `self.hydrate()` in a copy of `Model.refresh` for an exact `Model`
//! runs `Model.hydrate`, not the overrides in subclasses; `self.repo.list_ids()` in a copy of
//! `BaseService.count` for a `TagService` runs the `list_ids` of the repository class
//! `TagService` specializes `BaseService` to. The same goes for parameters: a function called
//! with an argument that is exactly an instance of a class (`checksum(io.BytesIO(data))`, see
//! [`Call::exact_args`]) is copied for it, and `stream.read()` on that parameter in the copy
//! runs the `read` of `BytesIO`, not of the declared `BinaryIO`.
//!
//! Copies keep the original's name and location, so diagnostics found in them read (and
//! deduplicate) like the original's.

use std::collections::VecDeque;

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use super::Hierarchy;
use crate::facts::{Call, ClassId, Facts, FnId, Instance};

/// At most this many copies: past it, methods are called unspecialized (as without copies).
const MAX_COPIES: usize = 50_000;

/// What a copy knows about its `self` (`class`, when it is a method copied for an instance),
/// and about its parameters.
#[derive(Clone, PartialEq, Eq, Hash)]
struct SelfKind {
    class: Option<ClassId>,
    /// `self` is an instance of exactly `class`, not of a subclass.
    exact: bool,
    /// See [`Instance::binds`].
    binds: Vec<(String, ClassId)>,
    /// Parameters that are exactly an instance of a class (see [`Call::exact_args`]).
    params: Vec<(String, ClassId)>,
}

/// Narrows call targets by what is known about the instances methods are called on, and adds
/// the method copies that takes (after the original functions, calls and flows). Nothing changes
/// when nothing is known beyond declared classes.
pub(super) fn specialize(facts: &mut Facts) {
    // Only attributes some method calls through a type variable matter.
    let generic: HashSet<String> =
        facts.calls.iter().filter_map(|c| c.instance.as_ref()?.generic.clone()).collect();
    let n = facts.functions.len();
    // The parameters of each function whose calls depend on what is passed.
    let mut sensitive_params: Vec<Vec<String>> = vec![Vec::new(); n];
    for call in &facts.calls {
        if let Some(param) = call.instance.as_ref().and_then(|i| i.param.as_ref())
            && !sensitive_params[call.caller.0 as usize].contains(param)
        {
            sensitive_params[call.caller.0 as usize].push(param.clone());
        }
    }
    let narrows = |i: &Instance| !i.exact.is_empty() || i.binds.iter().any(|(a, _)| generic.contains(a));
    let passes = |c: &Call| {
        c.exact_args.iter().any(|(p, _)| c.targets.iter().any(|t| sensitive_params[t.0 as usize].contains(p)))
    };
    if !facts.calls.iter().any(|c| c.instance.as_deref().is_some_and(narrows) || passes(c)) {
        return;
    }
    let mut calls_of = vec![Vec::new(); n];
    // Methods whose calls depend on the kind of `self`.
    let mut sensitive = vec![false; n];
    // The calls made on instances (or passing exact arguments), in call order: copies are
    // numbered as they are first needed.
    let mut on_instances = Vec::new();
    for (i, call) in facts.calls.iter().enumerate() {
        calls_of[call.caller.0 as usize].push(i);
        if let Some(instance) = &call.instance
            && (instance.is_self || instance.generic.is_some())
        {
            sensitive[call.caller.0 as usize] = true;
        }
        // A call on a parameter is only narrowed in a copy for its argument.
        let on_instance = call.instance.as_deref().is_some_and(|i| {
            i.is_self || i.generic.is_some() || !i.exact.is_empty() || !i.binds.is_empty()
        });
        if on_instance || passes(call) {
            on_instances.push(i);
        }
    }
    let mut flows_of = vec![Vec::new(); n];
    for (i, flow) in facts.flows.iter().enumerate() {
        flows_of[flow.caller.0 as usize].push(i);
    }
    let hierarchy = Hierarchy::new(facts);
    let mut specializer = Specializer {
        facts,
        generic,
        hierarchy,
        sensitive,
        sensitive_params,
        calls_of,
        narrowing: HashMap::default(),
        copies: HashMap::default(),
        queue: VecDeque::new(),
    };
    // Copies rewrite the calls as extracted: the rewritten originals replace them at the end.
    let rewritten: Vec<(usize, Call)> = on_instances.into_iter().map(|i| (i, specializer.rewrite(i, None))).collect();
    while let Some((original, copy, kind)) = specializer.queue.pop_front() {
        for i in specializer.calls_of[original.0 as usize].clone() {
            let mut call = specializer.rewrite(i, Some(&kind));
            call.caller = copy;
            specializer.facts.calls.push(call);
        }
        for &i in &flows_of[original.0 as usize] {
            let mut flow = specializer.facts.flows[i].clone();
            flow.caller = copy;
            specializer.facts.flows.push(flow);
        }
    }
    for (i, call) in rewritten {
        specializer.facts.calls[i] = call;
    }
}

struct Specializer<'a> {
    facts: &'a mut Facts,
    generic: HashSet<String>,
    hierarchy: Hierarchy,
    sensitive: Vec<bool>,
    sensitive_params: Vec<Vec<String>>,
    calls_of: Vec<Vec<usize>>,
    /// Memoized [`Specializer::narrows_params`].
    narrowing: HashMap<(FnId, Vec<(String, ClassId)>), bool>,
    copies: HashMap<(FnId, SelfKind), FnId>,
    /// Copies whose calls are still to be made: `(original, copy, kind of self)`.
    queue: VecDeque<(FnId, FnId, SelfKind)>,
}

impl Specializer<'_> {
    /// Call `i` (made in a copy for `kind`, if any) with its targets narrowed, then copied for
    /// the exact arguments it passes.
    fn rewrite(&mut self, i: usize, kind: Option<&SelfKind>) -> Call {
        let mut call = self.narrow(i, kind);
        let exact_args = std::mem::take(&mut call.exact_args);
        if exact_args.is_empty() {
            return call;
        }
        // Copies of originals only: a copy's calls are those of its original.
        for t in &mut call.targets {
            // Copies are numbered after the originals.
            let Some(sensitive) = self.sensitive_params.get(t.0 as usize) else { continue };
            let params: Vec<(String, ClassId)> =
                exact_args.iter().filter(|(p, _)| sensitive.contains(p)).cloned().collect();
            if params.is_empty() || !self.narrows_params(*t, &params) {
                continue;
            }
            let kind = SelfKind { class: None, exact: false, binds: Vec::new(), params };
            *t = self.copy_for(*t, &kind, true);
        }
        call
    }

    /// Call `i` (made in a copy for `kind`, if any) with its targets narrowed.
    fn narrow(&mut self, i: usize, kind: Option<&SelfKind>) -> Call {
        let mut call = self.facts.calls[i].clone();
        let Some(instance) = call.instance.take() else { return call };
        // Only plain method calls: a target that is something else (a decorator's wrapper, a
        // callable stored in an attribute) is kept as resolved.
        let is_method = |t: &FnId| {
            let f = self.facts.function(*t);
            f.class.is_some() && f.qualname.rsplit('.').next() == Some(instance.name.as_str())
        };
        if call.targets.is_empty() || !call.targets.iter().all(is_method) {
            return call;
        }
        let name = instance.name.as_str();
        match kind {
            // `param.name(...)` on a parameter exactly an instance of a class: its `name`.
            Some(kind)
                if let Some(&(_, class)) =
                    kind.params.iter().find(|(p, _)| instance.param.as_ref().is_some_and(|i| i == p)) =>
            {
                let Some(found) = self.lookup(class, name) else { return call };
                let kind = SelfKind { class: Some(class), exact: true, binds: Vec::new(), params: Vec::new() };
                call.receiver = Some(class);
                call.virtual_dispatch = false;
                call.targets = found.into_iter().map(|m| self.copy(m, &kind)).collect();
                return call;
            }
            // `self.name(...)`: the one `name` the instance the copy runs for finds.
            Some(kind) if instance.is_self && let Some(class) = kind.class => {
                let Some(found) = self.lookup(class, name) else { return call };
                let kind = SelfKind { params: Vec::new(), ..kind.clone() };
                call.receiver = Some(class);
                call.virtual_dispatch &= !kind.exact;
                call.targets = found.into_iter().map(|m| self.copy(m, &kind)).collect();
                return call;
            }
            // `self.repo.name(...)`, `repo` typed by a type variable the instance binds.
            Some(kind) if instance.generic.is_some() => {
                let attribute = instance.generic.as_deref();
                if let Some(&(_, class)) = kind.binds.iter().find(|(a, _)| Some(a.as_str()) == attribute)
                    && let Some(found) = self.lookup(class, name)
                {
                    call.receiver = Some(class);
                    call.targets = found;
                }
                return call;
            }
            _ => {}
        }
        let mut binds: Vec<(String, ClassId)> =
            instance.binds.iter().filter(|(a, _)| self.generic.contains(a)).cloned().collect();
        binds.sort();
        if !instance.exact.is_empty() {
            let mut targets = Vec::new();
            for &class in &instance.exact {
                let Some(found) = self.lookup(class, name) else { return call };
                let kind = SelfKind { class: Some(class), exact: true, binds: binds.clone(), params: Vec::new() };
                targets.extend(found.into_iter().map(|m| self.copy(m, &kind)));
            }
            targets.sort_unstable();
            targets.dedup();
            call.targets = targets;
            call.virtual_dispatch = false;
            if let [class] = instance.exact[..] {
                call.receiver = Some(class);
            }
        } else if !binds.is_empty()
            && let Some(class) = call.receiver
        {
            let kind = SelfKind { class: Some(class), exact: false, binds, params: Vec::new() };
            call.targets = call.targets.iter().map(|&t| self.copy(t, &kind)).collect();
        }
        call
    }

    /// Whether some call on a parameter in `function` finds other methods when the parameter is
    /// exactly an instance of its class in `params` (else a copy would change nothing).
    fn narrows_params(&mut self, function: FnId, params: &[(String, ClassId)]) -> bool {
        let key = (function, params.to_vec());
        if let Some(&found) = self.narrowing.get(&key) {
            return found;
        }
        let mut narrows = false;
        for &i in &self.calls_of[function.0 as usize] {
            let call = &self.facts.calls[i];
            let Some(instance) = &call.instance else { continue };
            let Some(&(_, class)) = params.iter().find(|(p, _)| instance.param.as_ref() == Some(p)) else { continue };
            let found = self.lookup(class, &instance.name);
            if found.is_some_and(|found| found != call.targets) {
                narrows = true;
                break;
            }
        }
        self.narrowing.insert(key, narrows);
        narrows
    }

    /// The methods `class.name` finds: those of the first class in its MRO defining `name`.
    fn lookup(&self, class: ClassId, name: &str) -> Option<Vec<FnId>> {
        for &c in self.hierarchy.mro(&*self.facts, class).iter() {
            let found: Vec<FnId> =
                self.facts.class(c).methods.iter().filter(|(n, _)| n == name).map(|(_, m)| *m).collect();
            if found.is_empty() {
                continue;
            }
            // Properties are no calls.
            return found.iter().all(|m| self.facts.function(*m).property.is_none()).then_some(found);
        }
        None
    }

    /// `method` running for `kind` of `self`: a copy, when its calls depend on it.
    fn copy(&mut self, method: FnId, kind: &SelfKind) -> FnId {
        let sensitive = self.sensitive[method.0 as usize];
        self.copy_for(method, kind, sensitive)
    }

    /// `function` running for `kind`: a copy, when its calls depend on it (`sensitive`).
    fn copy_for(&mut self, method: FnId, kind: &SelfKind, sensitive: bool) -> FnId {
        if !sensitive || !self.facts.function(method).analyzed {
            return method;
        }
        let key = (method, kind.clone());
        if let Some(&copy) = self.copies.get(&key) {
            return copy;
        }
        if self.copies.len() >= MAX_COPIES {
            return method;
        }
        let copy = FnId(self.facts.functions.len() as u32);
        let mut function = self.facts.function(method).clone();
        function.specializes = Some(method);
        self.facts.functions.push(function);
        self.copies.insert(key, copy);
        self.queue.push_back((method, copy, kind.clone()));
        copy
    }
}
