//! Copies of methods for one kind of `self`.
//!
//! Which `self.name(...)` runs inside a method depends on the instance the method was called on,
//! and the call graph has one node per function. Where the caller knows more about the instance
//! than its declared class (see [`Instance`]: it is exactly a `Model()`, or its type specializes
//! a generic class), the methods it calls are copied for that instance, and the copies' calls
//! dispatch by what is known: `self.hydrate()` in a copy of `Model.refresh` for an exact `Model`
//! runs `Model.hydrate`, not the overrides in subclasses; `self.repo.list_ids()` in a copy of
//! `BaseService.count` for a `TagService` runs the `list_ids` of the repository class
//! `TagService` specializes `BaseService` to.
//!
//! Copies keep the original's name and location, so diagnostics found in them read (and
//! deduplicate) like the original's.

use std::collections::VecDeque;

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use super::Hierarchy;
use crate::facts::{Call, ClassId, Facts, FnId, Instance};

/// At most this many copies: past it, methods are called unspecialized (as without copies).
const MAX_COPIES: usize = 50_000;

/// What a copy knows about its `self`.
#[derive(Clone, PartialEq, Eq, Hash)]
struct SelfKind {
    class: ClassId,
    /// `self` is an instance of exactly `class`, not of a subclass.
    exact: bool,
    /// See [`Instance::binds`].
    binds: Vec<(String, ClassId)>,
}

/// Narrows call targets by what is known about the instances methods are called on, and adds
/// the method copies that takes (after the original functions, calls and flows). Nothing changes
/// when nothing is known beyond declared classes.
pub(super) fn specialize(facts: &mut Facts) {
    // Only attributes some method calls through a type variable matter.
    let generic: HashSet<String> =
        facts.calls.iter().filter_map(|c| c.instance.as_ref()?.generic.clone()).collect();
    let narrows = |i: &Instance| !i.exact.is_empty() || i.binds.iter().any(|(a, _)| generic.contains(a));
    if !facts.calls.iter().any(|c| c.instance.as_ref().is_some_and(narrows)) {
        return;
    }
    let n = facts.functions.len();
    let mut calls_of = vec![Vec::new(); n];
    // Methods whose calls depend on the kind of `self`.
    let mut sensitive = vec![false; n];
    // The calls made on instances, as extracted: copies rewrite them again for their `self`.
    let mut originals: HashMap<usize, Call> = HashMap::default();
    for (i, call) in facts.calls.iter().enumerate() {
        calls_of[call.caller.0 as usize].push(i);
        if let Some(instance) = &call.instance {
            if instance.is_self || instance.generic.is_some() {
                sensitive[call.caller.0 as usize] = true;
            }
            originals.insert(i, call.clone());
        }
    }
    let mut flows_of = vec![Vec::new(); n];
    for (i, flow) in facts.flows.iter().enumerate() {
        flows_of[flow.caller.0 as usize].push(i);
    }
    let hierarchy = Hierarchy::new(facts);
    let mut specializer =
        Specializer { facts, generic, hierarchy, sensitive, copies: HashMap::default(), queue: VecDeque::new() };
    // In call order: copies are numbered as they are first needed.
    let mut on_instances: Vec<usize> = originals.keys().copied().collect();
    on_instances.sort_unstable();
    for i in on_instances {
        specializer.facts.calls[i] = specializer.rewrite(&originals[&i], None);
    }
    while let Some((original, copy, kind)) = specializer.queue.pop_front() {
        for &i in &calls_of[original.0 as usize] {
            let mut call = match originals.get(&i) {
                Some(call) => specializer.rewrite(call, Some(&kind)),
                None => specializer.facts.calls[i].clone(),
            };
            call.caller = copy;
            specializer.facts.calls.push(call);
        }
        for &i in &flows_of[original.0 as usize] {
            let mut flow = specializer.facts.flows[i].clone();
            flow.caller = copy;
            specializer.facts.flows.push(flow);
        }
    }
}

struct Specializer<'a> {
    facts: &'a mut Facts,
    generic: HashSet<String>,
    hierarchy: Hierarchy,
    sensitive: Vec<bool>,
    copies: HashMap<(FnId, SelfKind), FnId>,
    /// Copies whose calls are still to be made: `(original, copy, kind of self)`.
    queue: VecDeque<(FnId, FnId, SelfKind)>,
}

impl Specializer<'_> {
    /// `call` (made in a copy for `kind` of `self`, if any) with its targets narrowed.
    fn rewrite(&mut self, call: &Call, kind: Option<&SelfKind>) -> Call {
        let mut call = call.clone();
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
            // `self.name(...)`: the one `name` the instance the copy runs for finds.
            Some(kind) if instance.is_self => {
                let Some(found) = self.lookup(kind.class, name) else { return call };
                call.receiver = Some(kind.class);
                call.virtual_dispatch &= !kind.exact;
                call.targets = found.into_iter().map(|m| self.copy(m, kind)).collect();
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
                let kind = SelfKind { class, exact: true, binds: binds.clone() };
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
            let kind = SelfKind { class, exact: false, binds };
            call.targets = call.targets.iter().map(|&t| self.copy(t, &kind)).collect();
        }
        call
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
        if !self.sensitive[method.0 as usize] || !self.facts.function(method).analyzed {
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
