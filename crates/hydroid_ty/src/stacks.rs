//! Exit stacks (catalog `[[exit_stack]]`): a call storing a callable, or a context manager's
//! exit, on a stack does not run it. The stack runs what it stores where it is run: a call
//! running it (`stack.close()`), the end of a `with` block on it, or wherever its run method is
//! handed to (`to_thread(stack.close)`, `BackgroundTask(stack.close)`).
//!
//! A stack is followed inside the function creating it, as a local variable only ever assigned
//! a new instance (or what a `moves` call returns, `deferred = stack.pop_all()`) and only used to
//! store, run, move or hand its run method. A stack used any other way (handed to another
//! function, returned, stored, read in a nested function) may be run anywhere: what it stores
//! is taken to run when it is stored.
//!
//! Which stores each run sees is a flow over the function's statements, which follows only
//! functions whose statements touching a stack are all of a few shapes it models exactly:
//! storing, closing or moving it as a statement of its own (`stack.callback(f)`,
//! `stack.close()`, `later = stack.pop_all()`), handing its sync run method to calls
//! (`await asyncio.to_thread(stack.close)`), `if` statements, a sync `with` block on the stack
//! alone, and `try` ... `finally` without handlers. Any other statement touching a stack (a loop,
//! `except` handlers, a `with` on other managers, an async run) leaves every stack of the
//! function to the fallback. Statements not touching a stack keep its state; they may raise (but
//! for name and attribute reads and formatting stdlib values) and may return. Exceptions and
//! `return` go through the `with` blocks and `finally` clauses around them, which see the state
//! they leave from, and leave with what those left; a `with` exit may swallow the exception. An
//! awaited hand-off has run the stack when the await returns (only a cancellation would leave it
//! full).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, walk_expr, walk_stmt};
use ruff_python_ast::{self as ast, Expr, Stmt};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::extract::RawSlot;
use crate::index::Key;

/// What a call does to the stack it is called on (catalog `[[exit_stack]]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackRole {
    /// Stores the callable given first.
    Callback,
    /// Enters the context manager given first, and stores its exit.
    Context,
    /// Runs everything stored.
    Run,
    /// Returns a new stack holding everything stored, leaving this one empty.
    Move,
}

/// What the extraction resolves for the flow.
pub(crate) trait StackCalls {
    /// The role every function `call` may call has, if they share one.
    fn role(&self, call: &ast::ExprCall) -> Option<StackRole>;
    /// Whether a `Run` call calls a coroutine function: it runs nothing until awaited.
    fn coroutine(&self, call: &ast::ExprCall) -> bool;
    /// Whether `call` constructs a class.
    fn constructs(&self, call: &ast::ExprCall) -> bool;
    /// Whether leaving a `with` (`async with`) block on `expr` runs what it stores.
    fn exit_runs(&self, expr: &Expr, is_async: bool) -> bool;
    /// Whether `expr` (`stack.close`, not called) is a sync run method.
    fn handed_run(&self, expr: &Expr) -> bool;
    /// Whether reading `attribute` cannot raise: an instance attribute of a plain project class
    /// its `__init__` always assigns (see the implementation).
    fn inert_read(&self, attribute: &ast::ExprAttribute) -> bool;
    /// Whether `expr` is a `str`, `int` or `float` (formatting it without a spec cannot raise).
    fn plain(&self, expr: &Expr) -> bool;
    /// Whether `call` calls `argument` (given at `slot`) when it is a stack's sync run method.
    fn runs_handed(&self, call: &ast::ExprCall, slot: &RawSlot, argument: &Expr) -> bool;
    /// Whether a call handed a stack's run method runs it on the stack itself (not on a copy in
    /// another process).
    fn empties_handed(&self, call: &ast::ExprCall) -> bool;
    /// Whether `call` may reach the calling frame's variables.
    fn frame_access(&self, call: &ast::ExprCall) -> bool;
    /// What `call` calls, and its first string-literal argument.
    fn receivers(&self, call: &ast::ExprCall) -> (Vec<(Key, bool)>, Option<String>);
}

/// Where what a store stores runs.
#[derive(Clone, PartialEq)]
pub(crate) enum ExitSite {
    /// A call running the stack (at `offset`, reading as `text`), or the end of a `with` block
    /// on it (`with`: whether it is `async with`).
    At { offset: u32, text: TextRange, with: Option<bool> },
    /// The run method handed as a callable to the call at `offset`, at `slot`.
    Handed { offset: u32, slot: RawSlot, into: Vec<(Key, bool)>, literal: Option<String> },
}

/// The sites each store (by call offset) runs at, for the stores on stacks the flow follows.
/// A store not in it runs where it is made.
pub(crate) type ExitPlan = FxHashMap<u32, Vec<ExitSite>>;

