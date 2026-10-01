//! Walks function bodies and records calls (with their resolved callees) and callable flows.

use hydroid_core::facts::{Accessor, Location};
use ruff_db::files::{File, FilePath};
use ruff_db::parsed::parsed_module;
use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, walk_expr, walk_stmt};
use ruff_python_ast::{self as ast, AtomicNodeIndex, Expr, ExprContext, Stmt};
use ruff_text_size::{Ranged, TextRange};
use rustc_hash::FxHashSet;
use ty_python_core::definition::{Definition, DefinitionKind};
use ty_python_semantic::types::ide_support::{ResolvedDefinition, call_signature_details};
use ty_python_semantic::types::{Type, TypeDefinition};
use ty_python_semantic::{
    HasType, ImportAliasResolution, SemanticModel, definitions_for_attribute, definitions_for_name,
};

use crate::db::HydroidDb;
use crate::index::{FileIndex, Key, MODULE_KEY, is_class_object};

pub struct RawCall {
    pub caller: u32,
    pub location: Location,
    pub text: String,
    /// `(callee, receiver bound)`: when bound, the first parameter is not passed positionally.
    pub targets: Vec<(Key, bool)>,
    pub virtual_dispatch: bool,
    pub param: Option<(u32, String)>,
    pub awaited: bool,
    pub iterates: bool,
    /// The class called (or validated into), see [`hydroid_core::facts::Call::constructs`].
    pub constructs: Option<Key>,
    pub unresolved: Option<String>,
    /// An attribute access; only kept if a target turns out to be this part of a property.
    pub accessor: Option<Accessor>,
    /// See [`hydroid_core::facts::Call::attribute`].
    pub attribute: Option<(Key, String)>,
}

pub struct RawStore {
    pub class: Key,
    pub name: String,
    pub value: RawValue,
}

#[derive(Clone)]
pub enum RawSlot {
    Positional(usize),
    Keyword(String),
    Decorated,
}

#[derive(Clone)]
pub enum RawValue {
    Function(Key),
    Class(Key),
    Param(u32, String),
}

pub struct RawFlow {
    pub caller: u32,
    pub location: Location,
    pub into: Key,
    pub bound: bool,
    pub slot: RawSlot,
    pub value: RawValue,
    pub literal: Option<String>,
}

pub struct FileFacts {
    pub file: File,
    pub calls: Vec<RawCall>,
    pub flows: Vec<RawFlow>,
    pub stores: Vec<RawStore>,
    pub decorators: Vec<(u32, Vec<Key>)>,
}

/// Extracts the bodies of `wanted` functions (every function and the module body when `None`).
pub fn extract_file(
    db: &HydroidDb,
    index: &FileIndex,
    wanted: Option<&FxHashSet<u32>>,
    properties: Option<&FxHashSet<String>>,
) -> FileFacts {
    let program_file = ty_python_semantic::Db::program_file(db, index.file);
    let model = SemanticModel::new(db, program_file);
    let parsed = parsed_module(db, model.python_file()).load(db);
    let mut extractor = Extractor {
        model: &model,
        index,
        wanted,
        properties,
        current: MODULE_KEY,
        awaited: FxHashSet::default(),
        iterated: FxHashSet::default(),
        callees: FxHashSet::default(),
        out: FileFacts {
            file: index.file,
            calls: Vec::new(),
            flows: Vec::new(),
            stores: Vec::new(),
            decorators: Vec::new(),
        },
    };
    extractor.visit_body(&parsed.syntax().body);
    extractor.out
}

struct Resolution {
    targets: Vec<(Key, bool)>,
    virtual_dispatch: bool,
    constructs: Option<Key>,
    reason: String,
}

/// Builtins that iterate their positional arguments during the call.
const CONSUMERS: &[&str] = &[
    "list", "tuple", "set", "frozenset", "dict", "sorted", "sum", "min", "max", "any", "all", "next", "iter", "join",
    "extend", "update", "reduce",
];

/// Builtins returning lazy iterators over their arguments: iterating one iterates those.
const LAZY: &[&str] = &["map", "filter", "zip", "enumerate", "reversed", "iter"];

/// Pydantic classmethods that validate input into the model (running its validators).
const VALIDATES: &[&str] = &["model_validate", "model_validate_json", "model_validate_strings", "parse_obj", "parse_raw"];

