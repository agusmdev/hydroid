//! Walks function bodies and records calls (with their resolved callees) and callable flows.

use hydroid_core::facts::{Accessor, Dump, Location, LoggerName, Origin};
use ruff_db::files::{File, FilePath};
use ruff_db::parsed::parsed_module;
use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, walk_expr, walk_stmt};
use ruff_python_ast::{self as ast, AtomicNodeIndex, Expr, ExprContext, Stmt};
use ruff_text_size::{Ranged, TextRange};
use std::cell::RefCell;

use rustc_hash::{FxHashMap, FxHashSet};
use ty_python_core::definition::{Definition, DefinitionKind, ParameterDefinitionNodeKind};
use ty_python_semantic::types::ide_support::{ResolvedDefinition, call_signature_details};
use ty_python_semantic::types::{Type, TypeDefinition};
use ty_python_semantic::{
    HasType, ImportAliasResolution, SemanticModel, definitions_for_attribute, definitions_for_name,
};

use crate::db::HydroidDb;
use crate::index::{
    FileIndex, Key, MODULE_KEY, StateOwner, annotation_classes, describe, inherits_from, instance_class, is_class_object,
    json_only, state_owner, yield_values, yields,
};

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
    /// Implicit (`with`, `for`, operators): dropped when nothing it may run is known.
    pub implicit: bool,
    /// See [`hydroid_core::facts::Call::attribute`].
    pub attribute: Option<(Key, String)>,
    pub returned_by: Vec<Key>,
    /// See [`hydroid_core::facts::Call::sets`].
    pub sets: Option<(Key, Vec<String>)>,
    /// See [`hydroid_core::facts::Call::given`].
    pub given: Option<Vec<String>>,
    /// See [`hydroid_core::facts::Call::unpacks`].
    pub unpacks: bool,
    /// See [`hydroid_core::facts::Call::instances`].
    pub instances: Vec<String>,
    /// See [`hydroid_core::facts::Call::dump`].
    pub dump: Option<Dump>,
    /// See [`hydroid_core::facts::Call::level`]: `(value, receiver class of a `setLevel`)`.
    pub level: Option<(Option<u32>, Option<Key>)>,
    /// See [`hydroid_core::facts::Call::receiver`].
    pub receiver: Option<Key>,
    /// See [`hydroid_core::facts::Call::super_name`].
    pub super_name: Option<String>,
    /// See [`hydroid_core::facts::Call::logging`]: classes as keys.
    pub logging: Option<(Option<LoggerName>, Option<Key>, bool)>,
}

/// What code can run implicitly, from the definitions indexed so far.
#[derive(Default)]
pub struct Implicit {
    /// Names of properties, of dunder methods, and of class attributes that may hold a
    /// descriptor: attribute accesses and operators with other names run nothing.
    pub names: FxHashSet<String>,
    /// Classes defining (or inheriting) `__getattr__`.
    pub getattr_classes: FxHashSet<Key>,
}

/// What the project keeps on application and connection state (Starlette types `request.app`
/// and every `state` attribute as `Any`).
#[derive(Default)]
pub struct AppState {
    /// The class of what is stored under each name (`None`: the stores disagree, or one stores
    /// a value of unknown class).
    pub stored: FxHashMap<(StateOwner, String), Option<Key>>,
    /// The class of `request.app`: the one `Starlette` subclass the project defines, if any.
    pub app: Option<Key>,
}

/// What the project's SQLAlchemy models run implicitly on attribute access.
#[derive(Default)]
pub struct Orm {
    /// Attributes some `@validates` validator names.
    pub validated: FxHashSet<String>,
    /// `(mapped class, name)`: relationships loaded lazily on first read, inherited ones included.
    pub lazy: FxHashSet<(Key, String)>,
    pub lazy_names: FxHashSet<String>,
    /// Relationships each function names in eager loader options (see
    /// [`crate::index::FileIndex::eager_loads`]).
    pub eager: FxHashMap<Key, Vec<String>>,
    /// `Session.execute`, which a lazy load runs.
    pub load: Option<Key>,
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
    /// See [`hydroid_core::facts::Flow::receiver`].
    pub receiver: Option<Key>,
    pub literal: Option<String>,
}

/// See [`hydroid_core::facts::Signature`].
pub struct RawSignature {
    pub function: u32,
    pub params: Vec<RawValue>,
    pub annotation: Vec<Key>,
    pub returned: Vec<Key>,
    pub returns_unknown: bool,
    pub returns_response: bool,
}

/// See [`hydroid_core::facts::ModelField`].
#[derive(Default)]
pub struct RawField {
    pub names: Vec<String>,
    pub classes: Vec<Key>,
    pub validators: Vec<RawValue>,
    pub serializers: Vec<RawValue>,
    /// The `serializers` only used when dumping to JSON.
    pub json_serializers: Vec<RawValue>,
}

/// See [`hydroid_core::facts::ResponseOptions`].
pub struct RawResponseOptions {
    pub caller: u32,
    pub location: Location,
    pub no_model: bool,
    pub exclude: Vec<String>,
}

pub struct FileFacts {
    pub file: File,
    pub signatures: Vec<RawSignature>,
    pub calls: Vec<RawCall>,
    pub flows: Vec<RawFlow>,
    pub stores: Vec<RawStore>,
    pub returns: Vec<(u32, RawValue)>,
    /// `(dispatcher, implementation defined in this file)`.
    pub dispatches: Vec<(Key, u32)>,
    pub decorators: Vec<(u32, Vec<Key>)>,
    /// `(class defined in this file, names the field can be given by, factory)`, see
    /// [`hydroid_core::facts::Class::default_factories`].
    pub default_factories: Vec<(u32, Vec<String>, RawValue)>,
    /// `(class defined in this file, field)`.
    pub model_fields: Vec<(u32, RawField)>,
    pub response_options: Vec<RawResponseOptions>,
    /// See [`hydroid_core::facts::Facts::stops_propagation`].
    pub stops_propagation: Vec<(String, bool)>,
}

/// Extracts the bodies of `wanted` functions (every function and the module body when `None`).
pub fn extract_file(
    db: &HydroidDb,
    index: &FileIndex,
    wanted: Option<&FxHashSet<u32>>,
    implicit: Option<&Implicit>,
    state: &AppState,
    orm: &Orm,
) -> FileFacts {
    let program_file = ty_python_semantic::Db::program_file(db, index.file);
    let model = SemanticModel::new(db, program_file);
    let parsed = parsed_module(db, model.python_file()).load(db);
    let mut extractor = Extractor {
        db,
        state,
        orm,
        lazy_reads: Vec::new(),
        model: &model,
        index,
        wanted,
        implicit,
        current: MODULE_KEY,
        awaited: FxHashSet::default(),
        iterated: FxHashSet::default(),
        returned_classes: RefCell::default(),
        callees: FxHashSet::default(),
        attach_only: if ATTACHES.iter().any(|m| index.text(parsed.syntax().range()).contains(m)) {
            attach_only(&parsed.syntax().body)
        } else {
            FxHashSet::default()
        },
        out: FileFacts {
            file: index.file,
            signatures: Vec::new(),
            calls: Vec::new(),
            flows: Vec::new(),
            stores: Vec::new(),
            returns: Vec::new(),
            dispatches: Vec::new(),
            decorators: Vec::new(),
            default_factories: Vec::new(),
            model_fields: Vec::new(),
            response_options: Vec::new(),
            stops_propagation: Vec::new(),
        },
    };
    extractor.visit_body(&parsed.syntax().body);
    extractor.drop_eager_reads();
    extractor.out
}

struct Resolution {
    targets: Vec<(Key, bool)>,
    virtual_dispatch: bool,
    constructs: Option<Key>,
    reason: String,
}

/// Whether `expr` is an instance of a project class (or `None`).
fn is_project_instance(db: &HydroidDb, model: &SemanticModel<'_>, expr: &Expr) -> bool {
    instance_class(model, expr).is_some_and(|(file, _)| describe(db, file).1 == Origin::Project)
}

/// Whether an annotation declares instances of project classes, or a collection of them
/// (`list[Attachment]`, `dict[str, Attachment] | None`).
fn declares_instances(db: &HydroidDb, model: &SemanticModel<'_>, annotation: &Expr) -> bool {
    match annotation {
        Expr::NoneLiteral(_) => true,
        Expr::BinOp(ast::ExprBinOp { left, op: ast::Operator::BitOr, right, .. }) => {
            declares_instances(db, model, left) && declares_instances(db, model, right)
        }
        Expr::Subscript(subscript) => {
            let container = match &*subscript.value {
                Expr::Name(n) => n.id.as_str(),
                Expr::Attribute(a) => a.attr.as_str(),
                _ => return false,
            };
            let elements = match &*subscript.slice {
                Expr::Tuple(tuple) => &tuple.elts[..],
                slice => std::slice::from_ref(slice),
            };
            match container {
                "list" | "List" | "set" | "Set" | "frozenset" | "FrozenSet" | "Sequence" | "Optional" | "Union" => {
                    elements.iter().all(|e| declares_instances(db, model, e))
                }
                "tuple" | "Tuple" => {
                    elements.iter().all(|e| e.is_ellipsis_literal_expr() || declares_instances(db, model, e))
                }
                "dict" | "Dict" | "Mapping" => elements.last().is_some_and(|e| declares_instances(db, model, e)),
                _ => false,
            }
        }
        Expr::Name(_) | Expr::Attribute(_) => is_project_instance(db, model, annotation),
        _ => false,
    }
}

/// The top-level fields a keyword argument of a call leaves out: a literal set of field names, or
/// the names a literal dict maps to `True` (or `...`).
fn excluded_fields(call: &ast::ExprCall, keyword: &str) -> Vec<String> {
    let mut exclude = Vec::new();
    match call.arguments.find_keyword(keyword).map(|k| &k.value) {
        Some(Expr::Set(set)) => {
            exclude.extend(set.elts.iter().filter_map(|e| Some(e.as_string_literal_expr()?.value.to_str().to_string())));
        }
        Some(Expr::Dict(dict)) => {
            for item in &dict.items {
                if matches!(item.value, Expr::BooleanLiteral(ast::ExprBooleanLiteral { value: true, .. }) | Expr::EllipsisLiteral(_))
                    && let Some(Expr::StringLiteral(key)) = &item.key
                {
                    exclude.push(key.value.to_str().to_string());
                }
            }
        }
        _ => {}
    }
    exclude
}

/// Builtins that iterate their positional arguments during the call.
const CONSUMERS: &[&str] = &[
    "list", "tuple", "set", "frozenset", "dict", "sorted", "sum", "min", "max", "any", "all", "next", "iter", "join",
    "extend", "update", "reduce",
];

/// Builtins returning lazy iterators over their arguments: iterating one iterates those.
const LAZY: &[&str] = &["map", "filter", "zip", "enumerate", "reversed", "iter"];

/// `Annotated` metadata attaching a validator callable to a field or parameter (pydantic).
const VALIDATOR_METADATA: &[&str] = &["AfterValidator", "BeforeValidator", "PlainValidator", "WrapValidator"];

/// `Annotated` metadata attaching a serializer callable to a field (pydantic).
const SERIALIZER_METADATA: &[&str] = &["PlainSerializer", "WrapSerializer"];

