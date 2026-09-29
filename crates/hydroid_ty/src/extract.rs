//! Walks function bodies and records calls (with their resolved callees) and callable flows.

use hydroid_core::facts::Location;
use ruff_db::files::File;
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
    pub unresolved: Option<String>,
    /// An attribute read; only kept if a target turns out to be a property.
    pub property_access: bool,
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
    pub decorators: Vec<(u32, Vec<Key>)>,
}

/// Extracts the bodies of `wanted` functions (every function and the module body when `None`).
pub fn extract_file(db: &HydroidDb, index: &FileIndex, wanted: Option<&FxHashSet<u32>>) -> FileFacts {
    let program_file = ty_python_semantic::Db::program_file(db, index.file);
    let model = SemanticModel::new(db, program_file);
    let parsed = parsed_module(db, model.python_file()).load(db);
    let mut extractor = Extractor {
        model: &model,
        index,
        wanted,
        current: MODULE_KEY,
        awaited: FxHashSet::default(),
        callees: FxHashSet::default(),
        out: FileFacts { file: index.file, calls: Vec::new(), flows: Vec::new(), decorators: Vec::new() },
    };
    extractor.visit_body(&parsed.syntax().body);
    extractor.out
}

struct Resolution {
    targets: Vec<(Key, bool)>,
    virtual_dispatch: bool,
    reason: String,
}

struct Extractor<'a, 'db> {
    model: &'a SemanticModel<'db>,
    index: &'a FileIndex,
    wanted: Option<&'a FxHashSet<u32>>,
    current: u32,
    awaited: FxHashSet<u32>,
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
            _ => return None,
        };
        let module = parsed_module(db, def.python_file(db)).load(db);
        let range = def.focus_range(db, &module);
        Some(((range.file(), range.start().to_u32()), is_function))
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
        let mut resolution = Resolution { targets: Vec::new(), virtual_dispatch: false, reason: String::new() };
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
        if resolution.targets.is_empty() {
            // Decorated functions often lose their identity in the type; go-to-definition keeps it.
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
            resolution.targets = keys.into_iter().map(|k| (k, bound)).collect();
        }
        resolution
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

    fn implicit_call(&mut self, receiver: &Expr, dunder: &str, text: String, awaited: bool) {
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
            unresolved: None,
            property_access: false,
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
            unresolved,
            property_access: false,
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
            Stmt::With(with) if self.recording() => {
                let (enter, exit) = if with.is_async { ("__aenter__", "__aexit__") } else { ("__enter__", "__exit__") };
                for item in &with.items {
                    let text = format!("{}with {}", if with.is_async { "async " } else { "" }, self.text(item.context_expr.range()));
                    self.implicit_call(&item.context_expr, enter, text.clone(), with.is_async);
                    self.implicit_call(&item.context_expr, exit, text, with.is_async);
                }
                walk_stmt(self, stmt);
            }
            Stmt::For(for_stmt) if self.recording() => {
                let dunder = if for_stmt.is_async { "__aiter__" } else { "__iter__" };
                let text = format!("for ... in {}", self.text(for_stmt.iter.range()));
                self.implicit_call(&for_stmt.iter, dunder, text, false);
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
            Expr::Call(call) => {
                self.callees.insert((call.func.start().to_u32(), call.func.end().to_u32()));
                if self.recording() {
                    self.handle_call(call);
                }
                walk_expr(self, expr);
            }
            Expr::Attribute(attr)
                if attr.ctx == ExprContext::Load
                    && self.recording()
                    && !self.callees.contains(&(attr.start().to_u32(), attr.end().to_u32())) =>
            {
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
                        unresolved: None,
                        property_access: true,
                    });
                }
                walk_expr(self, expr);
            }
            _ => walk_expr(self, expr),
        }
    }
}