/// The plan of every function in a module, see [`ExitPlan`]. `names`: the last names of store
/// calls (a function calling none stores nothing).
pub(crate) fn exit_plan(body: &[Stmt], calls: &dyn StackCalls, names: &FxHashSet<&str>) -> ExitPlan {
    struct Functions<'a, 'c> {
        calls: Memo<'c>,
        names: &'a FxHashSet<&'a str>,
        plan: ExitPlan,
    }
    impl<'b> SourceOrderVisitor<'b> for Functions<'_, '_> {
        fn visit_stmt(&mut self, stmt: &'b Stmt) {
            if let Stmt::FunctionDef(def) = stmt {
                function_plan(def, &self.calls, self.names, &mut self.plan);
            }
            walk_stmt(self, stmt);
        }
    }
    let mut functions = Functions { calls: Memo { calls, roles: RefCell::default() }, names, plan: ExitPlan::default() };
    functions.visit_body(body);
    functions.plan
}

/// [`StackCalls::role`], memoized: the flow goes around loops.
struct Memo<'c> {
    calls: &'c dyn StackCalls,
    roles: RefCell<FxHashMap<u32, Option<StackRole>>>,
}

impl Memo<'_> {
    fn role(&self, call: &ast::ExprCall) -> Option<StackRole> {
        let offset = call.start().to_u32();
        if let Some(&role) = self.roles.borrow().get(&offset) {
            return role;
        }
        let role = self.calls.role(call);
        self.roles.borrow_mut().insert(offset, role);
        role
    }
}

/// `name.method(...)`: `name`.
fn receiver_name(call: &ast::ExprCall) -> Option<&ast::ExprName> {
    call.func.as_attribute_expr()?.value.as_name_expr()
}

/// The arguments of a call with their slots, as flows record them.
fn slotted_arguments(call: &ast::ExprCall) -> impl Iterator<Item = (RawSlot, &Expr)> {
    let positional = call.arguments.args.iter().take_while(|a| !a.is_starred_expr()).enumerate();
    let keywords = call.arguments.keywords.iter().filter_map(|k| Some((RawSlot::Keyword(k.arg.as_ref()?.to_string()), &k.value)));
    positional.map(|(i, a)| (RawSlot::Positional(i), a)).chain(keywords)
}

fn function_plan(def: &ast::StmtFunctionDef, calls: &Memo, names: &FxHashSet<&str>, plan: &mut ExitPlan) {
    // Stacks: the local names stores are called on.
    let mut stores = Stores { calls, names, candidates: FxHashSet::default(), assigned_calls: Vec::new() };
    stores.visit_body(&def.body);
    if stores.candidates.is_empty() {
        return;
    }
    // And the names given what a move returns.
    loop {
        let moved = stores.assigned_calls.iter().filter(|(target, call)| {
            !stores.candidates.contains(*target)
                && receiver_name(call).is_some_and(|n| stores.candidates.contains(n.id.as_str()))
                && calls.role(call) == Some(StackRole::Move)
        });
        let moved: Vec<String> = moved.map(|(target, _)| target.to_string()).collect();
        if moved.is_empty() {
            break;
        }
        stores.candidates.extend(moved);
    }
    let mut uses = Uses {
        calls,
        candidates: &stores.candidates,
        allowed: FxHashSet::default(),
        assigned: FxHashSet::default(),
        escaped: FxHashSet::default(),
        moves: Vec::new(),
        nested: 0,
    };
    uses.visit_body(&def.body);
    let parameters = def.parameters.iter().map(|p| p.name().as_str());
    uses.escaped.extend(parameters.filter(|p| stores.candidates.contains(*p)).map(str::to_string));
    let mut tracked: FxHashSet<String> =
        stores.candidates.iter().filter(|c| uses.assigned.contains(*c) && !uses.escaped.contains(*c)).cloned().collect();
    // What moves between two stacks is followed only if both are.
    loop {
        let before = tracked.len();
        for (to, from) in &uses.moves {
            if !tracked.contains(to) || !tracked.contains(from) {
                tracked.remove(to);
                tracked.remove(from);
            }
        }
        if tracked.len() == before {
            break;
        }
    }
    if tracked.is_empty() {
        return;
    }
    let mut flow = Flow {
        calls,
        tracked: &tracked,
        frames: Vec::new(),
        sites: Vec::new(),
        reached: FxHashMap::default(),
        stores: BTreeSet::new(),
        bail: false,
        bound: def.parameters.iter().map(|p| p.name().to_string()).collect(),
        unstable: unstable_names(def),
        open: Vec::new(),
    };
    if reaches_frame(&def.body, calls) {
        return;
    }
    flow.block(&def.body, Some(State::new()));
    // A statement outside what the flow models: every stack of the function is left alone.
    if flow.bail {
        return;
    }
    for store in flow.stores {
        let sites = flow.reached.get(&store).into_iter().flatten().map(|&i| flow.sites[i].clone()).collect();
        plan.insert(store, sites);
    }
}