/// Parameter markers whose value FastAPI gets by calling a dependency, not from the request.
const DEPENDENCY_MARKERS: &[&str] = &["Depends", "Security"];

/// Pydantic classmethods that validate input into the model (running its validators).
const VALIDATES: &[&str] =
    &["model_validate", "model_validate_json", "model_validate_strings", "parse_obj", "parse_raw"];

/// Logger methods logging a record.
const LOGS: &[&str] = &["debug", "info", "warning", "warn", "error", "exception", "critical", "fatal", "log"];

/// Methods attaching a handler, filter or formatter to a logger or handler.
const ATTACHES: &[&str] = &["addHandler", "addFilter", "setFormatter"];

struct Extractor<'a, 'db> {
    db: &'a HydroidDb,
    state: &'a AppState,
    orm: &'a Orm,
    /// Lazy relationship reads recorded: `(function, location, relationship)`.
    lazy_reads: Vec<(u32, Location, String)>,
    model: &'a SemanticModel<'db>,
    index: &'a FileIndex,
    wanted: Option<&'a FxHashSet<u32>>,
    /// `None`: anything may run code (`--follow-libs`, where library definitions count too).
    implicit: Option<&'a Implicit>,
    current: u32,
    awaited: FxHashSet<u32>,
    /// Calls whose result is iterated right away (see [`Extractor::iterate`]).
    iterated: FxHashSet<u32>,
    /// See [`Extractor::value_classes`].
    returned_classes: RefCell<FxHashMap<(Key, bool), Vec<Key>>>,
    /// Ranges of callee expressions (attribute reads there are calls, not property reads).
    callees: FxHashSet<(u32, u32)>,
    /// Constructions (by offset) whose instance is only attached, see [`attach_only`].
    attach_only: FxHashSet<u32>,
    out: FileFacts,
}

/// `Depends` in `Depends(dep)` and `fastapi.Depends(dep)`.
fn called_name(call: &ast::ExprCall) -> Option<&str> {
    match &*call.func {
        Expr::Name(name) => Some(name.id.as_str()),
        Expr::Attribute(attr) => Some(attr.attr.as_str()),
        _ => None,
    }
}