struct Extractor<'a, 'db> {
    model: &'a SemanticModel<'db>,
    index: &'a FileIndex,
    wanted: Option<&'a FxHashSet<u32>>,
    /// Only attribute reads with these names can be property calls (`None`: any name).
    properties: Option<&'a FxHashSet<String>>,
    current: u32,
    awaited: FxHashSet<u32>,
    /// Calls whose result is iterated right away (see [`Extractor::iterate`]).
    iterated: FxHashSet<u32>,
    /// Ranges of callee expressions (attribute reads there are calls, not property reads).
    callees: FxHashSet<(u32, u32)>,
    out: FileFacts,
}

fn first_string_literal(call: &ast::ExprCall) -> Option<String> {
    match call.arguments.args.first()? {
        Expr::StringLiteral(s) => Some(s.value.to_str().to_string()),
        _ => None,
    }
}

impl<'db> Extractor<'_, 'db> {
    fn recording(&self) -> bool {
        self.wanted.is_none_or(|w| w.contains(&self.current))
    }

    fn text(&self, range: TextRange) -> String {
        let text: String = self.index.text(range).split_whitespace().collect::<Vec<_>>().join(" ");
        if text.chars().count() > 80 { text.chars().take(77).chain("...".chars()).collect() } else { text }
    }

    fn def_key(&self, def: Definition<'db>) -> Option<(Key, bool)> {
        let db = self.model.db();
        let is_function = match def.kind(db) {
            DefinitionKind::Function(_) => true,
            DefinitionKind::Class(_) => false,
            DefinitionKind::Assignment(assignment) => return self.alias_key(def, assignment.value(&parsed_module(db, def.python_file(db)).load(db))),
            _ => return None,
        };
        let module = parsed_module(db, def.python_file(db)).load(db);
        let range = def.focus_range(db, &module);
        Some(((range.file(), range.start().to_u32()), is_function))
    }

    /// `__enter__ = acquire` in a class body: the function an alias names.
    fn alias_key(&self, def: Definition<'db>, value: &Expr) -> Option<(Key, bool)> {
        let Expr::Name(name) = value else { return None };
        let db = self.model.db();
        let model = SemanticModel::new(db, ty_python_semantic::Db::program_file(db, def.file(db)));
        definitions_for_name(&model, &name.id, value.into(), ImportAliasResolution::ResolveAliases)
            .into_iter()
            .find_map(|d| match d {
                ResolvedDefinition::Definition(def) if matches!(def.kind(db), DefinitionKind::Function(_)) => {
                    self.def_key(def)
                }
                _ => None,
            })
    }

    fn function_key(&self, def: Definition<'db>) -> Option<Key> {
        self.def_key(def).filter(|(_, f)| *f).map(|(k, _)| k)
    }

    fn resolved_function_keys(&self, defs: Vec<ResolvedDefinition<'db>>) -> Vec<Key> {
        defs.into_iter()
            .filter_map(|d| match d {
                ResolvedDefinition::Definition(def) => self.function_key(def),
                _ => None,
            })
            .collect()
    }

    /// The parameter `name` refers to, looking through enclosing functions (closures).
    fn param_of(&self, expr: &Expr) -> Option<(u32, String)> {
        let Expr::Name(name) = expr else { return None };
        let mut scope = Some(self.current);
        while let Some(key) = scope {
            let entry = self.index.functions.get(&key)?;
            if entry.params.iter().any(|p| p == name.id.as_str()) {
                return Some((key, name.id.to_string()));
            }
            scope = entry.parent;
        }
        None
    }

    fn is_super_call(expr: &Expr) -> bool {
        matches!(expr, Expr::Call(c) if matches!(&*c.func, Expr::Name(n) if n.id == "super"))
    }

    /// Whether `callee` passes its receiver implicitly (`obj.method`, but not `module.function`
    /// or `Class.function`).
    fn receiver_bound(&self, callee: &Expr) -> bool {
        let Expr::Attribute(attr) = callee else { return false };
        match attr.value.inferred_type(self.model) {
            Some(Type::ModuleLiteral(_)) => false,
            Some(ty) => !is_class_object(&ty),
            None => true,
        }
    }

    fn resolve(&self, callee: &Expr, call: Option<&ast::ExprCall>) -> Resolution {
        let db = self.model.db();
        let env = self.model.program_environment();
        let ty = callee.inferred_type(self.model);
        let mut resolution =
            Resolution { targets: Vec::new(), virtual_dispatch: false, constructs: None, reason: String::new() };
        match ty {
            Some(ty @ (Type::FunctionLiteral(_) | Type::BoundMethod(_))) => {
                if let Some(TypeDefinition::Function(def)) = ty.definition(db, &env)
                    && let Some(key) = self.function_key(def)
                {
                    let bound = matches!(ty, Type::BoundMethod(_));
                    resolution.targets.push((key, bound));
                    resolution.virtual_dispatch = bound
                        && matches!(callee, Expr::Attribute(a) if !Self::is_super_call(&a.value));
                }
            }
            Some(ty) => {
                if let Some(call) = call {
                    let bound = is_class_object(&ty)
                        || matches!(ty, Type::NominalInstance(_))
                        || self.receiver_bound(callee);
                    for details in call_signature_details(self.model, call) {
                        if let Some(key) = details.definition.and_then(|d| self.function_key(d))
                            && !resolution.targets.iter().any(|(k, _)| *k == key)
                        {
                            resolution.targets.push((key, bound));
                        }
                    }
                    resolution.virtual_dispatch = matches!(callee, Expr::Attribute(_)) && !is_class_object(&ty);
                }
                if is_class_object(&ty) {
                    resolution.constructs = self.class_key(&ty);
                }
                resolution.reason = match ty {
                    Type::Dynamic(_) => "the callee's type is unknown".to_string(),
                    Type::Callable(_) => "the callee is a callable type without a definition".to_string(),
                    other => {
                        let name = format!("{other:?}");
                        let name = name.split(['(', ' ']).next().unwrap_or_default().to_string();
                        format!("no definition for a `{name}` callee")
                    }
                };
            }
            None => resolution.reason = "the callee has no inferred type".to_string(),
        }
        if let Expr::Attribute(attr) = callee
            && VALIDATES.contains(&attr.attr.as_str())
            && let Some(receiver) = attr.value.inferred_type(self.model)
            && is_class_object(&receiver)
        {
            resolution.constructs = self.class_key(&receiver);
        }
        // Decorated functions often lose their identity in the type (or become callable objects,
        // like `functools.lru_cache` wrappers); go-to-definition keeps it.
        if resolution.targets.is_empty() || matches!(ty, Some(Type::NominalInstance(_))) {
            let keys = match callee {
                Expr::Name(name) => self.resolved_function_keys(definitions_for_name(
                    self.model,
                    &name.id,
                    callee.into(),
                    ImportAliasResolution::ResolveAliases,
                )),
                Expr::Attribute(attr) => self.resolved_function_keys(definitions_for_attribute(self.model, attr)),
                _ => Vec::new(),
            };
            let bound = self.receiver_bound(callee);
            for key in keys {
                if !resolution.targets.iter().any(|(k, _)| *k == key) {
                    resolution.targets.push((key, bound));
                }
            }
        }
        resolution
    }

    fn class_key(&self, ty: &Type<'db>) -> Option<Key> {
        match ty.definition(self.model.db(), &self.model.program_environment())? {
            TypeDefinition::StaticClass(def) => self.def_key(def).map(|(k, _)| k),
            _ => None,
        }
    }

    /// The name of a builtin (or `functools.reduce`) the resolved call targets.
    fn builtin_name<'e>(&self, func: &'e Expr, resolution: &Resolution) -> Option<&'e str> {
        let db = self.model.db();
        let is_builtin = |file: File| match file.path(db) {
            FilePath::Vendored(path) => matches!(path.as_str(), "stdlib/builtins.pyi" | "stdlib/functools.pyi"),
            _ => false,
        };
        // Builtin classes like `tuple` have no constructor definition: check the class itself.
        let by_targets =
            !resolution.targets.is_empty() && resolution.targets.iter().all(|((file, _), _)| is_builtin(*file));
        if !by_targets && !resolution.constructs.is_some_and(|(file, _)| is_builtin(file)) {
            return None;
        }
        match func {
            Expr::Name(name) => Some(name.id.as_str()),
            Expr::Attribute(attr) => Some(attr.attr.as_str()),
            _ => None,
        }
    }

    /// Iterating `expr` right away: its `__iter__` runs, and, for a generator created by a call,
    /// the generator's body. Lazy builtins (`map(f, xs)`) iterate their own arguments.
    fn iterate(&mut self, expr: &Expr) {
        if let Expr::Call(call) = expr {
            if let Expr::Name(name) = &*call.func
                && LAZY.contains(&name.id.as_str())
            {
                let skip = usize::from(matches!(name.id.as_str(), "map" | "filter"));
                for argument in call.arguments.args.iter().skip(skip) {
                    self.iterate(argument);
                }
                return;
            }
            self.iterated.insert(call.start().to_u32());
        }
        let text = format!("for ... in {}", self.text(expr.range()));
        self.implicit_call(expr, "__iter__", text, false, true);
    }

    /// A callable argument the callee calls during the call (`sorted(xs, key=f)`).
    fn call_value(&mut self, call: &ast::ExprCall, value: &Expr) {
        let (targets, param) = match self.value_of(value) {
            Some(RawValue::Function(key)) => (vec![(key, false)], None),
            Some(RawValue::Param(owner, name)) => (Vec::new(), Some((owner, name))),
            _ => return,
        };
        self.out.calls.push(RawCall {
            caller: self.current,
            location: self.index.location(call.start()),
            text: self.text(value.range()),
            targets,
            virtual_dispatch: false,
            param,
            awaited: false,
            iterates: false,
            constructs: None,
            unresolved: None,
            accessor: None,
            attribute: None,
        });
    }

    /// `obj.name(...)` where `name` is no method of `obj`'s class: an attribute holding a callable.
    fn attribute_slot(&self, callee: &Expr) -> Option<(Key, String)> {
        let Expr::Attribute(attr) = callee else { return None };
        if let Some(class) = self.self_class(&attr.value) {
            return Some((class, attr.attr.to_string()));
        }
        let receiver = attr.value.inferred_type(self.model)?;
        if !matches!(receiver, Type::NominalInstance(_)) {
            return None;
        }
        Some((self.class_key(&receiver)?, attr.attr.to_string()))
    }

    /// The class of the current method when `expr` is its first parameter (`self`, typed by ty as
    /// a `Self` type variable).
    fn self_class(&self, expr: &Expr) -> Option<Key> {
        let Expr::Name(name) = expr else { return None };
        let entry = self.index.functions.get(&self.current)?;
        (entry.params.first()? == name.id.as_str()).then_some((self.index.file, entry.class?))
    }

    /// `self.name = value` in a method: records callables stored on instances of the class.
    fn store(&mut self, target: &Expr, value: &Expr) {
        let Expr::Attribute(attr) = target else { return };
        let Some(class) = self.self_class(&attr.value) else { return };
        if let Some(value) = self.value_of(value) {
            self.out.stores.push(RawStore { class, name: attr.attr.to_string(), value });
        }
    }

    /// What builtins do with their arguments: iterate them, call them.
    fn builtin_call(&mut self, name: &str, call: &ast::ExprCall) {
        let args = &call.arguments.args;
        match name {
            "map" | "filter" | "reduce" => {
                if let Some(f) = args.first() {
                    self.call_value(call, f);
                }
                if name == "reduce"
                    && let Some(xs) = args.get(1)
                {
                    self.iterate(xs);
                }
                return;
            }
            "sorted" | "min" | "max" | "sort" => {
                for keyword in &call.arguments.keywords {
                    if keyword.arg.as_ref().is_some_and(|a| a.as_str() == "key") {
                        self.call_value(call, &keyword.value);
                    }
                }
            }
            _ => {}
        }
        if CONSUMERS.contains(&name) {
            for argument in args.iter().take_while(|a| !a.is_starred_expr()) {
                self.iterate(argument);
            }
        }
    }

    /// The callable an argument evaluates to, if any.
    fn value_of(&self, expr: &Expr) -> Option<RawValue> {
        if let Expr::Lambda(lambda) = expr {
            return Some(RawValue::Function((self.index.file, lambda.start().to_u32())));
        }
        if let Some((owner, name)) = self.param_of(expr) {
            return Some(RawValue::Param(owner, name));
        }
        if let Expr::Call(call) = expr
            && self.index.text(call.func.range()).ends_with("partial")
        {
            return self.value_of(call.arguments.args.first()?);
        }
        let db = self.model.db();
        let env = self.model.program_environment();
        match expr.inferred_type(self.model)? {
            ty @ (Type::FunctionLiteral(_) | Type::BoundMethod(_)) => match ty.definition(db, &env)? {
                TypeDefinition::Function(def) => self.function_key(def).map(RawValue::Function),
                _ => None,
            },
            ty @ Type::ClassLiteral(_) => match ty.definition(db, &env)? {
                TypeDefinition::StaticClass(def) => self.def_key(def).map(|(k, _)| RawValue::Class(k)),
                _ => None,
            },
            Type::NominalInstance(_) => {
                let call = self.synthesized_member(expr, "__call__");
                self.resolved_function_keys(definitions_for_attribute(self.model, &call))
                    .into_iter()
                    .next()
                    .map(RawValue::Function)
            }
            Type::Dynamic(_) | Type::Callable(_) | Type::Union(_) => {
                let keys = match expr {
                    Expr::Name(name) => self.resolved_function_keys(definitions_for_name(
                        self.model,
                        &name.id,
                        expr.into(),
                        ImportAliasResolution::ResolveAliases,
                    )),
                    Expr::Attribute(attr) => self.resolved_function_keys(definitions_for_attribute(self.model, attr)),
                    _ => Vec::new(),
                };
                keys.into_iter().next().map(RawValue::Function)
            }
            _ => None,
        }
    }

    /// `<receiver>.<name>` for looking up implicit dunder calls through ty's IDE queries.
    fn synthesized_member(&self, receiver: &Expr, name: &str) -> ast::ExprAttribute {
        ast::ExprAttribute {
            node_index: AtomicNodeIndex::NONE,
            range: receiver.range(),
            value: Box::new(receiver.clone()),
            attr: ast::Identifier::new(name, receiver.range()),
            ctx: ExprContext::Load,
        }
    }

    fn implicit_call(&mut self, receiver: &Expr, dunder: &str, text: String, awaited: bool, iterates: bool) {
        let member = self.synthesized_member(receiver, dunder);
        let keys = self.resolved_function_keys(definitions_for_attribute(self.model, &member));
        if keys.is_empty() {
            return;
        }
        self.out.calls.push(RawCall {
            caller: self.current,
            location: self.index.location(receiver.start()),
            text,
            targets: keys.into_iter().map(|k| (k, true)).collect(),
            virtual_dispatch: true,
            param: None,
            awaited,
            iterates,
            constructs: None,
            unresolved: None,
            accessor: None,
            attribute: None,
        });
    }

    fn handle_call(&mut self, call: &ast::ExprCall) {
        let resolution = self.resolve(&call.func, Some(call));
        let param = self.param_of(&call.func);
        let location = self.index.location(call.start());
        let literal = first_string_literal(call);
        let arguments = call
            .arguments
            .args
            .iter()
            .take_while(|a| !a.is_starred_expr())
            .enumerate()
            .map(|(i, a)| (RawSlot::Positional(i), a))
            .chain(
                call.arguments
                    .keywords
                    .iter()
                    .filter_map(|k| Some((RawSlot::Keyword(k.arg.as_ref()?.to_string()), &k.value))),
            );
        for (slot, argument) in arguments {
            let Some(value) = self.value_of(argument) else { continue };
            for &(into, bound) in &resolution.targets {
                self.out.flows.push(RawFlow {
                    caller: self.current,
                    location: location.clone(),
                    into,
                    bound,
                    slot: slot.clone(),
                    value: value.clone(),
                    literal: literal.clone(),
                });
            }
        }
        if let Some(name) = self.builtin_name(&call.func, &resolution) {
            self.builtin_call(name, call);
        }
        let attribute = if resolution.targets.is_empty() { self.attribute_slot(&call.func) } else { None };
        let unresolved =
            (resolution.targets.is_empty() && param.is_none()).then_some(resolution.reason);
        self.out.calls.push(RawCall {
            caller: self.current,
            location,
            text: self.text(call.func.range()),
            targets: resolution.targets,
            virtual_dispatch: resolution.virtual_dispatch,
            param,
            awaited: self.awaited.contains(&call.start().to_u32()),
            iterates: self.iterated.contains(&call.start().to_u32()),
            constructs: resolution.constructs,
            unresolved,
            accessor: None,
            attribute,
        });
    }

    fn handle_decorators(&mut self, def: &ast::StmtFunctionDef) {
        let key = def.name.start().to_u32();
        let mut decorators = Vec::new();
        for decorator in &def.decorator_list {
            let (callee, call) = match &decorator.expression {
                Expr::Call(call) => (&*call.func, Some(call)),
                expr => (expr, None),
            };
            let resolution = self.resolve(callee, call);
            let literal = call.and_then(first_string_literal);
            for &(into, bound) in &resolution.targets {
                decorators.push(into);
                self.out.flows.push(RawFlow {
                    caller: self.current,
                    location: self.index.location(decorator.expression.start()),
                    into,
                    bound,
                    slot: RawSlot::Decorated,
                    value: RawValue::Function((self.index.file, key)),
                    literal: literal.clone(),
                });
            }
        }
        self.out.decorators.push((key, decorators));
    }

    fn in_scope(&mut self, key: u32, f: impl FnOnce(&mut Self)) {
        let previous = std::mem::replace(&mut self.current, key);
        f(self);
        self.current = previous;
    }
}