/// Names whose binding a read cannot trust, anywhere in a function (nested scopes too): deleted,
/// declared `global` or `nonlocal`, bound by an `except ... as` (unbound when it ends), a loop
/// or comprehension target, a `match` pattern, or a parameter assigned again.
fn unstable_names(def: &ast::StmtFunctionDef) -> FxHashSet<String> {
    struct Unstable<'a> {
        names: FxHashSet<String>,
        parameters: Vec<&'a str>,
        targets: u32,
    }
    impl<'b> SourceOrderVisitor<'b> for Unstable<'_> {
        fn visit_stmt(&mut self, stmt: &'b Stmt) {
            match stmt {
                Stmt::Global(ast::StmtGlobal { names, .. }) | Stmt::Nonlocal(ast::StmtNonlocal { names, .. }) => {
                    self.names.extend(names.iter().map(|n| n.to_string()));
                }
                Stmt::Delete(ast::StmtDelete { targets, .. }) => {
                    self.names.extend(targets.iter().filter_map(|t| t.as_name_expr()).map(|n| n.id.to_string()));
                }
                Stmt::Try(try_stmt) => {
                    for handler in &try_stmt.handlers {
                        let ast::ExceptHandler::ExceptHandler(handler) = handler;
                        self.names.extend(handler.name.as_ref().map(|n| n.to_string()));
                    }
                }
                Stmt::For(for_stmt) => {
                    self.targets += 1;
                    self.visit_expr(&for_stmt.target);
                    self.targets -= 1;
                }
                // Every name a pattern may capture: all of them.
                Stmt::Match(_) => {
                    self.names.insert("*".to_string());
                }
                _ => {}
            }
            walk_stmt(self, stmt);
        }

        fn visit_comprehension(&mut self, comprehension: &'b ast::Comprehension) {
            self.targets += 1;
            self.visit_expr(&comprehension.target);
            self.targets -= 1;
            ruff_python_ast::visitor::source_order::walk_comprehension(self, comprehension);
        }

        fn visit_expr(&mut self, expr: &'b Expr) {
            if let Expr::Name(name) = expr
                && name.ctx == ast::ExprContext::Store
                && (self.targets > 0 || self.parameters.contains(&name.id.as_str()))
            {
                self.names.insert(name.id.to_string());
            }
            walk_expr(self, expr);
        }
    }
    let parameters = def.parameters.iter().map(|p| p.name().as_str()).collect();
    let mut unstable = Unstable { names: FxHashSet::default(), parameters, targets: 0 };
    unstable.visit_body(&def.body);
    unstable.names
}

/// Whether a body (nested scopes too) calls something that may reach the frame's variables.
fn reaches_frame(body: &[Stmt], calls: &Memo) -> bool {
    struct Frames<'a, 'c> {
        calls: &'a Memo<'c>,
        found: bool,
    }
    impl<'b> SourceOrderVisitor<'b> for Frames<'_, '_> {
        fn visit_expr(&mut self, expr: &'b Expr) {
            if let Expr::Call(call) = expr
                && !self.found
                && self.calls.calls.frame_access(call)
            {
                self.found = true;
            }
            walk_expr(self, expr);
        }
    }
    let mut frames = Frames { calls, found: false };
    frames.visit_body(body);
    frames.found
}

/// Finds the local names store calls are made on, in a function's own scope, and the names
/// assigned a method call.
struct Stores<'a, 'b, 'c> {
    calls: &'a Memo<'c>,
    names: &'a FxHashSet<&'a str>,
    candidates: FxHashSet<String>,
    assigned_calls: Vec<(&'b str, &'b ast::ExprCall)>,
}

impl<'b> SourceOrderVisitor<'b> for Stores<'_, 'b, '_> {
    fn visit_stmt(&mut self, stmt: &'b Stmt) {
        let assigned = match stmt {
            Stmt::Assign(ast::StmtAssign { targets, value, .. }) if targets.len() == 1 => Some((&targets[0], &**value)),
            Stmt::AnnAssign(ast::StmtAnnAssign { target, value: Some(value), .. }) => Some((&**target, &**value)),
            _ => None,
        };
        if let Some((Expr::Name(target), Expr::Call(call))) = assigned
            && receiver_name(call).is_some()
        {
            self.assigned_calls.push((target.id.as_str(), call));
        }
        if !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
            walk_stmt(self, stmt);
        }
    }

    fn visit_expr(&mut self, expr: &'b Expr) {
        match expr {
            Expr::Lambda(_) | Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {}
            Expr::Call(call) => {
                            if let Some(name) = receiver_name(call)
                    && call.func.as_attribute_expr().is_some_and(|a| self.names.contains(a.attr.as_str()))
                    && matches!(self.calls.role(call), Some(StackRole::Callback | StackRole::Context))
                {
                    self.candidates.insert(name.id.to_string());
                }
                walk_expr(self, expr);
            }
            _ => walk_expr(self, expr),
        }
    }
}