/// The elements of a dict (values), list or tuple literal that `container[key]` may be: the one
/// under `key` when both it and the container's keys are literals, else all of them.
fn selected_elements<'e>(container: &'e Expr, key: &Expr) -> Vec<&'e Expr> {
    match container {
        Expr::Dict(dict) => {
            let Expr::StringLiteral(key) = key else { return dict.items.iter().map(|i| &i.value).collect() };
            let mut selected = Vec::new();
            for item in &dict.items {
                match &item.key {
                    Some(Expr::StringLiteral(k)) if k.value.to_str() != key.value.to_str() => {}
                    // A matching key, or one (or a `**` spread) that might be equal.
                    _ => selected.push(&item.value),
                }
            }
            selected
        }
        Expr::List(ast::ExprList { elts, .. }) | Expr::Tuple(ast::ExprTuple { elts, .. }) => {
            let index = match key {
                Expr::NumberLiteral(ast::ExprNumberLiteral { value: ast::Number::Int(i), .. }) => i.as_usize(),
                _ => None,
            };
            match index {
                Some(i) if !elts.iter().any(Expr::is_starred_expr) => elts.get(i).into_iter().collect(),
                _ => elts.iter().filter(|e| !e.is_starred_expr()).collect(),
            }
        }
        _ => Vec::new(),
    }
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
            DefinitionKind::Assignment(assignment) => {
                let module = parsed_module(db, def.python_file(db)).load(db);
                return self.alias_key(def, assignment.value(&module));
            }
            _ => return None,
        };
        let module = parsed_module(db, def.python_file(db)).load(db);
        let range = def.focus_range(db, &module);
        Some(((range.file(), range.start().to_u32()), is_function))
    }

    /// `__enter__ = acquire` in a class body, `load = lambda: ...`: the function a name is bound to.
    fn alias_key(&self, def: Definition<'db>, value: &Expr) -> Option<(Key, bool)> {
        let db = self.model.db();
        if let Expr::Lambda(lambda) = value {
            return Some(((def.file(db), lambda.start().to_u32()), true));
        }
        let Expr::Name(name) = value else { return None };
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

    /// The class whose MRO the method lookup of `obj.name(...)` follows (see
    /// [`hydroid_core::facts::Call::receiver`]): `obj`'s class, or for `super().name(...)` the
    /// class of the calling method (with the name).
    fn mro_receiver(&self, callee: &Expr, resolution: &Resolution) -> (Option<Key>, Option<String>) {
        // Calling an instance (`scheme(request)`, `self.scheme(request)`) looks `__call__` up
        // in its class.
        if !resolution.targets.is_empty() && matches!(callee.inferred_type(self.model), Some(Type::NominalInstance(_))) {
            return (instance_class(self.model, callee), None);
        }
        let Expr::Attribute(attr) = callee else { return (None, None) };
        if Self::is_super_call(&attr.value) {
            let class = self.index.functions.get(&self.current).and_then(|f| f.class);
            return (class.map(|c| (self.index.file, c)), Some(attr.attr.to_string()));
        }
        if !resolution.targets.iter().any(|(_, bound)| *bound) {
            return (None, None);
        }
        (self.self_class(&attr.value).or_else(|| instance_class(self.model, &attr.value)), None)
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
        // Dispatch tables: ty widens the values of a dict (list, tuple) literal, the literal
        // itself still says which callable sits under which key.
        if resolution.targets.is_empty() || matches!(callee, Expr::Subscript(_)) {
            let stored = self.container_targets(self.model, callee, 0);
            if !stored.is_empty() {
                resolution.targets = stored;
                resolution.virtual_dispatch = false;
            }
        }
        resolution
    }

    /// `TABLE[key]`, or a variable assigned it (`send = TABLE[key]`), where `TABLE` is assigned a
    /// dict, list or tuple literal: the functions stored in it (`(function, receiver bound)`),
    /// only the entry under `key` when the key is a literal the container spells out.
    fn container_targets(&self, model: &SemanticModel<'db>, expr: &Expr, depth: u8) -> Vec<(Key, bool)> {
        let mut targets = Vec::new();
        if depth > 2 {
            return targets;
        }
        match expr {
            Expr::Subscript(subscript) => self.assigned_values(model, &subscript.value, &mut |model, container| {
                for element in selected_elements(container, &subscript.slice) {
                    if let Some(target) = self.stored_function(model, element)
                        && !targets.contains(&target)
                    {
                        targets.push(target);
                    }
                }
            }),
            Expr::Name(_) => self.assigned_values(model, expr, &mut |model, value| {
                if matches!(value, Expr::Subscript(_)) {
                    for target in self.container_targets(model, value, depth + 1) {
                        if !targets.contains(&target) {
                            targets.push(target);
                        }
                    }
                }
            }),
            _ => {}
        }
        targets
    }

    /// Calls `f` with the value of each plain assignment (`x = value`, `x: T = value`) defining
    /// the name or attribute `expr`, and the semantic model of the file the value is in.
    fn assigned_values(
        &self,
        model: &SemanticModel<'db>,
        expr: &Expr,
        f: &mut dyn FnMut(&SemanticModel<'db>, &Expr),
    ) {
        let db = model.db();
        let definitions = match expr {
            Expr::Name(name) => {
                definitions_for_name(model, &name.id, expr.into(), ImportAliasResolution::ResolveAliases)
            }
            Expr::Attribute(attr) => definitions_for_attribute(model, attr),
            _ => return,
        };
        for definition in definitions {
            let ResolvedDefinition::Definition(def) = definition else { continue };
            let module = parsed_module(db, def.python_file(db)).load(db);
            let value = match def.kind(db) {
                DefinitionKind::Assignment(a) if a.unpack().is_none() => Some(a.value(&module)),
                DefinitionKind::AnnotatedAssignment(a) => a.value(&module),
                _ => None,
            };
            if let Some(value) = value {
                let model = SemanticModel::new(db, ty_python_semantic::Db::program_file(db, def.file(db)));
                f(&model, value);
            }
        }
    }

    /// The function a container element is (`handler`, `session.get`, a lambda).
    fn stored_function(&self, model: &SemanticModel<'db>, element: &Expr) -> Option<(Key, bool)> {
        let db = model.db();
        if let Expr::Lambda(lambda) = element {
            return Some(((model.file(), lambda.start().to_u32()), false));
        }
        let ty = element.inferred_type(model)?;
        match ty {
            Type::FunctionLiteral(_) | Type::BoundMethod(_) => {
                match ty.definition(db, &model.program_environment())? {
                    TypeDefinition::Function(def) => {
                        self.function_key(def).map(|key| (key, matches!(ty, Type::BoundMethod(_))))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn class_key(&self, ty: &Type<'db>) -> Option<Key> {
        match ty.definition(self.model.db(), &self.model.program_environment())? {
            TypeDefinition::StaticClass(def) => self.def_key(def).map(|(k, _)| k),
            _ => None,
        }
    }

    /// The name of a builtin (or a `functools`, `contextlib` or `contextvars` function) the
    /// resolved call targets.
    fn builtin_name<'e>(&self, func: &'e Expr, resolution: &Resolution) -> Option<&'e str> {
        let db = self.model.db();
        let is_builtin = |file: File| match file.path(db) {
            FilePath::Vendored(path) => {
                matches!(
                path.as_str(),
                "stdlib/builtins.pyi" | "stdlib/functools.pyi" | "stdlib/contextlib.pyi" | "stdlib/_contextvars.pyi"
            )
            }
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
        self.implicit_call(expr, "__iter__", text.clone(), false, true);
        // Iterators returning themselves from `__iter__` do the work in `__next__`.
        if self.operator_may_run(expr, "__next__") {
            self.implicit_call(expr, "__next__", text, false, true);
        }
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
            implicit: false,
            attribute: None,
            returned_by: Vec::new(),
            sets: None,
            given: None,
            unpacks: false,
            instances: Vec::new(),
            dump: None,
            level: None,
            receiver: None,
            super_name: None,
            logging: None,
        });
    }

    fn may_run(&self, name: &str) -> bool {
        self.implicit.is_none_or(|implicit| implicit.names.contains(name))
    }

    /// Whether reading an attribute `attr` does not define may run `__getattr__`.
    fn may_run_getattr(&self, attr: &ast::ExprAttribute) -> bool {
        let Some(implicit) = self.implicit else { return true };
        if implicit.getattr_classes.is_empty() {
            return false;
        }
        let Some(ty @ Type::NominalInstance(_)) = attr.value.inferred_type(self.model) else { return false };
        self.class_key(&ty).is_some_and(|class| implicit.getattr_classes.contains(&class))
    }

    /// Whether an operator on `operand` may call a user-defined `dunder`: the dunder is defined
    /// somewhere, and the operand is an instance of a class outside the standard library.
    fn operator_may_run(&self, operand: &Expr, dunder: &str) -> bool {
        if !self.may_run(dunder) {
            return false;
        }
        let Some(ty @ Type::NominalInstance(_)) = operand.inferred_type(self.model) else { return false };
        self.class_key(&ty).is_some_and(|(file, _)| !file.path(self.model.db()).is_vendored_path())
    }

    /// The functions whose return value `callee` is: `make()` in `make()()`, or the call a local
    /// variable was assigned from (`nap = make(); nap()`).
    fn returned_by(&self, callee: &Expr) -> Vec<Key> {
        let call = match callee {
            Expr::Call(call) => call,
            Expr::Name(name) => {
                let db = self.model.db();
                let definitions =
                    definitions_for_name(self.model, &name.id, callee.into(), ImportAliasResolution::ResolveAliases);
                let assigned = definitions.into_iter().find_map(|d| match d {
                    ResolvedDefinition::Definition(def) if def.file(db) == self.index.file => match def.kind(db) {
                        DefinitionKind::Assignment(a) => {
                            Some(a.value(&parsed_module(db, def.python_file(db)).load(db)).range())
                        }
                        _ => None,
                    },
                    _ => None,
                });
                let Some(range) = assigned else { return Vec::new() };
                let parsed = parsed_module(db, self.model.python_file()).load(db);
                let Some(call) = find_call(parsed.syntax(), range) else { return Vec::new() };
                return self.resolve(&call.func, Some(call)).targets.into_iter().map(|(k, _)| k).collect();
            }
            _ => return Vec::new(),
        };
        self.resolve(&call.func, Some(call)).targets.into_iter().map(|(k, _)| k).collect()
    }

    /// `obj.name(...)` where `name` is no method of `obj`'s class: an attribute holding a callable.
    fn attribute_slot(&self, callee: &Expr) -> Option<(Key, String)> {
        let Expr::Attribute(attr) = callee else { return None };
        Some((self.receiver_class(&attr.value)?, attr.attr.to_string()))
    }

    /// The class of an instance: its type, or, when ty does not know it (unannotated returns are
    /// unknown to ty), the class the function it was returned from (or the dependency it was
    /// injected by) constructs.
    fn receiver_class(&self, receiver: &Expr) -> Option<Key> {
        if let Some(class) = self.self_class(receiver) {
            return Some(class);
        }
        match receiver.inferred_type(self.model)? {
            ty @ Type::NominalInstance(_) => self.class_key(&ty),
            Type::Dynamic(_) => {
                if let Some(class) = self.state_class(receiver, 0) {
                    return Some(class);
                }
                if let Some(class) = self.dependency_classes(receiver).first() {
                    return Some(*class);
                }
                self.returned_by(receiver).into_iter().find_map(|f| self.returned_classes(f).first().copied())
            }
            Type::Union(union)
                if union.elements(self.model.db()).iter().all(|t| matches!(t, Type::Dynamic(_))) =>
            {
                self.dependency_classes(receiver).first().copied()
            }
            _ => None,
        }
    }

    /// The class of an object read back from application or connection state:
    /// `request.state.name`, `request.app.state.name`, `request.app` (see [`AppState`]), or a
    /// local variable assigned one in this file.
    fn state_class(&self, expr: &Expr, depth: u8) -> Option<Key> {
        match expr {
            Expr::Attribute(attr) if attr.attr.as_str() == "app" => {
                let owner = state_owner(self.db, self.model, &attr.value)?;
                if owner == StateOwner::Connection { self.state.app } else { None }
            }
            Expr::Attribute(attr) => {
                let Expr::Attribute(state) = &*attr.value else { return None };
                if state.attr.as_str() != "state" {
                    return None;
                }
                let owner = state_owner(self.db, self.model, &state.value)?;
                self.state.stored.get(&(owner, attr.attr.to_string())).copied().flatten()
            }
            Expr::Name(name) if depth < 3 && (!self.state.stored.is_empty() || self.state.app.is_some()) => {
                let db = self.model.db();
                let mut class = None;
                for definition in
                    definitions_for_name(self.model, &name.id, expr.into(), ImportAliasResolution::ResolveAliases)
                {
                    let ResolvedDefinition::Definition(def) = definition else { return None };
                    let DefinitionKind::Assignment(assignment) = def.kind(db) else { return None };
                    if def.file(db) != self.index.file || assignment.unpack().is_some() {
                        return None;
                    }
                    let module = parsed_module(db, def.python_file(db)).load(db);
                    let found = self.state_class(assignment.value(&module), depth + 1)?;
                    if class.is_some_and(|c| c != found) {
                        return None;
                    }
                    class = Some(found);
                }
                class
            }
            _ => None,
        }
    }

    /// Classes of the instances a function returns, from the types of its `return` values.
    fn returned_classes(&self, function: Key) -> Vec<Key> {
        self.value_classes(function, false)
    }

    /// Classes of the instances a function returns (`yielded`: yields), from the types of its
    /// `return` (`yield`) values.
    fn value_classes(&self, function: Key, yielded: bool) -> Vec<Key> {
        if let Some(classes) = self.returned_classes.borrow().get(&(function, yielded)) {
            return classes.clone();
        }
        let db = self.model.db();
        let (file, offset) = function;
        let model = SemanticModel::new(db, ty_python_semantic::Db::program_file(db, file));
        let parsed = parsed_module(db, model.python_file()).load(db);
        let mut classes = Vec::new();
        if let Some(def) = find_function(parsed.syntax(), offset) {
            let values = if yielded { yield_values(&def.body) } else { return_values(&def.body) };
            for value in values {
                let types = match value.inferred_type(&model) {
                    Some(Type::Union(union)) => union.elements(db).to_vec(),
                    Some(ty) => vec![ty],
                    None => Vec::new(),
                };
                for ty in types.iter().filter(|t| matches!(t, Type::NominalInstance(_))) {
                    if let Some(class) = self.class_key(ty)
                        && !classes.contains(&class)
                    {
                        classes.push(class);
                    }
                }
            }
        }
        self.returned_classes.borrow_mut().insert((function, yielded), classes.clone());
        classes
    }

    /// The classes of an unannotated parameter's value when its default is `Depends(provider)`
    /// (or `Security(provider)`): FastAPI passes what the provider returns (yields, for generator
    /// dependencies), or an instance of a class used as a dependency.
    fn dependency_classes(&self, receiver: &Expr) -> Vec<Key> {
        let Some((owner, name)) = self.param_of(receiver) else { return Vec::new() };
        let db = self.model.db();
        let parsed = parsed_module(db, self.model.python_file()).load(db);
        let Some(def) = find_function(parsed.syntax(), owner) else { return Vec::new() };
        let parameters = &def.parameters;
        let Some(parameter) = parameters
            .posonlyargs
            .iter()
            .chain(&parameters.args)
            .chain(&parameters.kwonlyargs)
            .find(|p| p.parameter.name.as_str() == name)
        else {
            return Vec::new();
        };
        let Some(Expr::Call(call)) = parameter.default.as_deref() else { return Vec::new() };
        if parameter.parameter.annotation.is_some() || !called_name(call).is_some_and(|n| DEPENDENCY_MARKERS.contains(&n))
        {
            return Vec::new();
        }
        let provider = call.arguments.args.first().or_else(|| call.arguments.find_keyword("dependency").map(|k| &k.value));
        match provider.and_then(|p| self.value_of(p)) {
            Some(RawValue::Function(function)) => self.provided_classes(function),
            Some(RawValue::Class(class)) => vec![class],
            _ => Vec::new(),
        }
    }

    /// Classes of what a dependency provider gives FastAPI: its declared return type (the yield
    /// type of a generator), else the classes of its `return` (`yield`) values.
    fn provided_classes(&self, function: Key) -> Vec<Key> {
        let db = self.model.db();
        let (file, offset) = function;
        let model = SemanticModel::new(db, ty_python_semantic::Db::program_file(db, file));
        let parsed = parsed_module(db, model.python_file()).load(db);
        let Some(def) = find_function(parsed.syntax(), offset) else { return Vec::new() };
        let generator = yields(&def.body);
        let mut classes = Vec::new();
        if let Some(returns) = def.returns.as_deref() {
            // `Iterator[Client]`, `Generator[Client, None, None]`: the first type argument.
            let returns = match returns {
                Expr::Subscript(subscript) if generator => match &*subscript.slice {
                    Expr::Tuple(tuple) => tuple.elts.first(),
                    slice => Some(slice),
                },
                Expr::Subscript(_) => None,
                returns => Some(returns),
            };
            if let Some(returns) = returns.filter(|r| matches!(r, Expr::Name(_) | Expr::Attribute(_) | Expr::BinOp(_))) {
                annotation_classes(&model, returns, &mut classes, &mut |_| {});
            }
        }
        if classes.is_empty() {
            classes = self.value_classes(function, generator);
        }
        classes
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

    /// Class attributes holding a descriptor: accessing them runs `__get__` (`__set__`,
    /// `__delete__`) of the descriptor's class.
    fn descriptor_calls(
        &mut self,
        attr: &ast::ExprAttribute,
        accessor: Accessor,
        definitions: &[ResolvedDefinition<'db>],
    ) {
        let dunder = match accessor {
            Accessor::Get => "__get__",
            Accessor::Set => "__set__",
            Accessor::Delete => "__delete__",
        };
        if !self.may_run(dunder) {
            return;
        }
        let db = self.model.db();
        for definition in definitions {
            let ResolvedDefinition::Definition(def) = definition else { continue };
            let DefinitionKind::Assignment(assignment) = def.kind(db) else { continue };
            let model = SemanticModel::new(db, ty_python_semantic::Db::program_file(db, def.file(db)));
            let module = parsed_module(db, model.python_file()).load(db);
            let Some(ty @ Type::NominalInstance(_)) = assignment.value(&module).inferred_type(&model) else { continue };
            let Some(class) = self.class_key(&ty) else { continue };
            self.out.calls.push(RawCall {
                caller: self.current,
                location: self.index.location(attr.start()),
                text: self.text(attr.range()),
                targets: Vec::new(),
                virtual_dispatch: false,
                param: None,
                awaited: false,
                iterates: false,
                constructs: None,
                unresolved: None,
                accessor: None,
                implicit: true,
                attribute: Some((class, dunder.to_string())),
                returned_by: Vec::new(),
                sets: None,
                given: None,
                unpacks: false,
                instances: Vec::new(),
                dump: None,
                level: None,
                receiver: None,
                super_name: None,
                logging: None,
            });
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
            // contextlib: `stack.enter_context(cm)` enters `cm`, `closing(x)` closes `x`.
            "enter_context" => {
                if let Some(cm) = args.first() {
                    let text = format!("enter_context({})", self.text(cm.range()));
                    self.implicit_call(cm, "__enter__", text.clone(), false, false);
                    self.implicit_call(cm, "__exit__", text, false, false);
                }
                return;
            }
            "closing" => {
                if let Some(x) = args.first() {
                    let text = format!("closing({})", self.text(x.range()));
                    self.implicit_call(x, "close", text, false, false);
                }
                return;
            }
            // `ExitStack.callback(f)`; `Context.run(f, *args)` calls `f` right away, in this thread.
            "callback" | "run" => {
                if let Some(f) = args.first() {
                    self.call_value(call, f);
                }
                return;
            }
            "len" => {
                if let Some(x) = args.first()
                    && self.operator_may_run(x, "__len__")
                {
                    self.implicit_call(x, "__len__", format!("len({})", self.text(x.range())), false, false);
                }
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

    /// The class of a callable instance (whose value is its `__call__`), when its class is a
    /// project class: see [`hydroid_core::facts::Flow::receiver`].
    fn called_instance(&self, expr: &Expr) -> Option<Key> {
        if !matches!(expr.inferred_type(self.model), Some(Type::NominalInstance(_))) {
            return None;
        }
        instance_class(self.model, expr).filter(|(file, _)| describe(self.db, *file).1 == Origin::Project)
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
        // An instance ty knows nothing about: look the dunder up in the class it came from.
        let attribute = if keys.is_empty() {
            if !matches!(receiver.inferred_type(self.model), Some(Type::Dynamic(_))) {
                return;
            }
            let Some(class) = self.receiver_class(receiver) else { return };
            Some((class, dunder.to_string()))
        } else {
            None
        };
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
            implicit: true,
            attribute,
            returned_by: Vec::new(),
            sets: None,
            given: None,
            unpacks: false,
            instances: Vec::new(),
            dump: None,
            level: None,
            receiver: None,
            super_name: None,
            logging: None,
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
            // Callables in a literal list flow too (`FastAPI(on_startup=[init])`).
            let values: Vec<(RawValue, Option<Key>)> = match argument {
                Expr::List(ast::ExprList { elts, .. }) | Expr::Tuple(ast::ExprTuple { elts, .. }) => {
                    elts.iter().filter_map(|e| Some((self.value_of(e)?, self.called_instance(e)))).collect()
                }
                _ => self.value_of(argument).map(|v| (v, self.called_instance(argument))).into_iter().collect(),
            };
            // `response_model=list[Item]`: the classes it names.
            let values = if values.is_empty() && matches!(&slot, RawSlot::Keyword(k) if k == "response_model") {
                let mut classes = Vec::new();
                annotation_classes(self.model, argument, &mut classes, &mut |_| {});
                classes.into_iter().map(|c| (RawValue::Class(c), None)).collect()
            } else {
                values
            };
            for (value, receiver) in values {
                for &(into, bound) in &resolution.targets {
                    self.out.flows.push(RawFlow {
                        caller: self.current,
                        location: location.clone(),
                        into,
                        bound,
                        slot: slot.clone(),
                        value: value.clone(),
                        receiver,
                        literal: literal.clone(),
                    });
                }
            }
        }
        if let Some(name) = self.builtin_name(&call.func, &resolution) {
            self.builtin_call(name, call);
        }
        let attribute = if resolution.targets.is_empty() { self.attribute_slot(&call.func) } else { None };
        let returned_by =
            if resolution.targets.is_empty() && param.is_none() { self.returned_by(&call.func) } else { Vec::new() };
        let (receiver, super_name) = self.mro_receiver(&call.func, &resolution);
        let unresolved =
            (resolution.targets.is_empty() && param.is_none()).then_some(resolution.reason);
        // The declarative constructor sets each keyword argument on the new instance.
        let sets = resolution.constructs.and_then(|class| {
            let names: Vec<String> = call
                .arguments
                .keywords
                .iter()
                .filter_map(|k| k.arg.as_ref().map(|a| a.to_string()))
                .filter(|n| self.orm.validated.contains(n))
                .collect();
            (!names.is_empty()).then_some((class, names))
        });
        // A class called with keyword arguments only gives those fields. Positional arguments
        // (dataclasses) and `Model.model_validate(data)` may give any.
        let validates = matches!(&*call.func, Expr::Attribute(a) if VALIDATES.contains(&a.attr.as_str()));
        let given = (!validates && call.arguments.args.is_empty()).then(|| {
            call.arguments.keywords.iter().filter_map(|k| k.arg.as_ref().map(|a| a.to_string())).collect()
        });
        let unpacks = call.arguments.keywords.iter().any(|k| k.arg.is_none());
        let instances = if resolution.constructs.is_some() {
            call.arguments
                .keywords
                .iter()
                .filter(|k| self.holds_instances(self.model, &k.value, 0))
                .filter_map(|k| k.arg.as_ref().map(|a| a.to_string()))
                .collect()
        } else {
            Vec::new()
        };
        let dump = self.dump(call);
        self.response_options(call, &location);
        let level = self.level_argument(call);
        let logging = self.log_use(call, resolution.constructs.is_some());
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
            implicit: false,
            attribute,
            returned_by,
            sets,
            given,
            unpacks,
            instances,
            dump,
            level,
            receiver,
            super_name,
            logging,
        });
    }

    /// See [`hydroid_core::facts::Call::logging`].
    /// Whether a keyword argument of a construction holds nothing pydantic validates into a
    /// nested model (instances are kept as is, `revalidate_instances` defaults to `"never"`): an
    /// instance of a project class (`Attachment(...)`), `None`, a literal collection or a
    /// comprehension of those (`[]`), or a variable only ever bound to one of those or declared
    /// as one (`attachments: list[Attachment]`).
    fn holds_instances(&self, model: &SemanticModel<'db>, value: &Expr, depth: u8) -> bool {
        match value {
            Expr::NoneLiteral(_) => true,
            Expr::List(ast::ExprList { elts, .. })
            | Expr::Tuple(ast::ExprTuple { elts, .. })
            | Expr::Set(ast::ExprSet { elts, .. }) => elts.iter().all(|e| self.holds_instances(model, e, depth)),
            Expr::ListComp(ast::ExprListComp { elt, .. })
            | Expr::SetComp(ast::ExprSetComp { elt, .. })
            | Expr::Generator(ast::ExprGenerator { elt, .. }) => self.holds_instances(model, elt, depth),
            _ if is_project_instance(self.db, model, value) => true,
            Expr::Name(name) if depth < 3 => {
                let db = model.db();
                let definitions =
                    definitions_for_name(model, &name.id, value.into(), ImportAliasResolution::ResolveAliases);
                !definitions.is_empty()
                    && definitions.iter().all(|definition| {
                        let ResolvedDefinition::Definition(def) = definition else { return false };
                        if def.file(db) != model.file() {
                            return false;
                        }
                        let module = parsed_module(db, def.python_file(db)).load(db);
                        match def.kind(db) {
                            DefinitionKind::Assignment(a) if a.unpack().is_none() => {
                                self.holds_instances(model, a.value(&module), depth + 1)
                            }
                            DefinitionKind::AnnotatedAssignment(a) => {
                                declares_instances(self.db, model, a.annotation(&module))
                            }
                            DefinitionKind::Parameter(ParameterDefinitionNodeKind::Parameter(p)) => {
                                let annotation = p.node(&module).parameter.annotation();
                                annotation.is_some_and(|a| declares_instances(self.db, model, a))
                            }
                            _ => false,
                        }
                    })
            }
            _ => false,
        }
    }

    /// `response_model=None` and `response_model_exclude={...}` (see [`excluded_fields`]) given
    /// to a call.
    fn response_options(&mut self, call: &ast::ExprCall, location: &Location) {
        let no_model = call.arguments.find_keyword("response_model").is_some_and(|k| k.value.is_none_literal_expr());
        let exclude = excluded_fields(call, "response_model_exclude");
        if no_model || !exclude.is_empty() {
            let (caller, location) = (self.current, location.clone());
            self.out.response_options.push(RawResponseOptions { caller, location, no_model, exclude });
        }
    }

    /// See [`Dump`]: `obj.model_dump(...)` and the other pydantic dump methods by name (the
    /// analysis keeps it only for calls resolving to them).
    fn dump(&self, call: &ast::ExprCall) -> Option<Dump> {
        let Expr::Attribute(attr) = &*call.func else { return None };
        let json = match attr.attr.as_str() {
            "model_dump_json" | "json" => true,
            "dict" => false,
            "model_dump" => match call.arguments.find_keyword("mode").map(|k| &k.value) {
                None => false,
                Some(Expr::StringLiteral(mode)) => mode.value.to_str() != "python",
                Some(_) => true,
            },
            _ => return None,
        };
        let exclude = excluded_fields(call, "exclude");
        // Any key of a literal `include` is dumped (a nested set or dict dumps part of it).
        let include = match call.arguments.find_keyword("include").map(|k| &k.value) {
            None => None,
            Some(Expr::Set(set)) => {
                set.elts.iter().map(|e| Some(e.as_string_literal_expr()?.value.to_str().to_string())).collect()
            }
            Some(Expr::Dict(dict)) => dict
                .items
                .iter()
                .map(|item| match &item.key {
                    Some(Expr::StringLiteral(key)) => Some(key.value.to_str().to_string()),
                    _ => None,
                })
                .collect(),
            Some(_) => None,
        };
        Some(Dump { json, exclude, include })
    }

    fn log_use(&self, call: &ast::ExprCall, constructs: bool) -> Option<(Option<LoggerName>, Option<Key>, bool)> {
        if constructs {
            return self.attach_only.contains(&call.start().to_u32()).then_some((None, None, true));
        }
        let Expr::Attribute(attr) = &*call.func else { return None };
        let attaching = ATTACHES.contains(&attr.attr.as_str());
        if !attaching && !LOGS.contains(&attr.attr.as_str()) {
            return None;
        }
        let logger = if matches!(attr.value.inferred_type(self.model), Some(Type::ModuleLiteral(_))) {
            // `logging.info(...)` logs on the root logger.
            Some(LoggerName { name: String::new(), exact: true })
        } else {
            self.logger_name(self.model, &attr.value, 0)
        };
        let attaches = if attaching {
            let argument = call.arguments.args.first()?;
            self.receiver_class(argument).or_else(|| instance_class(self.model, argument))
        } else {
            None
        };
        Some((logger, attaches, false))
    }

    /// The name of the logger an expression (in the file of `model`) evaluates to:
    /// `logging.getLogger("audit")`, `getLogger(__name__)`, `getLogger(f"svc.{name}")` (only the
    /// start known), `logging.root`, a variable assigned one, or the call of a function returning
    /// one. `None` when not known.
    fn logger_name(&self, model: &SemanticModel<'db>, expr: &Expr, depth: u8) -> Option<LoggerName> {
        if depth > 4 {
            return None;
        }
        let db = model.db();
        let exact = |name: &str| {
            // `getLogger("root")` is the root logger too.
            let name = if name == "root" { "" } else { name };
            Some(LoggerName { name: name.to_string(), exact: true })
        };
        match expr {
            Expr::Call(call) if called_name(call) == Some("getLogger") => {
                let argument = call.arguments.args.first().or(call.arguments.find_keyword("name").map(|k| &k.value));
                match argument {
                    None | Some(Expr::NoneLiteral(_)) => exact(""),
                    Some(Expr::StringLiteral(s)) => exact(s.value.to_str()),
                    Some(Expr::Name(n)) if n.id.as_str() == "__name__" => exact(&describe(self.db, model.file()).0),
                    Some(Expr::FString(f)) => {
                        let mut name = String::new();
                        for part in f.value.iter() {
                            match part {
                                ast::FStringPartRef::Literal(literal) => name.push_str(&literal.value),
                                ast::FStringPartRef::FString(f) => {
                                    for element in &f.elements {
                                        let ast::InterpolatedStringElement::Literal(literal) = element else {
                                            return (!name.is_empty()).then_some(LoggerName { name, exact: false });
                                        };
                                        name.push_str(&literal.value);
                                    }
                                }
                            }
                        }
                        exact(&name)
                    }
                    _ => None,
                }
            }
            Expr::Attribute(attr) if attr.attr.as_str() == "root" => {
                matches!(attr.value.inferred_type(model), Some(Type::ModuleLiteral(_))).then(|| exact(""))?
            }
            Expr::Name(name) => {
                let mut found: Option<LoggerName> = None;
                for definition in definitions_for_name(model, &name.id, expr.into(), ImportAliasResolution::ResolveAliases)
                {
                    let ResolvedDefinition::Definition(def) = definition else { return None };
                    let module = parsed_module(db, def.python_file(db)).load(db);
                    let value = match def.kind(db) {
                        DefinitionKind::Assignment(a) if a.unpack().is_none() => a.value(&module),
                        DefinitionKind::AnnotatedAssignment(a) => a.value(&module)?,
                        _ => return None,
                    };
                    let other;
                    let model = if def.file(db) == model.file() {
                        model
                    } else {
                        other = SemanticModel::new(db, ty_python_semantic::Db::program_file(db, def.file(db)));
                        &other
                    };
                    let name = self.logger_name(model, value, depth + 1)?;
                    if found.as_ref().is_some_and(|f| *f != name) {
                        return None;
                    }
                    found = Some(name);
                }
                found
            }
            // `service_logger("payments")`: what the function returns.
            Expr::Call(call) if model.file() == self.model.file() => {
                let Some(RawValue::Function(key)) = self.value_of(&call.func) else { return None };
                let model = SemanticModel::new(db, ty_python_semantic::Db::program_file(db, key.0));
                let module = parsed_module(db, model.python_file()).load(db);
                let def = find_function(module.syntax(), key.1)?;
                let mut found: Option<LoggerName> = None;
                for value in return_values(&def.body) {
                    let name = self.logger_name(&model, value, depth + 1)?;
                    if found.as_ref().is_some_and(|f| *f != name) {
                        return None;
                    }
                    found = Some(name);
                }
                found
            }
            _ => None,
        }
    }

    /// `logger.propagate = value` (see [`hydroid_core::facts::Facts::stops_propagation`]).
    fn propagation(&mut self, target: &Expr, value: &Expr) {
        let Expr::Attribute(attr) = target else { return };
        if attr.attr.as_str() != "propagate" {
            return;
        }
        if let Some(LoggerName { name, exact: true }) = self.logger_name(self.model, &attr.value, 0) {
            self.out.stops_propagation.push((name, matches!(value, Expr::BooleanLiteral(b) if !b.value)));
        }
    }

    /// The logging level a call passes: its `level=` keyword, or the first argument of
    /// `setLevel(...)` (with the class of the receiver), `log(...)` and `__init__(...)`. See
    /// [`hydroid_core::facts::Call::level`].
    fn level_argument(&self, call: &ast::ExprCall) -> Option<(Option<u32>, Option<Key>)> {
        if let Some(keyword) = call.arguments.find_keyword("level") {
            return Some((self.level_value(&keyword.value), None));
        }
        let Expr::Attribute(attr) = &*call.func else { return None };
        let first = call.arguments.args.first().filter(|a| !a.is_starred_expr())?;
        match attr.attr.as_str() {
            "setLevel" => Some((self.level_value(first), self.receiver_class(&attr.value))),
            // `super().__init__(logging.ERROR)` in a `logging.Handler` subclass.
            "log" | "__init__" => Some((self.level_value(first), None)),
            _ => None,
        }
    }

    /// `logging.ERROR`, `ERROR`, `40`, `"ERROR"`, or a name ty knows the literal value of.
    fn level_value(&self, expr: &Expr) -> Option<u32> {
        let named = |name: &str| match name {
            "NOTSET" => Some(0),
            "DEBUG" => Some(10),
            "INFO" => Some(20),
            "WARN" | "WARNING" => Some(30),
            "ERROR" => Some(40),
            "FATAL" | "CRITICAL" => Some(50),
            _ => None,
        };
        match expr {
            Expr::StringLiteral(s) => named(s.value.to_str()),
            Expr::NumberLiteral(n) => n.value.as_int().and_then(|i| i.as_u16()).map(u32::from),
            Expr::Attribute(attr) if matches!(&*attr.value, Expr::Name(n) if n.id.as_str() == "logging") => {
                named(attr.attr.as_str())
            }
            _ => {
                let ty = expr.inferred_type(self.model)?;
                let shown = ty.display(self.model.db(), &self.model.program_environment()).to_string();
                shown.strip_prefix("Literal[")?.strip_suffix(']')?.parse().ok()
            }
        }
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
            // `@render.register`: an implementation of a `functools.singledispatch` function.
            if let Expr::Attribute(attr) = callee
                && attr.attr.as_str() == "register"
                && let Expr::Name(dispatcher) = &*attr.value
            {
                let definitions = definitions_for_name(
                    self.model,
                    &dispatcher.id,
                    (&*attr.value).into(),
                    ImportAliasResolution::ResolveAliases,
                );
                for dispatcher in self.resolved_function_keys(definitions) {
                    self.out.dispatches.push((dispatcher, key));
                }
            }
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
                    receiver: None,
                    literal: literal.clone(),
                });
            }
        }
        self.out.decorators.push((key, decorators));
    }

    /// What FastAPI validates and serializes around `def` if it is an endpoint or a dependency.
    fn signature(&mut self, def: &ast::StmtFunctionDef) {
        let key = def.name.start().to_u32();
        let parameters = &def.parameters;
        let mut params = Vec::new();
        for parameter in parameters.posonlyargs.iter().chain(&parameters.args).chain(&parameters.kwonlyargs) {
            let name = parameter.parameter.name.as_str();
            let Some(annotation) = parameter.parameter.annotation.as_deref() else { continue };
            let is_dependency =
                |call: &ast::ExprCall| called_name(call).is_some_and(|n| DEPENDENCY_MARKERS.contains(&n));
            let default_dependency = matches!(parameter.default.as_deref(), Some(Expr::Call(c)) if is_dependency(c));
            if name == "self" || name == "cls" || default_dependency {
                continue;
            }
            let mut classes = Vec::new();
            let mut calls = Vec::new();
            annotation_classes(self.model, annotation, &mut classes, &mut |call| calls.push(call));
            if calls.iter().any(|c| is_dependency(c)) {
                continue;
            }
            params.extend(classes.into_iter().map(RawValue::Class));
            for call in calls.into_iter().filter(|c| called_name(c).is_some_and(|n| VALIDATOR_METADATA.contains(&n))) {
                let func = call.arguments.args.first().or_else(|| call.arguments.find_keyword("func").map(|k| &k.value));
                if let Some(value @ RawValue::Function(_)) = func.and_then(|f| self.value_of(f)) {
                    params.push(value);
                }
            }
        }
        let (mut annotation, mut returned, mut returns_unknown) = (Vec::new(), Vec::new(), false);
        let mut returns_response = false;
        if !yields(&def.body) {
            if let Some(returns) = def.returns.as_deref() {
                annotation_classes(self.model, returns, &mut annotation, &mut |_| {});
            }
            let db = self.model.db();
            returned = self.returned_classes((self.index.file, key));
            returned.retain(|(file, _)| !file.path(db).is_vendored_path());
            returns_unknown = return_values(&def.body).iter().any(|value| match value.inferred_type(self.model) {
                Some(Type::Union(union)) => union.elements(db).iter().any(|t| matches!(t, Type::Dynamic(_))),
                Some(ty) => matches!(ty, Type::Dynamic(_)),
                None => true,
            });
            returns_response = self.returns_response(&def.body);
        }
        if !params.is_empty() || !annotation.is_empty() || !returned.is_empty() || returns_unknown || returns_response {
            let signature = RawSignature { function: key, params, annotation, returned, returns_unknown, returns_response };
            self.out.signatures.push(signature);
        }
    }

    /// Whether a function body always returns a Starlette `Response` (every `return` gives an
    /// instance of one, and the body cannot end without returning).
    fn returns_response(&self, body: &[Stmt]) -> bool {
        let db = self.model.db();
        let values = return_values(body);
        !values.is_empty()
            && ends_in_return(body)
            && values.iter().all(|value| {
                let types = match value.inferred_type(self.model) {
                    Some(Type::Union(union)) => union.elements(db).to_vec(),
                    Some(ty) => vec![ty],
                    None => return false,
                };
                types.iter().all(|ty| {
                    matches!(ty, Type::NominalInstance(_))
                        && self.class_key(ty).is_some_and(|c| inherits_from(self.db, c, "starlette.responses.Response"))
                })
            })
    }

    /// Implicit SQLAlchemy work on an attribute of a mapped instance: `obj.email = ...` runs the
    /// `@validates("email")` validators, reading a lazily loaded relationship (`author.books`)
    /// queries through the session.
    fn orm_attribute(&mut self, attr: &ast::ExprAttribute) {
        let name = attr.attr.as_str();
        let validated = attr.ctx == ExprContext::Store && self.orm.validated.contains(name);
        let lazy = attr.ctx == ExprContext::Load && self.orm.lazy_names.contains(name) && self.orm.load.is_some();
        if !validated && !lazy {
            return;
        }
        let Some(class) = self.mapped_instance_class(&attr.value) else { return };
        let (targets, sets) = if validated {
            (Vec::new(), Some((class, vec![name.to_string()])))
        } else {
            if !self.orm.lazy.contains(&(class, name.to_string()))
                || !self.may_be_persistent(&attr.value)
                || self.preloaded_off_loop(&attr.value, name, attr.start().to_u32())
            {
                return;
            }
            let load = self.orm.load.expect("checked above");
            self.lazy_reads.push((self.current, self.index.location(attr.start()), name.to_string()));
            (vec![(load, true)], None)
        };
        self.out.calls.push(RawCall {
            caller: self.current,
            location: self.index.location(attr.start()),
            text: self.text(attr.range()),
            targets,
            virtual_dispatch: false,
            param: None,
            awaited: false,
            iterates: false,
            constructs: None,
            unresolved: None,
            accessor: None,
            implicit: true,
            attribute: None,
            returned_by: Vec::new(),
            sets,
            given: None,
            unpacks: false,
            instances: Vec::new(),
            dump: None,
            level: None,
            receiver: None,
            super_name: None,
            logging: None,
        });
    }

    /// The class of a (possibly mapped) instance: its type (`T | None`, `T & ~AlwaysFalsy`
    /// narrowed), else, for a variable ty cannot type (`run_in_threadpool(db.get, Author, id)`
    /// leaves the type variable unsolved), the one mapped class passed to the call it was
    /// assigned from (`Session.get` and friends return an instance of the entity they are given).
    fn mapped_instance_class(&self, receiver: &Expr) -> Option<Key> {
        if let Some(class) = self.receiver_class(receiver).or_else(|| instance_class(self.model, receiver)) {
            return Some(class);
        }
        let db = self.model.db();
        match receiver.inferred_type(self.model)? {
            Type::Intersection(intersection) => {
                let mut positive = intersection.iter_positive(db).filter(|t| matches!(t, Type::NominalInstance(_)));
                return positive.next().and_then(|ty| self.class_key(&ty));
            }
            Type::NominalInstance(_) | Type::ClassLiteral(_) | Type::ModuleLiteral(_) | Type::FunctionLiteral(_) => {
                return None;
            }
            _ => {}
        }
        let Expr::Name(name) = receiver else { return None };
        let mut found = None;
        for definition in definitions_for_name(self.model, &name.id, receiver.into(), ImportAliasResolution::ResolveAliases) {
            let ResolvedDefinition::Definition(def) = definition else { return None };
            let DefinitionKind::Assignment(assignment) = def.kind(db) else { return None };
            if def.file(db) != self.index.file || assignment.unpack().is_some() {
                return None;
            }
            let module = parsed_module(db, def.python_file(db)).load(db);
            let mut value = assignment.value(&module);
            if let Expr::Await(await_expr) = value {
                value = &await_expr.value;
            }
            let Expr::Call(call) = value else { return None };
            let mut mapped = call.arguments.args.iter().filter_map(|argument| {
                let ty = argument.inferred_type(self.model).filter(is_class_object)?;
                self.class_key(&ty).filter(|class| self.orm.lazy.iter().any(|(c, _)| c == class))
            });
            let class = mapped.next()?;
            if mapped.next().is_some() || found.is_some_and(|c| c != class) {
                return None;
            }
            found = Some(class);
        }
        found
    }

    /// Whether a mapped instance may be attached to a sync session. Not when the variable holding
    /// it is assigned a new instance (transient objects load nothing), nor when it was obtained by
    /// awaiting something in a function using the async ORM: an `AsyncSession` lazy load raises
    /// `MissingGreenlet` instead of blocking. Awaiting an offload API with a sync callable
    /// (`await run_in_threadpool(db.get, Author, id)`) does return an instance of a sync session.
    fn may_be_persistent(&self, receiver: &Expr) -> bool {
        if self.uses_async_orm() {
            return false;
        }
        self.may_be_persistent_value(receiver, 0)
    }

    fn may_be_persistent_value(&self, value: &Expr, depth: u8) -> bool {
        if depth > 3 {
            return true;
        }
        match value {
            Expr::Await(await_expr) => {
                matches!(&*await_expr.value, Expr::Call(call) if self.offloads(call))
            }
            Expr::Call(call) if call.func.inferred_type(self.model).is_some_and(|t| is_class_object(&t)) => false,
            Expr::Name(name) => {
                let db = self.model.db();
                let definitions =
                    definitions_for_name(self.model, &name.id, value.into(), ImportAliasResolution::ResolveAliases);
                for definition in definitions {
                    let ResolvedDefinition::Definition(def) = definition else { continue };
                    if def.file(db) != self.index.file {
                        continue;
                    }
                    let module = parsed_module(db, def.python_file(db)).load(db);
                    let assigned = match def.kind(db) {
                        DefinitionKind::Assignment(a) => Some(a.value(&module)),
                        DefinitionKind::AnnotatedAssignment(a) => a.value(&module),
                        DefinitionKind::For(f) => Some(f.iterable(&module)),
                        DefinitionKind::Comprehension(c) => Some(c.iterable(&module)),
                        _ => None,
                    };
                    let Some(assigned) = assigned else { continue };
                    if !self.may_be_persistent_value(assigned, depth + 1) {
                        return false;
                    }
                }
                true
            }
            // `(await session.scalars(...)).first()`, `for x in await f()`: anything awaited on
            // the way, unless it offloads.
            _ => awaited_calls(value).iter().all(|call| self.offloads(call)),
        }
    }

    /// `run_in_threadpool(db.get, ...)`, `asyncio.to_thread(f)`: a call handed a sync callable.
    fn offloads(&self, call: &ast::ExprCall) -> bool {
        call.arguments.args.iter().any(|a| matches!(self.value_of(a), Some(RawValue::Function(_))))
    }

    /// Whether the current function (or one enclosing it) mentions the async ORM:
    /// `session: AsyncSession`, `async_sessionmaker`.
    fn uses_async_orm(&self) -> bool {
        let db = self.model.db();
        let parsed = parsed_module(db, self.model.python_file()).load(db);
        let mut scope = Some(self.current);
        while let Some(key) = scope {
            if key != MODULE_KEY
                && let Some(def) = find_function(parsed.syntax(), key)
            {
                let text = self.index.text(def.range());
                if text.contains("AsyncSession") || text.contains("async_sessionmaker") {
                    return true;
                }
            }
            scope = self.index.functions.get(&key).and_then(|f| f.parent);
        }
        false
    }

    /// Whether relationship `name` of the instance a variable holds was already loaded before
    /// this read (at offset `read`), off the loop: the variable holds what an awaited offload
    /// returned (`project = await run_in_threadpool(load_project, db, id)`), or was passed to one
    /// (`await to_thread(prepare, project)`), and the helper reads the relationship on that
    /// value or refreshes it by name (`db.refresh(project, attribute_names=["members"])`). A
    /// loaded collection sits in the instance `__dict__`, which attribute reads return without
    /// loading. Not when anything that may expire it (`commit()`, `expire(...)`, ...) runs in
    /// between, or the variable is assigned again.
    fn preloaded_off_loop(&self, receiver: &Expr, name: &str, read: u32) -> bool {
        let Expr::Name(var) = receiver else { return false };
        let var = var.id.as_str();
        let db = self.model.db();
        let parsed = parsed_module(db, self.model.python_file()).load(db);
        let Some(def) = find_function(parsed.syntax(), self.current) else { return false };
        let body = scope_items(&def.body);
        let stores: Vec<u32> = body.stores.iter().filter(|(n, _)| *n == var).map(|(_, at)| *at).collect();
        for (call, assigned) in &body.awaited {
            let (start, end) = (call.start().to_u32(), call.end().to_u32());
            if end > read {
                continue;
            }
            // The value from where it is set (or passed) until the read stays as it was.
            let since = if *assigned == Some(var) { end } else { start };
            if stores.iter().any(|&at| at > since && at < read)
                || body.calls.iter().any(|c| c.start().to_u32() > end && c.start().to_u32() < read && expires(c, var))
            {
                continue;
            }
            let args = &call.arguments.args;
            let Some(at) = args.iter().position(|a| matches!(self.value_of(a), Some(RawValue::Function(_)))) else {
                continue;
            };
            let Some(RawValue::Function(helper)) = self.value_of(&args[at]) else { continue };
            let helper_file = SemanticModel::new(db, ty_python_semantic::Db::program_file(db, helper.0)).python_file();
            let helper_module = parsed_module(db, helper_file).load(db);
            let Some(helper_def) = find_function(helper_module.syntax(), helper.1) else { continue };
            let mut params: Vec<&str> =
                helper_def.parameters.iter_non_variadic_params().map(|p| p.parameter.name.as_str()).collect();
            if matches!(&args[at], Expr::Attribute(_)) && matches!(params.first(), Some(&("self" | "cls"))) {
                params.remove(0);
            }
            // The helper's names for the variable: the parameter it is passed as, and what it
            // returns when the result is assigned to the variable.
            let mut names: Vec<&str> = Vec::new();
            let passed = |e: &Expr| matches!(e, Expr::Name(n) if n.id.as_str() == var);
            for (i, a) in args.iter().enumerate().skip(at + 1) {
                if a.is_starred_expr() {
                    break;
                }
                if passed(a) {
                    names.extend(params.get(i - at - 1));
                }
            }
            for keyword in &call.arguments.keywords {
                if let Some(arg) = &keyword.arg
                    && passed(&keyword.value)
                {
                    names.extend(params.iter().find(|p| **p == arg.as_str()));
                }
            }
            let helper_body = scope_items(&helper_def.body);
            if names.iter().any(|p| helper_body.preloads(p, name)) {
                return true;
            }
            if *assigned == Some(var) {
                let returned: Vec<&Expr> =
                    return_values(&helper_def.body).into_iter().filter(|v| !v.is_none_literal_expr()).collect();
                if !returned.is_empty()
                    && returned
                        .iter()
                        .all(|v| matches!(v, Expr::Name(n) if helper_body.preloads(n.id.as_str(), name)))
                {
                    return true;
                }
            }
        }
        false
    }

    /// Drops the lazy relationship reads whose relationship the function (or one it calls or
    /// passes on, or the module) loads eagerly: `db.get(Author, id, options=[selectinload(Author.books)])`.
    fn drop_eager_reads(&mut self) {
        let Some(load) = self.orm.load else { return };
        let file = self.index.file;
        let mut dropped: Vec<(u32, Location)> = Vec::new();
        for (function, location, name) in std::mem::take(&mut self.lazy_reads) {
            let mut sources: Vec<Key> = vec![(file, MODULE_KEY)];
            let mut scope = Some(function);
            while let Some(key) = scope {
                sources.push((file, key));
                scope = self.index.functions.get(&key).and_then(|f| f.parent);
            }
            sources.extend(
                self.out.calls.iter().filter(|c| c.caller == function).flat_map(|c| c.targets.iter().map(|(k, _)| *k)),
            );
            sources.extend(self.out.flows.iter().filter(|f| f.caller == function).filter_map(|f| match f.value {
                RawValue::Function(key) => Some(key),
                _ => None,
            }));
            let eager = sources
                .iter()
                .filter_map(|k| self.orm.eager.get(k))
                .flatten()
                .any(|n| n == &name || n == "*");
            if eager {
                dropped.push((function, location));
            }
        }
        self.out.calls.retain(|c| {
            !(c.targets.len() == 1
                && c.targets[0].0 == load
                && c.implicit
                && dropped.iter().any(|(f, l)| *f == c.caller && *l == c.location))
        });
    }

    /// `name: T = Field(default_factory=f)` (or `field(...)`) in the body of `class`. The field can
    /// also be given by its alias.
    fn default_factory(&mut self, class: u32, field: &Stmt) {
        let (target, value) = match field {
            Stmt::AnnAssign(ast::StmtAnnAssign { target, value: Some(value), .. }) => (&**target, &**value),
            Stmt::Assign(ast::StmtAssign { targets, value, .. }) if targets.len() == 1 => (&targets[0], &**value),
            _ => return,
        };
        let (Expr::Name(name), Expr::Call(call)) = (target, value) else { return };
        let keyword = |arg: &str| call.arguments.find_keyword(arg).map(|k| &k.value);
        let Some(factory) = keyword("default_factory").and_then(|f| self.value_of(f)) else { return };
        let mut names = vec![name.id.to_string()];
        for alias in ["alias", "validation_alias"].into_iter().filter_map(keyword) {
            if let Expr::StringLiteral(alias) = alias {
                names.push(alias.value.to_str().to_string());
            }
        }
        self.out.default_factories.push((class, names, factory));
    }

    /// `name: T = ...` in the body of a project `class` (not a class variable or a private
    /// attribute, which pydantic neither validates nor dumps): see [`RawField`].
    fn model_field(&mut self, class: u32, field: &Stmt) {
        let Stmt::AnnAssign(ast::StmtAnnAssign { target, annotation, value, .. }) = field else { return };
        let Expr::Name(name) = &**target else { return };
        let is_class_var = match &**annotation {
            Expr::Subscript(s) => match &*s.value {
                Expr::Name(n) => n.id == "ClassVar",
                Expr::Attribute(a) => a.attr.as_str() == "ClassVar",
                _ => false,
            },
            _ => false,
        };
        if self.index.origin != Origin::Project || is_class_var || name.id.starts_with('_') {
            return;
        }
        let mut field = RawField { names: vec![name.id.to_string()], ..RawField::default() };
        if let Some(Expr::Call(call)) = value.as_deref() {
            for alias in ["alias", "validation_alias"] {
                if let Some(Expr::StringLiteral(alias)) = call.arguments.find_keyword(alias).map(|k| &k.value) {
                    field.names.push(alias.value.to_str().to_string());
                }
            }
        }
        self.field_types(self.model, annotation, 3, &mut field);
        field.classes.retain(|&c| c != (self.index.file, class));
        if !field.classes.is_empty() || !field.validators.is_empty() || !field.serializers.is_empty() {
            self.out.model_fields.push((class, field));
        }
    }

    /// The classes a field annotation names and the validators and serializers its `Annotated`
    /// metadata attaches, looking through type aliases (`Label = Annotated[str, ...]`, also in
    /// other modules) up to `depth` levels.
    fn field_types(&self, model: &SemanticModel<'db>, annotation: &Expr, depth: u8, out: &mut RawField) {
        let db = self.model.db();
        match annotation {
            Expr::Name(_) | Expr::Attribute(_) => {
                let found = out.classes.len();
                annotation_classes(model, annotation, &mut out.classes, &mut |_| {});
                if out.classes.len() > found || depth == 0 {
                    return;
                }
                let definitions = match annotation {
                    Expr::Name(name) => {
                        definitions_for_name(model, &name.id, annotation.into(), ImportAliasResolution::ResolveAliases)
                    }
                    Expr::Attribute(attr) => definitions_for_attribute(model, attr),
                    _ => Vec::new(),
                };
                for definition in definitions {
                    let ResolvedDefinition::Definition(def) = definition else { continue };
                    let file = def.file(db);
                    if file.path(db).is_vendored_path() {
                        continue;
                    }
                    let module = parsed_module(db, def.python_file(db)).load(db);
                    let value = match def.kind(db) {
                        DefinitionKind::TypeAlias(alias) => Some(&*alias.node(&module).value),
                        kind => kind.value(&module),
                    };
                    let Some(value @ (Expr::Subscript(_) | Expr::Name(_) | Expr::Attribute(_) | Expr::BinOp(_))) = value
                    else {
                        continue;
                    };
                    let other = SemanticModel::new(db, ty_python_semantic::Db::program_file(db, file));
                    self.field_types(&other, value, depth - 1, out);
                }
            }
            Expr::Subscript(subscript) => {
                self.field_types(model, &subscript.value, depth, out);
                self.field_types(model, &subscript.slice, depth, out);
            }
            Expr::Tuple(tuple) => {
                for element in &tuple.elts {
                    self.field_types(model, element, depth, out);
                }
            }
            Expr::BinOp(binop) => {
                self.field_types(model, &binop.left, depth, out);
                self.field_types(model, &binop.right, depth, out);
            }
            Expr::Call(call) => {
                let serializer = match called_name(call) {
                    Some(name) if VALIDATOR_METADATA.contains(&name) => false,
                    Some(name) if SERIALIZER_METADATA.contains(&name) => true,
                    _ => return,
                };
                let func = call.arguments.args.first().or_else(|| call.arguments.find_keyword("func").map(|k| &k.value));
                if let Some(value @ RawValue::Function(_)) = func.and_then(|f| self.function_value(model, f)) {
                    if !serializer {
                        out.validators.push(value);
                    } else if json_only(call) {
                        out.json_serializers.push(value.clone());
                        out.serializers.push(value);
                    } else {
                        out.serializers.push(value);
                    }
                }
            }
            _ => {}
        }
    }

    /// The function an expression refers to, in the file of `model` (which may be another one).
    fn function_value(&self, model: &SemanticModel<'db>, expr: &Expr) -> Option<RawValue> {
        if model.python_file() == self.model.python_file() {
            return self.value_of(expr);
        }
        let db = self.model.db();
        if let Expr::Lambda(lambda) = expr {
            return Some(RawValue::Function((model.python_file().file(db), lambda.start().to_u32())));
        }
        match expr.inferred_type(model)? {
            ty @ (Type::FunctionLiteral(_) | Type::BoundMethod(_)) => match ty.definition(db, &model.program_environment())? {
                TypeDefinition::Function(def) => self.function_key(def).map(RawValue::Function),
                _ => None,
            },
            _ => None,
        }
    }

    /// Whether a `try` body only asks for the running loop (`asyncio.get_running_loop()`, maybe
    /// assigned): the only exception it can raise is the `RuntimeError` of "no running loop".
    fn probes_running_loop(&self, body: &[Stmt]) -> bool {
        let simple =
            body.iter().all(|s| matches!(s, Stmt::Expr(_) | Stmt::Assign(_) | Stmt::AnnAssign(_) | Stmt::Pass(_)));
        let calls = body_calls(body);
        simple
            && !calls.is_empty()
            && calls.iter().all(|call| {
                called_name(call) == Some("get_running_loop")
                    && self
                        .resolve(&call.func, Some(call))
                        .targets
                        .iter()
                        .any(|((file, _), _)| {
                            matches!(describe(self.db, *file).0.as_str(), "_asyncio" | "asyncio.events")
                        })
            })
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
                if self.wanted.is_none() && self.index.origin == Origin::Project {
                    self.signature(def);
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
                    self.propagation(target, &assign.value);
                }
                walk_stmt(self, stmt);
            }
            Stmt::AnnAssign(assign) if self.recording() => {
                if let Some(value) = &assign.value {
                    self.store(&assign.target, value);
                }
                walk_stmt(self, stmt);
            }
            Stmt::ClassDef(def) if self.recording() => {
                for field in &def.body {
                    self.default_factory(def.name.start().to_u32(), field);
                    self.model_field(def.name.start().to_u32(), field);
                }
                walk_stmt(self, stmt);
            }
            Stmt::Return(ast::StmtReturn { value: Some(value), .. })
                if self.recording() && self.current != MODULE_KEY =>
            {
                if let Some(value) = self.value_of(value) {
                    self.out.returns.push((self.current, value));
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
            // `try: asyncio.get_running_loop()` / `except RuntimeError:`: the handler runs only
            // when no loop is running in this thread, so never on the loop. Its calls are left
            // out (a sync helper shared by loop code and worker threads).
            Stmt::Try(try_stmt) if self.recording() && self.probes_running_loop(&try_stmt.body) => {
                self.visit_body(&try_stmt.body);
                for handler in &try_stmt.handlers {
                    let ast::ExceptHandler::ExceptHandler(h) = handler;
                    if !catches_runtime_error(h.type_.as_deref()) {
                        self.visit_except_handler(handler);
                    }
                }
                self.visit_body(&try_stmt.orelse);
                self.visit_body(&try_stmt.finalbody);
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        if let Expr::Attribute(attr) = expr
            && self.recording()
        {
            self.orm_attribute(attr);
        }
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
            Expr::Subscript(subscript) if self.recording() => {
                let dunder = match subscript.ctx {
                    ExprContext::Store => "__setitem__",
                    ExprContext::Del => "__delitem__",
                    _ => "__getitem__",
                };
                if self.operator_may_run(&subscript.value, dunder) {
                    let text = self.text(subscript.range());
                    self.implicit_call(&subscript.value, dunder, text, false, false);
                }
                walk_expr(self, expr);
            }
            Expr::Compare(compare) if self.recording() => {
                for (op, right) in compare.ops.iter().zip(compare.comparators()) {
                    if matches!(op, ast::CmpOp::In | ast::CmpOp::NotIn)
                        && self.operator_may_run(right, "__contains__")
                    {
                        let text = self.text(compare.range());
                        self.implicit_call(right, "__contains__", text, false, false);
                    }
                }
                walk_expr(self, expr);
            }
            Expr::BinOp(binop) if self.recording() => {
                let dunder = binop.op.dunder();
                if self.operator_may_run(&binop.left, dunder) {
                    let text = self.text(binop.range());
                    self.implicit_call(&binop.left, dunder, text, false, false);
                }
                walk_expr(self, expr);
            }
            Expr::YieldFrom(yield_from) if self.recording() => {
                self.iterate(&yield_from.value);
                walk_expr(self, expr);
            }
            Expr::Call(call) => {
                self.callees.insert((call.func.start().to_u32(), call.func.end().to_u32()));
                // Recorded after the calls in its receiver and arguments, in evaluation order (the
                // first blocking call to run is the witness), but handled first: it marks which
                // of them it awaits or iterates.
                let mark = self.out.calls.len();
                if self.recording() {
                    self.handle_call(call);
                }
                let handled = self.out.calls.split_off(mark);
                walk_expr(self, expr);
                self.out.calls.extend(handled);
            }
            Expr::Attribute(attr)
                if self.recording()
                    && (self.may_run(attr.attr.as_str()) || self.may_run_getattr(attr))
                    && !self.callees.contains(&(attr.start().to_u32(), attr.end().to_u32())) =>
            {
                let accessor = match attr.ctx {
                    ExprContext::Store => Accessor::Set,
                    ExprContext::Del => Accessor::Delete,
                    _ => Accessor::Get,
                };
                let definitions = definitions_for_attribute(self.model, attr);
                self.descriptor_calls(attr, accessor, &definitions);
                // No such attribute: `__getattr__` computes it (lazy settings, proxies).
                if definitions.is_empty() && accessor == Accessor::Get && self.may_run_getattr(attr) {
                    let text = self.text(attr.range());
                    self.implicit_call(&attr.value, "__getattr__", text, false, false);
                }
                let keys = self.resolved_function_keys(definitions);
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
                        implicit: true,
                        attribute: None,
                        returned_by: Vec::new(),
                        sets: None,
                        given: None,
                        unpacks: false,
                        instances: Vec::new(),
                        dump: None,
                        level: None,
                        receiver: None,
                        super_name: None,
                        logging: None,
                    });
                }
                walk_expr(self, expr);
            }
            _ => walk_expr(self, expr),
        }
    }
}