impl<'ast> SourceOrderVisitor<'ast> for Extractor<'_, '_> {
    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        match stmt {
            Stmt::FunctionDef(def) => {
                let key = def.name.start().to_u32();
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                if self.recording() || self.wanted.is_some_and(|w| w.contains(&key)) {
                    self.handle_decorators(def);
                }
                if let Some(type_params) = &def.type_params {
                    self.visit_type_params(type_params);
                }
                self.visit_parameters(&def.parameters);
                if let Some(returns) = &def.returns {
                    self.visit_annotation(returns);
                }
                self.in_scope(key, |this| this.visit_body(&def.body));
            }
            Stmt::Assign(assign) if self.recording() => {
                for target in &assign.targets {
                    self.store(target, &assign.value);
                }
                walk_stmt(self, stmt);
            }
            Stmt::AnnAssign(assign) if self.recording() => {
                if let Some(value) = &assign.value {
                    self.store(&assign.target, value);
                }
                walk_stmt(self, stmt);
            }
            Stmt::With(with) if self.recording() => {
                let (enter, exit) = if with.is_async { ("__aenter__", "__aexit__") } else { ("__enter__", "__exit__") };
                for item in &with.items {
                    let text = format!("{}with {}", if with.is_async { "async " } else { "" }, self.text(item.context_expr.range()));
                    self.implicit_call(&item.context_expr, enter, text.clone(), with.is_async, false);
                    self.implicit_call(&item.context_expr, exit, text, with.is_async, false);
                }
                walk_stmt(self, stmt);
            }
            Stmt::For(for_stmt) if self.recording() => {
                if for_stmt.is_async {
                    let text = format!("async for ... in {}", self.text(for_stmt.iter.range()));
                    self.implicit_call(&for_stmt.iter, "__aiter__", text, false, false);
                } else {
                    self.iterate(&for_stmt.iter);
                }
                walk_stmt(self, stmt);
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::Await(await_expr) => {
                if let Expr::Call(call) = &*await_expr.value {
                    self.awaited.insert(call.start().to_u32());
                }
                walk_expr(self, expr);
            }
            Expr::Lambda(lambda) => {
                if let Some(parameters) = &lambda.parameters {
                    self.visit_parameters(parameters);
                }
                self.in_scope(lambda.start().to_u32(), |this| this.visit_expr(&lambda.body));
            }
            Expr::ListComp(ast::ExprListComp { generators, .. })
            | Expr::SetComp(ast::ExprSetComp { generators, .. })
            | Expr::Generator(ast::ExprGenerator { generators, .. })
                if self.recording() =>
            {
                for generator in generators.iter().filter(|g| !g.is_async) {
                    self.iterate(&generator.iter);
                }
                walk_expr(self, expr);
            }
            Expr::DictComp(ast::ExprDictComp { generators, .. }) if self.recording() => {
                for generator in generators.iter().filter(|g| !g.is_async) {
                    self.iterate(&generator.iter);
                }
                walk_expr(self, expr);
            }
            Expr::YieldFrom(yield_from) if self.recording() => {
                self.iterate(&yield_from.value);
                walk_expr(self, expr);
            }
            Expr::Call(call) => {
                self.callees.insert((call.func.start().to_u32(), call.func.end().to_u32()));
                if self.recording() {
                    self.handle_call(call);
                }
                walk_expr(self, expr);
            }
            Expr::Attribute(attr)
                if self.recording()
                    && self.properties.is_none_or(|p| p.contains(attr.attr.as_str()))
                    && !self.callees.contains(&(attr.start().to_u32(), attr.end().to_u32())) =>
            {
                let accessor = match attr.ctx {
                    ExprContext::Store => Accessor::Set,
                    ExprContext::Del => Accessor::Delete,
                    _ => Accessor::Get,
                };
                let keys = self.resolved_function_keys(definitions_for_attribute(self.model, attr));
                if !keys.is_empty() {
                    self.out.calls.push(RawCall {
                        caller: self.current,
                        location: self.index.location(attr.start()),
                        text: self.text(attr.range()),
                        targets: keys.into_iter().map(|k| (k, true)).collect(),
                        virtual_dispatch: true,
                        param: None,
                        awaited: false,
                        iterates: false,
                        constructs: None,
                        unresolved: None,
                        accessor: Some(accessor),
                        attribute: None,
                    });
                }
                walk_expr(self, expr);
            }
            _ => walk_expr(self, expr),
        }
    }
}