/// Sorts the uses of candidate stacks: those the flow follows, and the others (escapes).
struct Uses<'a, 'c> {
    calls: &'a Memo<'c>,
    candidates: &'a FxHashSet<String>,
    /// Name expressions (by offset) the flow follows.
    allowed: FxHashSet<u32>,
    /// Candidates assigned a new stack (or a moved one).
    assigned: FxHashSet<String>,
    escaped: FxHashSet<String>,
    /// `(to, from)`: `to = from.pop_all()`.
    moves: Vec<(String, String)>,
    /// Depth in nested scopes, where every use escapes.
    nested: u32,
}

impl Uses<'_, '_> {
    fn candidate<'e>(&self, expr: &'e Expr) -> Option<&'e ast::ExprName> {
        expr.as_name_expr().filter(|n| self.candidates.contains(n.id.as_str()))
    }

    /// `target = value`: a new stack, or the one a move returns.
    fn assign(&mut self, target: &Expr, value: &Expr) {
        let (Some(target), Expr::Call(call)) = (self.candidate(target), value) else { return };
        let moved = receiver_name(call).filter(|n| self.candidates.contains(n.id.as_str()));
        if let Some(from) = moved.filter(|_| self.calls.role(call) == Some(StackRole::Move)) {
            self.allowed.insert(from.start().to_u32());
            self.moves.push((target.id.to_string(), from.id.to_string()));
        } else if !self.calls.calls.constructs(call) {
            return;
        }
        self.allowed.insert(target.start().to_u32());
        self.assigned.insert(target.id.to_string());
    }
}