/// The calls awaited in an expression (`await session.get(...)`), outside nested scopes.
fn awaited_calls(expr: &Expr) -> Vec<&ast::ExprCall> {
    struct Finder<'a>(Vec<&'a ast::ExprCall>);
    impl<'a> SourceOrderVisitor<'a> for Finder<'a> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Lambda(_) => {}
                Expr::Await(await_expr) => {
                    if let Expr::Call(call) = &*await_expr.value {
                        self.0.push(call);
                    }
                    walk_expr(self, expr);
                }
                _ => walk_expr(self, expr),
            }
        }
    }
    let mut finder = Finder(Vec::new());
    finder.visit_expr(expr);
    finder.0
}

/// Every call in a block of statements, nested scopes included.
fn body_calls(body: &[Stmt]) -> Vec<&ast::ExprCall> {
    struct Finder<'a>(Vec<&'a ast::ExprCall>);
    impl<'a> SourceOrderVisitor<'a> for Finder<'a> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Call(call) = expr {
                self.0.push(call);
            }
            walk_expr(self, expr);
        }
    }
    let mut finder = Finder(Vec::new());
    finder.visit_body(body);
    finder.0
}

/// Constructions whose instance is only attached to a logger or handler (see
/// [`hydroid_core::facts::LogUse::attach_only`]): the argument of `addHandler(...)`,
/// `addFilter(...)` or `setFormatter(...)`, or assigned to a function's variable that is only
/// passed to those or has methods called on it (`handler.setLevel(...)`).
fn attach_only(body: &[Stmt]) -> FxHashSet<u32> {
    #[derive(Default)]
    struct Uses<'a> {
        functions: Vec<&'a ast::StmtFunctionDef>,
        assigned: Vec<(&'a str, u32)>,
        stores: Vec<&'a str>,
        /// Variable reads, and whether each is an attachment or a method call.
        loads: Vec<(&'a str, bool)>,
        direct: FxHashSet<u32>,
        attached: FxHashSet<u32>,
    }
    impl<'a> SourceOrderVisitor<'a> for Uses<'a> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if let Stmt::FunctionDef(def) = stmt {
                self.functions.push(def);
            }
            if let Stmt::Assign(assign) = stmt
                && let ([Expr::Name(name)], Expr::Call(call)) = (&assign.targets[..], &*assign.value)
            {
                self.assigned.push((name.id.as_str(), call.start().to_u32()));
            }
            walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Call(call) => {
                    if let Expr::Attribute(attr) = &*call.func {
                        if let Expr::Name(name) = &*attr.value {
                            self.attached.insert(name.start().to_u32());
                        }
                        if ATTACHES.contains(&attr.attr.as_str()) {
                            match call.arguments.args.first() {
                                Some(Expr::Call(argument)) => {
                                    self.direct.insert(argument.start().to_u32());
                                }
                                Some(Expr::Name(name)) => {
                                    self.attached.insert(name.start().to_u32());
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Expr::Name(name) if name.ctx == ExprContext::Store => self.stores.push(name.id.as_str()),
                Expr::Name(name) if name.ctx == ExprContext::Load => {
                    self.loads.push((name.id.as_str(), self.attached.contains(&name.start().to_u32())));
                }
                _ => {}
            }
            walk_expr(self, expr);
        }
    }
    let mut module = Uses::default();
    module.visit_body(body);
    let mut found = std::mem::take(&mut module.direct);
    for def in module.functions {
        let mut uses = Uses::default();
        uses.visit_body(&def.body);
        for (name, call) in &uses.assigned {
            let assigned_once = uses.stores.iter().filter(|s| *s == name).count() == 1;
            if assigned_once && uses.loads.iter().all(|(n, attached)| n != name || *attached) {
                found.insert(*call);
            }
        }
    }
    found
}

/// What a function body (its own scope) does, for following an ORM instance through it.
#[derive(Default)]
struct ScopeItems<'a> {
    /// Awaited calls, with the variable the result is assigned to (`x = await f(...)`).
    awaited: Vec<(&'a ast::ExprCall, Option<&'a str>)>,
    calls: Vec<&'a ast::ExprCall>,
    /// Variables assigned, and where.
    stores: Vec<(&'a str, u32)>,
    /// `name.attribute` reads: `(name, attribute, offset)`.
    reads: Vec<(&'a str, &'a str, u32)>,
}

impl<'a> SourceOrderVisitor<'a> for ScopeItems<'a> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
            _ => {
                let (targets, value): (&[Expr], _) = match stmt {
                    Stmt::Assign(a) => (&a.targets, Some(&*a.value)),
                    Stmt::AnnAssign(a) => (std::slice::from_ref(&*a.target), a.value.as_deref()),
                    _ => (&[], None),
                };
                if let ([Expr::Name(name)], Some(Expr::Await(await_expr))) = (targets, value)
                    && let Expr::Call(call) = &*await_expr.value
                {
                    self.awaited.push((call, Some(name.id.as_str())));
                }
                walk_stmt(self, stmt);
            }
        }
    }
    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Lambda(_) => return,
            Expr::Await(await_expr) => {
                if let Expr::Call(call) = &*await_expr.value
                    && !self.awaited.iter().any(|(c, _)| std::ptr::eq(*c, call))
                {
                    self.awaited.push((call, None));
                }
            }
            Expr::Call(call) => self.calls.push(call),
            Expr::Name(name) if name.ctx == ExprContext::Store => {
                self.stores.push((name.id.as_str(), name.start().to_u32()));
            }
            Expr::Attribute(attr) if attr.ctx == ExprContext::Load => {
                if let Expr::Name(name) = &*attr.value {
                    self.reads.push((name.id.as_str(), attr.attr.as_str(), attr.start().to_u32()));
                }
            }
            _ => {}
        }
        walk_expr(self, expr);
    }
}

fn scope_items(body: &[Stmt]) -> ScopeItems<'_> {
    let mut items = ScopeItems::default();
    items.visit_body(body);
    items
}

impl ScopeItems<'_> {
    /// Whether relationship `name` of the instance in variable `var` is loaded when the body
    /// ends: read (`len(var.name)`) or refreshed by name (`db.refresh(var, attribute_names=[...])`)
    /// after anything that may expire it and after the variable is last assigned.
    fn preloads(&self, var: &str, name: &str) -> bool {
        let refreshes = self.calls.iter().filter(|c| {
            let names = c.arguments.find_keyword("attribute_names").map(|k| &k.value).or(c.arguments.args.get(1));
            let named = match names {
                Some(Expr::List(ast::ExprList { elts, .. }))
                | Some(Expr::Tuple(ast::ExprTuple { elts, .. }))
                | Some(Expr::Set(ast::ExprSet { elts, .. })) => {
                    elts.iter().any(|e| matches!(e, Expr::StringLiteral(s) if s.value.to_str() == name))
                }
                _ => false,
            };
            called_name(c) == Some("refresh") && named && first_argument_is(c, var)
        });
        let loaded = self
            .reads
            .iter()
            .filter(|(v, attribute, _)| *v == var && *attribute == name)
            .map(|(_, _, at)| *at)
            .chain(refreshes.map(|c| c.start().to_u32()))
            .max();
        let unloaded = self
            .calls
            .iter()
            .filter(|c| expires(c, var))
            .map(|c| c.start().to_u32())
            .chain(self.stores.iter().filter(|(v, _)| *v == var).map(|(_, at)| *at))
            .max();
        loaded.is_some_and(|loaded| unloaded.is_none_or(|unloaded| loaded > unloaded))
    }
}