impl<'b> SourceOrderVisitor<'b> for Uses<'_, '_> {
    fn visit_stmt(&mut self, stmt: &'b Stmt) {
        if self.nested == 0 {
            match stmt {
                Stmt::Assign(assign) if assign.targets.len() == 1 => self.assign(&assign.targets[0], &assign.value),
                Stmt::AnnAssign(ast::StmtAnnAssign { target, value: Some(value), .. }) => self.assign(target, value),
                Stmt::Expr(ast::StmtExpr { value, .. }) => {
                    if let Expr::Call(call) = &**value
                        && let Some(from) = receiver_name(call).filter(|n| self.candidates.contains(n.id.as_str()))
                        && self.calls.role(call) == Some(StackRole::Move)
                    {
                        self.allowed.insert(from.start().to_u32());
                    }
                }
                Stmt::With(with) => {
                    for item in &with.items {
                        let context = &item.context_expr;
                        match (context, item.optional_vars.as_deref().and_then(|v| self.candidate(v))) {
                            (Expr::Call(call), Some(target))
                                if self.calls.calls.constructs(call) && self.calls.calls.exit_runs(context, with.is_async) =>
                            {
                                self.allowed.insert(target.start().to_u32());
                                self.assigned.insert(target.id.to_string());
                            }
                            (Expr::Name(name), None)
                                if self.candidates.contains(name.id.as_str())
                                    && self.calls.calls.exit_runs(context, with.is_async) =>
                            {
                                self.allowed.insert(name.start().to_u32());
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        match stmt {
            Stmt::Global(ast::StmtGlobal { names, .. }) | Stmt::Nonlocal(ast::StmtNonlocal { names, .. }) => {
                let named = names.iter().filter(|n| self.candidates.contains(n.as_str()));
                self.escaped.extend(named.map(|n| n.to_string()));
            }
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                self.nested += 1;
                walk_stmt(self, stmt);
                self.nested -= 1;
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'b Expr) {
        match expr {
            Expr::Lambda(_) | Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {
                self.nested += 1;
                walk_expr(self, expr);
                self.nested -= 1;
            }
            Expr::Call(call) if self.nested == 0 => {
                if let Some(name) = receiver_name(call).filter(|n| self.candidates.contains(n.id.as_str()))
                    && matches!(self.calls.role(call), Some(StackRole::Callback | StackRole::Context | StackRole::Run))
                {
                    self.allowed.insert(name.start().to_u32());
                }
                // Handed to a call nothing resolves, the run method may run anywhere.
                for (_, argument) in slotted_arguments(call) {
                    if let Some(attribute) = argument.as_attribute_expr()
                        && let Some(name) = self.candidate(&attribute.value)
                        && self.calls.calls.handed_run(argument)
                        && !self.calls.calls.receivers(call).0.is_empty()
                    {
                        self.allowed.insert(name.start().to_u32());
                    }
                }
                walk_expr(self, expr);
            }
            Expr::Name(name) if self.candidates.contains(name.id.as_str()) => {
                if self.nested > 0 || !self.allowed.contains(&name.start().to_u32()) {
                    self.escaped.insert(name.id.to_string());
                }
            }
            _ => walk_expr(self, expr),
        }
    }
}

/// The stores (by call offset) each followed stack holds.
type State = BTreeMap<String, BTreeSet<u32>>;

fn join(into: &mut Option<State>, other: Option<State>) {
    let Some(other) = other else { return };
    match into {
        None => *into = Some(other),
        Some(state) => {
            for (stack, stores) in other {
                state.entry(stack).or_default().extend(stores);
            }
        }
    }
}

/// Leaving every stack named in `stacks` empty.
fn emptied(state: &Option<State>, stacks: &[&str]) -> Option<State> {
    state.clone().map(|mut state| {
        for stack in stacks {
            state.remove(*stack);
        }
        state
    })
}

/// A `with` block on a stack, or a `try` with a `finally`: the states raising and returning
/// inside it.
#[derive(Default)]
struct Frame {
    raised: Option<State>,
    returned: Option<State>,
}

struct Flow<'a, 'c> {
    calls: &'a Memo<'c>,
    tracked: &'a FxHashSet<String>,
    frames: Vec<Frame>,
    sites: Vec<ExitSite>,
    /// The sites (by index) each store reaches.
    reached: FxHashMap<u32, BTreeSet<usize>>,
    stores: BTreeSet<u32>,
    /// A statement touching a stack is outside what the flow models.
    bail: bool,
    /// Names bound on every path to the statement being walked.
    bound: Vec<String>,
    /// Names the function deletes or declares `global`/`nonlocal` (at any depth).
    unstable: FxHashSet<String>,
    /// Stacks whose `with` block is being walked.
    open: Vec<String>,
}

/// Whether a statement (or expression) names a followed stack.
struct Mentions<'a> {
    tracked: &'a FxHashSet<String>,
    found: usize,
}

impl<'b> SourceOrderVisitor<'b> for Mentions<'_> {
    fn visit_expr(&mut self, expr: &'b Expr) {
        if let Expr::Name(name) = expr
            && self.tracked.contains(name.id.as_str())
        {
            self.found += 1;
        }
        walk_expr(self, expr);
    }
}

/// Whether a statement holds a `return` (outside nested scopes).
pub(crate) fn returns(stmt: &Stmt) -> bool {
    struct Returns(bool);
    impl<'b> SourceOrderVisitor<'b> for Returns {
        fn visit_stmt(&mut self, stmt: &'b Stmt) {
            match stmt {
                Stmt::Return(_) => self.0 = true,
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
                _ => walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, _: &'b Expr) {}
    }
    let mut found = Returns(false);
    found.visit_stmt(stmt);
    found.0
}

/// `name.method(...)` on a followed stack: `name`.
fn stack_receiver<'e>(call: &'e ast::ExprCall, tracked: &FxHashSet<String>) -> Option<&'e str> {
    receiver_name(call).map(|n| n.id.as_str()).filter(|n| tracked.contains(*n))
}

/// The calls handing a followed stack's run method in an expression: `(call, slot, stack,
/// awaited)`, outside nested scopes. Awaited: the call is the awaited expression itself (one
/// nested in another awaited call may still be running when that await returns).
struct Handoffs<'b> {
    found: Vec<(&'b ast::ExprCall, RawSlot, &'b Expr, &'b str, bool)>,
    awaited: Option<TextRange>,
}

impl<'b> SourceOrderVisitor<'b> for Handoffs<'b> {
    fn visit_expr(&mut self, expr: &'b Expr) {
        match expr {
            Expr::Lambda(_) | Expr::ListComp(_) | Expr::SetComp(_) | Expr::DictComp(_) | Expr::Generator(_) => {}
            Expr::Await(ast::ExprAwait { value, .. }) => {
                if let Expr::Call(call) = &**value {
                    self.awaited = Some(call.range());
                }
                walk_expr(self, expr);
            }
            Expr::Call(call) => {
                let awaited = self.awaited == Some(call.range());
                for (slot, argument) in slotted_arguments(call) {
                    if let Some(name) = argument.as_attribute_expr().and_then(|a| a.value.as_name_expr()) {
                        self.found.push((call, slot, argument, name.id.as_str(), awaited));
                    }
                }
                walk_expr(self, expr);
            }
            _ => walk_expr(self, expr),
        }
    }
}

impl Flow<'_, '_> {
    fn run(&mut self, stack: &str, state: &State, site: ExitSite) {
        let Some(stores) = state.get(stack).filter(|s| !s.is_empty()) else { return };
        let index = match self.sites.iter().position(|s| *s == site) {
            Some(i) => i,
            None => {
                self.sites.push(site);
                self.sites.len() - 1
            }
        };
        for &store in stores {
            self.reached.entry(store).or_default().insert(index);
        }
    }

    /// Raising from `state`: the innermost `with` block or `finally` sees it.
    fn raise(&mut self, state: &State) {
        if let Some(frame) = self.frames.last_mut() {
            join(&mut frame.raised, Some(state.clone()));
        }
    }

    /// Returning from `state`.
    fn ret(&mut self, state: &State) {
        if let Some(frame) = self.frames.last_mut() {
            join(&mut frame.returned, Some(state.clone()));
        }
    }

    fn mentions(&self, stmt: Option<&Stmt>, expr: Option<&Expr>) -> usize {
        let mut mentions = Mentions { tracked: self.tracked, found: 0 };
        if let Some(stmt) = stmt {
            mentions.visit_stmt(stmt);
        }
        if let Some(expr) = expr {
            mentions.visit_expr(expr);
        }
        mentions.found
    }

    /// Whether evaluating `expr` cannot raise: literals, names bound on every path, attribute
    /// reads [`StackCalls::inert_read`] allows on them, and f-strings formatting those (`str`,
    /// `int` or `float`, without spec or conversion).
    fn inert(&self, expr: &Expr) -> bool {
        let read = |name: &ast::ExprName| {
            self.bound.iter().any(|b| b == name.id.as_str())
                && !self.unstable.contains(name.id.as_str())
                && !self.unstable.contains("*")
        };
        match expr {
            Expr::NoneLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::EllipsisLiteral(_) => true,
            Expr::Name(name) => read(name),
            Expr::Attribute(attribute) => {
                attribute.value.as_name_expr().is_some_and(read) && self.calls.calls.inert_read(attribute)
            }
            Expr::FString(fstring) => fstring.value.iter().all(|part| match part {
                ast::FStringPartRef::Literal(_) => true,
                ast::FStringPartRef::FString(f) => f.elements.iter().all(|element| match element {
                    ast::InterpolatedStringElement::Literal(_) => true,
                    ast::InterpolatedStringElement::Interpolation(i) => {
                        i.debug_text.is_none()
                            && i.conversion == ast::ConversionFlag::None
                            && i.format_spec.is_none()
                            && self.inert(&i.expression)
                            && self.calls.calls.plain(&i.expression)
                    }
                }),
            }),
            _ => false,
        }
    }

    /// Binding `name` with a statement that completed.
    fn bind(&mut self, target: &Expr) {
        if let Expr::Name(name) = target {
            self.bound.push(name.id.to_string());
        }
    }

    fn inert_stmt(&self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Pass(_) | Stmt::AnnAssign(ast::StmtAnnAssign { value: None, .. }) => true,
            Stmt::Expr(ast::StmtExpr { value, .. }) => value.is_string_literal_expr(),
            Stmt::Assign(ast::StmtAssign { targets, value, .. }) => {
                matches!(&targets[..], [Expr::Name(_)]) && self.inert(value)
            }
            Stmt::AnnAssign(ast::StmtAnnAssign { target, value: Some(value), .. }) => {
                target.is_name_expr() && self.inert(value)
            }
            _ => false,
        }
    }

    fn block(&mut self, body: &[Stmt], mut state: Option<State>) -> Option<State> {
        let mark = self.bound.len();
        for stmt in body {
            if self.bail {
                break;
            }
            let Some(before) = state else { break };
            state = self.stmt(stmt, before);
            match stmt {
                Stmt::Assign(ast::StmtAssign { targets, .. }) if targets.len() == 1 => self.bind(&targets[0]),
                Stmt::AnnAssign(ast::StmtAnnAssign { target, value: Some(_), .. }) => self.bind(target),
                _ => {}
            }
        }
        self.bound.truncate(mark);
        if self.bail { None } else { state }
    }

    fn stmt(&mut self, stmt: &Stmt, state: State) -> Option<State> {
        if self.mentions(Some(stmt), None) == 0 {
            return self.untouched(stmt, state);
        }
        match stmt {
            Stmt::If(if_stmt) => {
                let tests = std::iter::once(&*if_stmt.test).chain(if_stmt.elif_else_clauses.iter().filter_map(|c| c.test.as_ref()));
                for test in tests {
                    if self.mentions(None, Some(test)) > 0 {
                        self.bail = true;
                        return None;
                    }
                }
                if !self.inert(&if_stmt.test) {
                    self.raise(&state);
                }
                let mut out = self.block(&if_stmt.body, Some(state.clone()));
                let mut otherwise = true;
                for clause in &if_stmt.elif_else_clauses {
                    match &clause.test {
                        Some(test) if !self.inert(test) => self.raise(&state),
                        Some(_) => {}
                        None => otherwise = false,
                    }
                    let branch = self.block(&clause.body, Some(state.clone()));
                    join(&mut out, branch);
                }
                if otherwise {
                    join(&mut out, Some(state));
                }
                out
            }
            Stmt::With(with) => self.with(with, state),
            Stmt::Try(try_stmt) if try_stmt.handlers.is_empty() && try_stmt.orelse.is_empty() && !try_stmt.is_star => {
                self.try_finally(try_stmt, state)
            }
            Stmt::Expr(ast::StmtExpr { value, .. }) => self.simple(None, value, false, state),
            Stmt::Assign(ast::StmtAssign { targets, value, .. }) if targets.len() == 1 => {
                self.simple(Some(&targets[0]), value, false, state)
            }
            Stmt::AnnAssign(ast::StmtAnnAssign { target, value: Some(value), .. }) => {
                self.simple(Some(target), value, false, state)
            }
            Stmt::Return(ast::StmtReturn { value: Some(value), .. }) => self.simple(None, value, true, state),
            _ => {
                self.bail = true;
                None
            }
        }
    }

    /// A statement naming no stack: it keeps the state, and may raise and return.
    fn untouched(&mut self, stmt: &Stmt, state: State) -> Option<State> {
        match stmt {
            Stmt::Return(ast::StmtReturn { value, .. }) => {
                if value.as_deref().is_some_and(|v| !self.inert(v)) {
                    self.raise(&state);
                }
                self.ret(&state);
                None
            }
            Stmt::Raise(_) => {
                self.raise(&state);
                None
            }
            Stmt::Break(_) | Stmt::Continue(_) => {
                self.bail = true;
                None
            }
            _ => {
                if !self.inert_stmt(stmt) {
                    self.raise(&state);
                }
                if returns(stmt) {
                    self.ret(&state);
                }
                Some(state)
            }
        }
    }

    /// `[target =] value` or `return value` naming a stack: storing on it, running it, moving
    /// it, creating it, or handing its run method to calls. Any other shape bails.
    fn simple(&mut self, target: Option<&Expr>, value: &Expr, returning: bool, mut state: State) -> Option<State> {
        let tracked = self.tracked;
        if let Some(target) = target
            && self.mentions(None, Some(target)) > 0
        {
            // A new stack, or the one a move returns.
            let (Expr::Name(target), Expr::Call(call)) = (target, value) else {
                self.bail = true;
                return None;
            };
            // Rebinding a stack that holds stores, or whose `with` block is open.
            if !self.rebindable(target.id.as_str(), &state) {
                self.bail = true;
                return None;
            }
            match stack_receiver(call, tracked) {
                Some(from) if self.calls.role(call) == Some(StackRole::Move) && call.arguments.is_empty() => {
                    let stores = state.remove(from).unwrap_or_default();
                    state.insert(target.id.to_string(), stores);
                }
                None if self.mentions(None, Some(value)) == 0 => {
                    self.raise(&state);
                    state.insert(target.id.to_string(), BTreeSet::new());
                }
                _ => {
                    self.bail = true;
                    return None;
                }
            }
            return Some(state);
        }
        let (inner, awaited) = match value {
            Expr::Await(ast::ExprAwait { value, .. }) => (&**value, true),
            value => (value, false),
        };
        if let Expr::Call(call) = inner
            && let Some(stack) = stack_receiver(call, tracked)
        {
            if self.mentions(None, Some(value)) != 1 {
                self.bail = true;
                return None;
            }
            match self.calls.role(call) {
                Some(StackRole::Callback | StackRole::Context) if !awaited => {
                    self.raise(&state);
                    state.entry(stack.to_string()).or_default().insert(call.start().to_u32());
                    self.stores.insert(call.start().to_u32());
                    // A target other than a name is assigned after the store, and may raise.
                    if target.is_some_and(|t| !t.is_name_expr()) {
                        self.raise(&state);
                    }
                }
                Some(StackRole::Run) if !awaited && !self.calls.calls.coroutine(call) => {
                    let site = ExitSite::At { offset: call.start().to_u32(), text: call.func.range(), with: None };
                    self.run(stack, &state, site);
                    // What it runs may raise, after running everything.
                    state.remove(stack);
                    self.raise(&state);
                }
                Some(StackRole::Move) if !awaited && target.is_none() && !returning => {
                    state.remove(stack);
                }
                _ => {
                    self.bail = true;
                    return None;
                }
            }
        } else {
            let mut handoffs = Handoffs { found: Vec::new(), awaited: None };
            handoffs.visit_expr(value);
            let handoffs: Vec<_> = handoffs.found.into_iter().filter(|(_, _, _, stack, _)| tracked.contains(*stack)).collect();
            let valid = handoffs.iter().all(|(call, slot, argument, _, _)| self.calls.calls.runs_handed(call, slot, argument));
            if !valid || handoffs.len() != self.mentions(None, Some(value)) {
                self.bail = true;
                return None;
            }
            let before = state.clone();
            for (call, slot, _, stack, awaited) in handoffs {
                let (into, literal) = self.calls.calls.receivers(call);
                let site = ExitSite::Handed { offset: call.start().to_u32(), slot, into, literal };
                self.run(stack, &before, site);
                // Awaiting the call handed the run method waits for it to run.
                if awaited && self.calls.calls.empties_handed(call) {
                    state.remove(stack);
                }
            }
            self.raise(&state);
        }
        if returning {
            self.ret(&state);
            return None;
        }
        Some(state)
    }

    /// A sync `with` block on one stack alone (`with stack:`, `with ExitStack() as stack:`).
    fn with(&mut self, with: &ast::StmtWith, mut state: State) -> Option<State> {
        let stack = match &with.items[..] {
            [item] if !with.is_async => match (&item.context_expr, item.optional_vars.as_deref()) {
                (Expr::Call(_), Some(Expr::Name(target)))
                    if self.tracked.contains(target.id.as_str())
                        && self.mentions(None, Some(&item.context_expr)) == 0
                        && self.rebindable(target.id.as_str(), &state) =>
                {
                    self.raise(&state);
                    state.insert(target.id.to_string(), BTreeSet::new());
                    target.id.as_str()
                }
                (Expr::Name(name), None) if self.tracked.contains(name.id.as_str()) => name.id.as_str(),
                _ => "",
            },
            _ => "",
        };
        if stack.is_empty() {
            self.bail = true;
            return None;
        }
        let context = &with.items[0].context_expr;
        self.frames.push(Frame::default());
        self.open.push(stack.to_string());
        let mark = self.bound.len();
        if let Some(target) = with.items[0].optional_vars.as_deref() {
            self.bind(target);
        }
        let end = self.block(&with.body, Some(state));
        self.bound.truncate(mark);
        self.open.pop();
        let frame = self.frames.pop().unwrap_or_default();
        if self.bail {
            return None;
        }
        let mut leaving = end.clone();
        join(&mut leaving, frame.raised.clone());
        join(&mut leaving, frame.returned.clone());
        if let Some(leaving) = &leaving {
            let site = ExitSite::At { offset: context.start().to_u32(), text: context.range(), with: Some(false) };
            self.run(stack, leaving, site);
        }
        let (end, raised, returned) = (emptied(&end, &[stack]), emptied(&frame.raised, &[stack]), emptied(&frame.returned, &[stack]));
        // What the exit runs may raise; an exit it runs may swallow the exception.
        if let Some(end) = &end {
            self.raise(end);
        }
        if let Some(raised) = &raised {
            self.raise(raised);
        }
        if let Some(returned) = &returned {
            self.ret(returned);
        }
        let mut out = end;
        join(&mut out, raised);
        out
    }

    /// Whether a stack name may be bound to a new stack here: it holds no stores and no `with`
    /// block on it is open.
    fn rebindable(&self, name: &str, state: &State) -> bool {
        state.get(name).is_none_or(BTreeSet::is_empty) && !self.open.iter().any(|o| o == name)
    }

    /// `try` ... `finally` without handlers: the `finally` runs on every way out, which goes on
    /// with what it leaves.
    fn try_finally(&mut self, try_stmt: &ast::StmtTry, state: State) -> Option<State> {
        self.frames.push(Frame::default());
        let end = self.block(&try_stmt.body, Some(state));
        let frame = self.frames.pop().unwrap_or_default();
        let finally = |this: &mut Self, entry: Option<State>| entry.and_then(|e| this.block(&try_stmt.finalbody, Some(e)));
        if let Some(raised) = finally(self, frame.raised) {
            self.raise(&raised);
        }
        if let Some(returned) = finally(self, frame.returned) {
            self.ret(&returned);
        }
        finally(self, end)
    }
}