/// Whether a call may expire the loaded attributes of the instance in variable `var`:
/// `commit()` (`expire_on_commit`), `rollback()`, `expire_all()`, `expire(var)`, and
/// `refresh(var)` without `attribute_names` (which leaves lazy relationships unloaded).
fn expires(call: &ast::ExprCall, var: &str) -> bool {
    match called_name(call) {
        Some("commit" | "rollback" | "expire_all") => true,
        Some("expire") => call.arguments.args.is_empty() || first_argument_is(call, var),
        Some("refresh") => {
            first_argument_is(call, var)
                && call.arguments.args.len() < 2
                && call.arguments.find_keyword("attribute_names").is_none()
        }
        _ => false,
    }
}

fn first_argument_is(call: &ast::ExprCall, var: &str) -> bool {
    matches!(call.arguments.args.first(), Some(Expr::Name(n)) if n.id.as_str() == var)
}

/// Whether an `except` clause catches `RuntimeError`: bare, `RuntimeError` or a base of it, or
/// a tuple naming one of those.
fn catches_runtime_error(type_: Option<&Expr>) -> bool {
    let named = |e: &Expr| {
        matches!(e, Expr::Name(n) if matches!(n.id.as_str(), "RuntimeError" | "Exception" | "BaseException"))
    };
    match type_ {
        None => true,
        Some(Expr::Tuple(tuple)) => tuple.elts.iter().any(named),
        Some(e) => named(e),
    }
}

/// The call expression spanning exactly `range` in a module.
fn find_call(module: &ast::ModModule, range: TextRange) -> Option<&ast::ExprCall> {
    struct Finder<'a> {
        range: TextRange,
        found: Option<&'a ast::ExprCall>,
    }
    impl<'a> SourceOrderVisitor<'a> for Finder<'a> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if self.found.is_some() || !expr.range().contains_range(self.range) {
                return;
            }
            if let Expr::Call(call) = expr
                && call.range() == self.range
            {
                self.found = Some(call);
                return;
            }
            walk_expr(self, expr);
        }
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if self.found.is_none() && stmt.range().contains_range(self.range) {
                walk_stmt(self, stmt);
            }
        }
    }
    let mut finder = Finder { range, found: None };
    finder.visit_body(&module.body);
    finder.found
}

/// The function whose name starts at `offset`.
fn find_function(module: &ast::ModModule, offset: u32) -> Option<&ast::StmtFunctionDef> {
    struct Finder<'a> {
        offset: u32,
        found: Option<&'a ast::StmtFunctionDef>,
    }
    impl<'a> SourceOrderVisitor<'a> for Finder<'a> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if self.found.is_some() || !stmt.range().contains(ruff_text_size::TextSize::new(self.offset)) {
                return;
            }
            if let Stmt::FunctionDef(def) = stmt
                && def.name.start().to_u32() == self.offset
            {
                self.found = Some(def);
                return;
            }
            walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, _: &'a Expr) {}
    }
    let mut finder = Finder { offset, found: None };
    finder.visit_body(&module.body);
    finder.found
}

/// The values a function body returns (in its own scope).
/// Whether a body cannot complete without a `return` (or `raise`), and has no bare `return`.
fn ends_in_return(body: &[Stmt]) -> bool {
    struct Bare(bool);
    impl<'a> SourceOrderVisitor<'a> for Bare {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
                Stmt::Return(ast::StmtReturn { value: None, .. }) => self.0 = true,
                _ => walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, _: &'a Expr) {}
    }
    fn ends(body: &[Stmt]) -> bool {
        match body.last() {
            Some(Stmt::Return(_) | Stmt::Raise(_)) => true,
            Some(Stmt::If(stmt)) => {
                ends(&stmt.body)
                    && stmt.elif_else_clauses.last().is_some_and(|c| c.test.is_none())
                    && stmt.elif_else_clauses.iter().all(|c| ends(&c.body))
            }
            Some(Stmt::With(stmt)) => ends(&stmt.body),
            Some(Stmt::Try(stmt)) => {
                (ends(&stmt.body) || ends(&stmt.orelse)) && stmt.handlers.iter().all(|h| match h {
                    ast::ExceptHandler::ExceptHandler(h) => ends(&h.body),
                })
            }
            _ => false,
        }
    }
    let mut bare = Bare(false);
    bare.visit_body(body);
    !bare.0 && ends(body)
}

fn return_values(body: &[Stmt]) -> Vec<&Expr> {
    struct Returns<'a>(Vec<&'a Expr>);
    impl<'a> SourceOrderVisitor<'a> for Returns<'a> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
                Stmt::Return(ast::StmtReturn { value: Some(value), .. }) => self.0.push(value),
                _ => walk_stmt(self, stmt),
            }
        }
        fn visit_expr(&mut self, _: &'a Expr) {}
    }
    let mut returns = Returns(Vec::new());
    returns.visit_body(body);
    returns.0
}
