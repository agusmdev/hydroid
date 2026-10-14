//! Per-file index of function and class definitions: qualified names, parameters, nesting.
//! Built for project files and for every file a call resolves into.

use hydroid_core::catalog::StateNamespace;
use hydroid_core::facts::{Accessor, FunctionKind, Location, Origin};
use ruff_db::files::{File, FilePath};
use ruff_db::parsed::parsed_module;
use ruff_db::source::{line_index, source_text};
use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, walk_expr, walk_stmt};
use ruff_python_ast::{self as ast, Expr, Stmt};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextSize};
use rustc_hash::{FxHashMap, FxHashSet};
use ty_python_core::definition::DefinitionKind;
use ty_python_semantic::types::ide_support::ResolvedDefinition;
use ty_python_semantic::types::{Type, TypeDefinition};
use ty_python_semantic::{HasType, ImportAliasResolution, SemanticModel, TypeQualifiers, definitions_for_name};

use crate::db::HydroidDb;

/// Key of the module-level pseudo-function in [`FileIndex::functions`].
pub const MODULE_KEY: u32 = u32::MAX;

/// A definition, identified by its file and the offset of its name (lambdas: of `lambda`).
pub type Key = (File, u32);

pub struct FnEntry {
    pub qualname: String,
    pub kind: FunctionKind,
    pub location: Location,
    pub is_async: bool,
    pub property: Option<Accessor>,
    pub is_generator: bool,
    /// See [`hydroid_core::facts::Function::validates`] (project files only).
    pub validates: Vec<String>,
    pub params: Vec<String>,
    /// How many of `params` can be passed positionally.
    pub positional: usize,
    pub parent: Option<u32>,
    pub class: Option<u32>,
    /// See [`hydroid_core::facts::Function::self_calls`] (library methods only).
    pub self_calls: Vec<String>,
    /// See [`hydroid_core::facts::Function::closure_self_calls`] (library methods only).
    pub closure_self_calls: Vec<String>,
    /// See [`hydroid_core::facts::Function::called_params`] (library functions only).
    pub called_params: Vec<String>,
    /// See [`hydroid_core::facts::Function::stored_params`] (library methods only).
    pub stored_params: Vec<(String, String)>,
    /// See [`hydroid_core::facts::Function::called_attributes`] (library methods only).
    pub called_attributes: Vec<String>,
}

pub struct ClassEntry {
    pub qualname: String,
    /// Project classes only: resolving bases runs type inference on the module, which is costly
    /// on large libraries. Library bases are resolved on demand, see [`class_bases`].
    pub bases: Vec<Key>,
    pub is_protocol: bool,
    pub methods: Vec<(String, u32)>,
    /// Classes named in the annotations of class-level fields (project classes only).
    pub fields: Vec<Key>,
    /// SQLAlchemy relationships loaded lazily on first access (`relationship(...)` with the
    /// default `lazy="select"`), project classes only.
    pub lazy_relationships: Vec<String>,
    /// Decorated, or given keywords (`metaclass=...`): something may register (and instantiate)
    /// it without naming it.
    pub registered: bool,
}

/// `<namespace>.name = value` (`app.state.name = value`), or `"name": value` in a dict a
/// lifespan yields, for the catalog `[[state_namespace]]` at index `namespace`. `class` is the
/// class of the value (`None`: unknown).
pub struct StateStore {
    pub namespace: usize,
    pub name: String,
    pub class: Option<Key>,
}

pub struct FileIndex {
    pub file: File,
    pub origin: Origin,
    pub functions: FxHashMap<u32, FnEntry>,
    pub classes: FxHashMap<u32, ClassEntry>,
    /// Lines with a `# hydroid: ignore` comment (project files only).
    pub suppressed: Vec<u32>,
    /// Class attributes assigned the result of a call (`flags = FromFile(...)`): the only ones
    /// that can hold a descriptor.
    pub class_attributes: Vec<String>,
    /// Objects stored on application or connection state (project files only).
    pub state_stores: Vec<StateStore>,
    /// Relationship names each function names in an eager loader option
    /// (`selectinload(Author.books)`, `joinedload("books")`; `*` for wildcards), project files
    /// only: what it loads is not loaded lazily later.
    pub eager_loads: FxHashMap<u32, Vec<String>>,
    /// Attributes whose methods the methods of a generic class call (`self.repo.list_ids()` in
    /// `class Service(Generic[RepoT])`), project files only: an instance's type may specialize
    /// them (see [`hydroid_core::facts::Instance::binds`]).
    pub generic_attributes: Vec<String>,
    /// Names rebound where a variable's own definitions do not show it (project files only):
    /// declared `global` or `nonlocal`, or assigned as an attribute of something other than
    /// `self` (`module.client = ...`).
    pub rebound: Vec<String>,
    /// The names the module uses (project files only), see [`used_names`].
    pub used_names: FxHashSet<String>,
    display_path: String,
    lines: LineIndex,
    source: ruff_db::source::SourceText,
}

impl FileIndex {
    pub fn build(db: &HydroidDb, file: File, namespaces: &[StateNamespace]) -> Self {
        let (module, origin, display_path) = describe(db, file);
        let program_file = ty_python_semantic::Db::program_file(db, file);
        let model = SemanticModel::new(db, program_file);
        let parsed = parsed_module(db, model.python_file()).load(db);
        let source = source_text(db, file);
        let lines = line_index(db, file);
        let suppressed = if origin == Origin::Project {
            source
                .as_str()
                .lines()
                .enumerate()
                .filter(|(_, line)| line.contains("# hydroid: ignore"))
                .map(|(i, _)| i as u32 + 1)
                .collect()
        } else {
            Vec::new()
        };
        let mut index = Self {
            file,
            origin,
            functions: FxHashMap::default(),
            classes: FxHashMap::default(),
            suppressed,
            class_attributes: Vec::new(),
            state_stores: Vec::new(),
            eager_loads: FxHashMap::default(),
            generic_attributes: Vec::new(),
            rebound: Vec::new(),
            used_names: if origin == Origin::Project { used_names(&parsed.syntax().body) } else { FxHashSet::default() },
            display_path,
            lines,
            source,
        };
        let module_entry = FnEntry {
            qualname: format!("{module}.<module>"),
            kind: FunctionKind::Module,
            location: index.location(TextSize::new(0)),
            is_async: false,
            property: None,
            is_generator: false,
            validates: Vec::new(),
            params: Vec::new(),
            positional: 0,
            parent: None,
            class: None,
            self_calls: Vec::new(),
            closure_self_calls: Vec::new(),
            called_params: Vec::new(),
            stored_params: Vec::new(),
            called_attributes: Vec::new(),
        };
        index.functions.insert(MODULE_KEY, module_entry);
        let mut builder = Builder { db, index: &mut index, model: &model, namespaces, scopes: Vec::new(), module };
        builder.visit_body(&parsed.syntax().body);
        index
    }

    pub fn location(&self, offset: TextSize) -> Location {
        let lc = self.lines.line_column(offset, self.source.as_str());
        Location {
            path: self.display_path.clone(),
            line: lc.line.get() as u32,
            column: lc.column.get() as u32,
        }
    }

    pub fn text(&self, range: ruff_text_size::TextRange) -> &str {
        &self.source.as_str()[range]
    }
}

/// Module name, origin and display path of a file.
pub(crate) fn describe(db: &HydroidDb, file: File) -> (String, Origin, String) {
    let layout = db.layout();
    let module_of = |relative: &str| {
        let stem = relative.trim_end_matches(".pyi").trim_end_matches(".py");
        let stem = stem.strip_suffix("/__init__").unwrap_or(stem);
        stem.replace('/', ".")
    };
    match file.path(db) {
        FilePath::Vendored(path) => {
            let relative = path.as_str().strip_prefix("stdlib/").unwrap_or(path.as_str());
            (module_of(relative), Origin::Stdlib, path.as_str().to_string())
        }
        FilePath::System(path) => {
            if let Some(relative) = layout.site_packages.iter().find_map(|sp| path.strip_prefix(sp).ok()) {
                return (module_of(relative.as_str()), Origin::Library, path.to_string());
            }
            let Ok(in_project) = path.strip_prefix(&layout.root) else {
                let stem = path.file_stem().unwrap_or_default().to_string();
                return (stem, Origin::Library, path.to_string());
            };
            let relative = layout
                .src_roots
                .iter()
                .rev() // `src/` before the root
                .find_map(|root| path.strip_prefix(root).ok())
                .unwrap_or(in_project);
            (module_of(relative.as_str()), Origin::Project, in_project.as_str().replace('\\', "/"))
        }
        FilePath::SystemVirtual(path) => (path.to_string(), Origin::Project, path.to_string()),
    }
}

enum Scope {
    Function(u32),
    Class(u32),
}

struct Builder<'a, 'db> {
    db: &'a HydroidDb,
    index: &'a mut FileIndex,
    model: &'a SemanticModel<'db>,
    namespaces: &'a [StateNamespace],
    scopes: Vec<(Scope, String)>,
    module: String,
}

impl Builder<'_, '_> {
    fn prefix(&self) -> String {
        match self.scopes.last() {
            None => self.module.clone(),
            Some((Scope::Function(_), q)) => format!("{q}.<locals>"),
            Some((Scope::Class(_), q)) => q.clone(),
        }
    }

    fn enclosing_function(&self) -> Option<u32> {
        self.scopes.iter().rev().find_map(|(s, _)| match s {
            Scope::Function(k) => Some(*k),
            Scope::Class(_) => None,
        })
    }

    fn direct_class(&self) -> Option<u32> {
        match self.scopes.last() {
            Some((Scope::Class(k), _)) => Some(*k),
            _ => None,
        }
    }

    /// `<namespace>.<name> = value`: see [`StateStore`].
    fn state_store(&mut self, target: &Expr, value: &Expr) {
        let Expr::Attribute(attr) = target else { return };
        if value.is_none_literal_expr() {
            return;
        }
        let Some(namespace) = namespace_of(self.db, self.model, self.namespaces, &attr.value) else { return };
        let class = instance_class(self.model, value);
        self.index.state_stores.push(StateStore { namespace, name: attr.attr.to_string(), class });
    }

    /// `yield {"name": value}` in the lifespan given to an application's constructor (catalog
    /// `[[state_namespace]]` `lifespan`): the framework copies the dict into the namespace.
    fn lifespan_state(&mut self, call: &ast::ExprCall) {
        for (namespace, group) in self.namespaces.iter().enumerate() {
            let Some(lifespan) = &group.lifespan else { continue };
            let Some(keyword) = call.arguments.find_keyword(&lifespan.keyword) else { continue };
            let Expr::Name(name) = &keyword.value else { continue };
            let constructs = match call.func.inferred_type(self.model) {
                Some(ty @ Type::ClassLiteral(_)) => class_of(self.model, ty),
                _ => None,
            };
            if !constructs.is_some_and(|class| inherits_any(self.db, class, &lifespan.owners)) {
                continue;
            }
            let db = self.model.db();
            let definitions =
                definitions_for_name(self.model, &name.id, (&keyword.value).into(), ImportAliasResolution::ResolveAliases);
            for definition in definitions {
                let ResolvedDefinition::Definition(def) = definition else { continue };
                let DefinitionKind::Function(function) = def.kind(db) else { continue };
                let module = parsed_module(db, def.python_file(db)).load(db);
                let model = SemanticModel::new(db, ty_python_semantic::Db::program_file(db, def.file(db)));
                for value in yield_values(&function.node(&module).body) {
                    let Expr::Dict(dict) = value else { continue };
                    for item in &dict.items {
                        let Some(Expr::StringLiteral(key)) = &item.key else { continue };
                        let class = instance_class(&model, &item.value);
                        let name = key.value.to_str().to_string();
                        self.index.state_stores.push(StateStore { namespace, name, class });
                    }
                }
            }
        }
    }

    /// `name = relationship(...)` (or `name: Mapped[...] = relationship(...)`) in the body of a
    /// project class, loaded lazily: see [`ClassEntry::lazy_relationships`].
    fn relationship(&mut self, name: &ast::ExprName, value: &Expr) {
        let Some(class) = self.direct_class() else { return };
        let Expr::Call(call) = value else { return };
        if self.index.origin != Origin::Project
            || !matches!(callee_name(&call.func), Some("relationship"))
            || !defined_in_sqlalchemy(self.model, &call.func)
        {
            return;
        }
        // `lazy="selectin"`, `"joined"`, `"raise"`, `"noload"`, `"dynamic"`, `"write_only"`, ...:
        // reading the attribute emits no query (or not by itself).
        let lazy = match call.arguments.find_keyword("lazy").map(|k| &k.value) {
            None => true,
            Some(Expr::StringLiteral(s)) => s.value.to_str() == "select",
            Some(Expr::BooleanLiteral(b)) => b.value,
            Some(_) => false,
        };
        if lazy && let Some(entry) = self.index.classes.get_mut(&class) {
            entry.lazy_relationships.push(name.id.to_string());
        }
    }

    /// `selectinload(Author.books)`, `joinedload("books")`, `Load(Author).selectinload(...)`:
    /// see [`FileIndex::eager_loads`].
    fn eager_load(&mut self, call: &ast::ExprCall) {
        if !callee_name(&call.func).is_some_and(|n| EAGER_LOADERS.contains(&n)) {
            return;
        }
        // Module-level options (`AUTHOR_OPTIONS = [selectinload(...)]`) may be used anywhere in
        // the file.
        let function = self.enclosing_function().unwrap_or(MODULE_KEY);
        let mut names = Vec::new();
        for argument in &call.arguments.args {
            match argument {
                Expr::Attribute(attr) => names.push(attr.attr.to_string()),
                // `Author.books.of_type(Novel)`, `Author.books.and_(...)`.
                Expr::Call(inner) => {
                    if let Expr::Attribute(method) = &*inner.func
                        && let Expr::Attribute(attr) = &*method.value
                    {
                        names.push(attr.attr.to_string());
                    }
                }
                Expr::StringLiteral(s) => names.extend(s.value.to_str().split('.').map(str::to_string)),
                _ => {}
            }
        }
        if !names.is_empty() {
            self.index.eager_loads.entry(function).or_default().extend(names);
        }
    }

    fn params(parameters: &ast::Parameters) -> (Vec<String>, usize) {
        let positional: Vec<String> = parameters
            .posonlyargs
            .iter()
            .chain(&parameters.args)
            .map(|p| p.parameter.name.to_string())
            .collect();
        let count = positional.len();
        let rest = parameters
            .vararg
            .iter()
            .map(|p| p.name.to_string())
            .chain(parameters.kwonlyargs.iter().map(|p| p.parameter.name.to_string()))
            .chain(parameters.kwarg.iter().map(|p| p.name.to_string()));
        (positional.into_iter().chain(rest).collect(), count)
    }

}

fn resolve_base(model: &SemanticModel<'_>, base: &Expr) -> Option<Key> {
    class_of(model, base.inferred_type(model)?)
}

/// Whether the name an annotated assignment in a class body declares is a class variable
/// (`ClassVar[...]`), as ty reads its type qualifiers.
pub fn is_class_variable(model: &SemanticModel<'_>, target: &Expr) -> bool {
    let Expr::Name(name) = target else { return false };
    model.type_qualifiers(ast::ExprRef::Name(name)).contains(TypeQualifiers::CLASS_VAR)
}

/// The class a class object or an instance type is defined by.
pub fn class_of<'db>(model: &SemanticModel<'db>, ty: Type<'db>) -> Option<Key> {
    let db = model.db();
    let def = match ty.definition(db, &model.program_environment())? {
        TypeDefinition::StaticClass(def) | TypeDefinition::DynamicClass(def) => def,
        _ => return None,
    };
    let module = parsed_module(db, def.python_file(db)).load(db);
    let range = def.focus_range(db, &module);
    Some((range.file(), range.start().to_u32()))
}

/// The definition of the class whose name starts at `offset`.
fn find_class(module: &ast::ModModule, offset: u32) -> Option<&ast::StmtClassDef> {
    struct Finder<'a> {
        offset: u32,
        found: Option<&'a ast::StmtClassDef>,
    }
    impl<'a> SourceOrderVisitor<'a> for Finder<'a> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if self.found.is_some() || !stmt.range().contains(TextSize::new(self.offset)) {
                return;
            }
            if let Stmt::ClassDef(def) = stmt
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

/// The bases of the class whose name starts at `class.1` in `class.0`.
pub fn class_bases(db: &HydroidDb, class: Key) -> Vec<Key> {
    let model = SemanticModel::new(db, ty_python_semantic::Db::program_file(db, class.0));
    let parsed = parsed_module(db, model.python_file()).load(db);
    let Some(def) = find_class(parsed.syntax(), class.1) else { return Vec::new() };
    def.bases().iter().filter_map(|b| resolve_base(&model, b)).collect()
}

/// The method `name` an instance of `class` has: defined in the class or inherited (bases
/// searched depth-first, which is the MRO without diamonds).
pub fn class_method(db: &HydroidDb, class: Key, name: &str) -> Option<Key> {
    let (mut stack, mut seen) = (vec![class], rustc_hash::FxHashSet::default());
    while let Some(class) = stack.pop() {
        if !seen.insert(class) || seen.len() > 32 {
            continue;
        }
        let parsed = parsed_module(db, ty_python_semantic::Db::program_file(db, class.0).python_file(db)).load(db);
        let def = find_class(parsed.syntax(), class.1)?;
        let method = def.body.iter().find_map(|stmt| match stmt {
            Stmt::FunctionDef(f) if f.name.as_str() == name => Some(f.name.start().to_u32()),
            _ => None,
        });
        if let Some(method) = method {
            return Some((class.0, method));
        }
        stack.extend(class_bases(db, class).into_iter().rev());
    }
    None
}

/// The class an expression's value is an instance of (`T | None`: `T`), if it is one class.
pub fn instance_class(model: &SemanticModel<'_>, expr: &Expr) -> Option<Key> {
    let db = model.db();
    let types = match expr.inferred_type(model)? {
        Type::Union(union) => union.elements(db).to_vec(),
        ty => vec![ty],
    };
    let mut class = None;
    for ty in types.into_iter().filter(|t| !t.is_none(db)) {
        if !matches!(ty, Type::NominalInstance(_)) {
            return None;
        }
        let key = class_of(model, ty)?;
        if class.is_some_and(|c| c != key) {
            return None;
        }
        class = Some(key);
    }
    class
}

/// Whether `class` inherits from `base` (through indexed bases).
pub fn inherits(db: &HydroidDb, class: Key, base: Key) -> bool {
    let (mut queue, mut seen) = (vec![class], rustc_hash::FxHashSet::default());
    while let Some(class) = queue.pop() {
        if class == base {
            return true;
        }
        if seen.insert(class) && seen.len() <= 32 {
            queue.extend(class_bases(db, class));
        }
    }
    false
}

/// The fully qualified name (`module.Class`) of a top-level class, and its bases.
fn qualname_and_bases(db: &HydroidDb, class: Key) -> Option<(String, Vec<Key>)> {
    Some((class_qualname(db, class)?, class_bases(db, class)))
}

/// The fully qualified name (`module.Class`) of a top-level class.
pub fn class_qualname(db: &HydroidDb, class: Key) -> Option<String> {
    let (module, _, _) = describe(db, class.0);
    let source = source_text(db, class.0);
    let name: String =
        source.as_str().get(class.1 as usize..)?.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    Some(format!("{module}.{name}"))
}

/// Whether `class` is one of the top-level classes `qualnames` or inherits from one.
pub fn inherits_any(db: &HydroidDb, class: Key, qualnames: &[String]) -> bool {
    let (mut queue, mut seen) = (vec![class], rustc_hash::FxHashSet::default());
    while let Some(class) = queue.pop() {
        if !seen.insert(class) || seen.len() > 32 {
            continue;
        }
        let Some((name, bases)) = qualname_and_bases(db, class) else { continue };
        if qualnames.contains(&name) {
            return true;
        }
        queue.extend(bases);
    }
    false
}

/// Whether `expr` is an instance of `owners` (or of a subclass): by its type, or as what the
/// `app` attribute of another namespace's owner gives (`request.app`).
pub fn is_owner(db: &HydroidDb, model: &SemanticModel<'_>, namespaces: &[StateNamespace], expr: &Expr, owners: &[String]) -> bool {
    if let Expr::Attribute(attr) = expr {
        let gives = |n: &StateNamespace| {
            n.app.as_ref().is_some_and(|a| a.attribute == attr.attr.as_str() && a.owners.iter().any(|o| owners.contains(o)))
        };
        if namespaces.iter().any(|n| gives(n) && is_owner(db, model, namespaces, &attr.value, &n.owners)) {
            return true;
        }
    }
    instance_class(model, expr).is_some_and(|class| inherits_any(db, class, owners))
}

/// The catalog `[[state_namespace]]` (by index) `expr` is: `<owner>.<attribute>`, or an owner
/// itself for a namespace without `attribute`. Untyped owners have none.
pub fn namespace_of(db: &HydroidDb, model: &SemanticModel<'_>, namespaces: &[StateNamespace], expr: &Expr) -> Option<usize> {
    namespaces.iter().position(|n| match (&n.attribute, expr) {
        (Some(attribute), Expr::Attribute(attr)) => {
            attr.attr.as_str() == attribute && is_owner(db, model, namespaces, &attr.value, &n.owners)
        }
        (Some(_), _) => false,
        (None, _) => is_owner(db, model, namespaces, expr, &n.owners),
    })
}

/// The last name of a decorator: `property`, `setter` (`@x.setter`), `field_validator` (`@field_validator("x")`).
fn decorator_name(decorator: &ast::Decorator) -> Option<&str> {
    let expr = match &decorator.expression {
        Expr::Call(call) => &*call.func,
        expr => expr,
    };
    match expr {
        Expr::Name(name) => Some(name.id.as_str()),
        Expr::Attribute(attr) => Some(attr.attr.as_str()),
        _ => None,
    }
}

/// SQLAlchemy loader options that load a relationship up front (or keep it from loading).
const EAGER_LOADERS: &[&str] =
    &["selectinload", "joinedload", "subqueryload", "immediateload", "contains_eager", "raiseload", "noload"];

/// `relationship` in `relationship(...)` and `orm.relationship(...)`.
fn callee_name(func: &Expr) -> Option<&str> {
    match func {
        Expr::Name(name) => Some(name.id.as_str()),
        Expr::Attribute(attr) => Some(attr.attr.as_str()),
        _ => None,
    }
}

/// Whether a callee is a function defined in the `sqlalchemy` package.
fn defined_in_sqlalchemy(model: &SemanticModel<'_>, func: &Expr) -> bool {
    let db = model.db();
    let Some(ty @ Type::FunctionLiteral(_)) = func.inferred_type(model) else { return false };
    let Some(TypeDefinition::Function(def)) = ty.definition(db, &model.program_environment()) else { return false };
    let path = def.file(db).path(db).as_str().replace('\\', "/");
    path.contains("/sqlalchemy/")
}

/// The attributes a SQLAlchemy `@validates("a", "b")` decorator names.
fn orm_validates(model: &SemanticModel<'_>, def: &ast::StmtFunctionDef) -> Vec<String> {
    let mut names = Vec::new();
    for decorator in &def.decorator_list {
        let Expr::Call(call) = &decorator.expression else { continue };
        if callee_name(&call.func) != Some("validates") || !defined_in_sqlalchemy(model, &call.func) {
            continue;
        }
        for argument in &call.arguments.args {
            if let Expr::StringLiteral(s) = argument {
                names.push(s.value.to_str().to_string());
            }
        }
    }
    names
}

fn accessor(def: &ast::StmtFunctionDef) -> Option<Accessor> {
    def.decorator_list.iter().find_map(|d| match decorator_name(d)? {
        "property" | "cached_property" => Some(Accessor::Get),
        "setter" => Some(Accessor::Set),
        "deleter" => Some(Accessor::Delete),
        _ => None,
    })
}

/// The classes an annotation names (`Item`, `list[Item] | None`, `Annotated[Item, Body()]`),
/// outside the standard library. Calls in the annotation (`Annotated` metadata) are handed to
/// `call` instead.
pub fn annotation_classes<'e>(
    model: &SemanticModel<'_>,
    annotation: &'e Expr,
    classes: &mut Vec<Key>,
    call: &mut dyn FnMut(&'e ast::ExprCall),
) {
    match annotation {
        Expr::Name(_) | Expr::Attribute(_) => {
            let db = model.db();
            // Names in a type expression have the instance type (`Item`), in a value expression
            // (`response_model=Item`) the class type.
            let Some(ty) = annotation.inferred_type(model) else { return };
            if !is_class_object(&ty) && !matches!(ty, Type::NominalInstance(_)) {
                return;
            }
            let Some(TypeDefinition::StaticClass(def)) = ty.definition(db, &model.program_environment()) else {
                return;
            };
            let file = def.file(db);
            if file.path(db).is_vendored_path() {
                return;
            }
            let module = parsed_module(db, def.python_file(db)).load(db);
            let key = (file, def.focus_range(db, &module).start().to_u32());
            if !classes.contains(&key) {
                classes.push(key);
            }
        }
        Expr::Subscript(subscript) => {
            annotation_classes(model, &subscript.value, classes, call);
            annotation_classes(model, &subscript.slice, classes, call);
        }
        Expr::Tuple(tuple) => {
            for element in &tuple.elts {
                annotation_classes(model, element, classes, call);
            }
        }
        Expr::BinOp(binop) => {
            annotation_classes(model, &binop.left, classes, call);
            annotation_classes(model, &binop.right, classes, call);
        }
        Expr::Call(c) => call(c),
        _ => {}
    }
}

/// The values a generator body yields (in its own scope; not `yield from`).
pub fn yield_values(body: &[Stmt]) -> Vec<&Expr> {
    struct Yields<'a>(Vec<&'a Expr>);
    impl<'a> SourceOrderVisitor<'a> for Yields<'a> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
                walk_stmt(self, stmt);
            }
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Yield(ast::ExprYield { value: Some(value), .. }) => {
                    self.0.push(value);
                    walk_expr(self, value);
                }
                Expr::Lambda(_) => {}
                _ => walk_expr(self, expr),
            }
        }
    }
    let mut yields = Yields(Vec::new());
    yields.visit_body(body);
    yields.0
}

/// The methods an instance method calls on its own instance (`self.name(...)`), in its own
/// scope: not in nested functions, lambdas or classes, which may run elsewhere.
/// `X` of the `self.X.name(...)` calls in the methods of a class.
fn attribute_calls(class: &ast::StmtClassDef) -> Vec<String> {
    struct Finder {
        names: Vec<String>,
    }
    impl<'ast> SourceOrderVisitor<'ast> for Finder {
        fn visit_expr(&mut self, expr: &'ast Expr) {
            if let Expr::Call(call) = expr
                && let Expr::Attribute(method) = &*call.func
                && let Expr::Attribute(attr) = &*method.value
                && matches!(&*attr.value, Expr::Name(n) if n.id.as_str() == "self")
                && !self.names.iter().any(|n| n == attr.attr.as_str())
            {
                self.names.push(attr.attr.to_string());
            }
            walk_expr(self, expr);
        }
    }
    let mut finder = Finder { names: Vec::new() };
    finder.visit_body(&class.body);
    finder.names
}

/// What a library function calls of what it is given, to construct it (`route_class(...)`): the
/// parameters it calls, the parameters a method stores on its instance (`self.route_class =
/// route_class`), and the attributes of its instance it calls (`self.route_class(...)`), directly
/// or through a local name assigned from them (`cls = override or self.route_class; cls(...)`).
/// Only in its own scope, not in nested functions or classes.
fn constructions(def: &ast::StmtFunctionDef, is_method: bool) -> (Vec<String>, Vec<(String, String)>, Vec<String>) {
    struct Finder<'a> {
        receiver: Option<&'a str>,
        params: Vec<&'a str>,
        /// What each local name was assigned from: `(parameters, attributes)`.
        locals: Vec<(&'a str, Vec<String>, Vec<String>)>,
        called_params: Vec<String>,
        stored: Vec<(String, String)>,
        called_attributes: Vec<String>,
    }
    impl<'a> Finder<'a> {
        /// The parameters and instance attributes an expression reads.
        fn sources(&self, expr: &'a Expr) -> (Vec<String>, Vec<String>) {
            struct Reads<'f, 'a>(&'f Finder<'a>, Vec<String>, Vec<String>);
            impl<'b> SourceOrderVisitor<'b> for Reads<'_, '_> {
                fn visit_expr(&mut self, expr: &'b Expr) {
                    match expr {
                        Expr::Name(n) if self.0.params.contains(&n.id.as_str()) => self.1.push(n.id.to_string()),
                        Expr::Attribute(a) if self.0.is_receiver(&a.value) => self.2.push(a.attr.to_string()),
                        Expr::Lambda(_) => return,
                        _ => {}
                    }
                    walk_expr(self, expr);
                }
            }
            let mut reads = Reads(self, Vec::new(), Vec::new());
            reads.visit_expr(expr);
            (reads.1, reads.2)
        }

        fn is_receiver(&self, expr: &Expr) -> bool {
            matches!(expr, Expr::Name(n) if Some(n.id.as_str()) == self.receiver)
        }
    }
    impl<'a> SourceOrderVisitor<'a> for Finder<'a> {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => return,
                Stmt::Assign(ast::StmtAssign { targets, value, .. }) => match &targets[..] {
                    [Expr::Name(name)] => {
                        let (params, attributes) = self.sources(value);
                        self.locals.push((name.id.as_str(), params, attributes));
                    }
                    [Expr::Attribute(attr)] if self.is_receiver(&attr.value) => {
                        if let Expr::Name(value) = &**value
                            && self.params.contains(&value.id.as_str())
                        {
                            self.stored.push((value.id.to_string(), attr.attr.to_string()));
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
            walk_stmt(self, stmt);
        }

        fn visit_expr(&mut self, expr: &'a Expr) {
            if let Expr::Call(call) = expr {
                match &*call.func {
                    Expr::Name(n) if self.params.contains(&n.id.as_str()) => self.called_params.push(n.id.to_string()),
                    Expr::Name(n) => {
                        if let Some((_, params, attributes)) = self.locals.iter().rev().find(|(l, _, _)| *l == n.id.as_str()) {
                            let (params, attributes) = (params.clone(), attributes.clone());
                            self.called_params.extend(params);
                            self.called_attributes.extend(attributes);
                        }
                    }
                    Expr::Attribute(a) if self.is_receiver(&a.value) => self.called_attributes.push(a.attr.to_string()),
                    _ => {}
                }
            }
            if !matches!(expr, Expr::Lambda(_)) {
                walk_expr(self, expr);
            }
        }
    }
    let receiver = if is_method { receiver_name(def) } else { None };
    let params = def.parameters.iter_non_variadic_params().map(|p| p.parameter.name.as_str()).filter(|p| Some(*p) != receiver).collect();
    let mut finder = Finder {
        receiver,
        params,
        locals: Vec::new(),
        called_params: Vec::new(),
        stored: Vec::new(),
        called_attributes: Vec::new(),
    };
    finder.visit_body(&def.body);
    let dedup = |mut v: Vec<String>| {
        v.sort();
        v.dedup();
        v
    };
    (dedup(finder.called_params), finder.stored, dedup(finder.called_attributes))
}

/// The names a project module uses, outside the bases of its class definitions: names and
/// attributes read, and the dotted segments of its string literals (forward references, names
/// given to registries). A class whose name no module uses is never instantiated as itself.
fn used_names(body: &[Stmt]) -> FxHashSet<String> {
    struct Names(FxHashSet<String>);
    impl<'a> SourceOrderVisitor<'a> for Names {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            if let Stmt::ClassDef(def) = stmt {
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                for keyword in def.arguments.iter().flat_map(|a| a.keywords.iter()) {
                    self.visit_expr(&keyword.value);
                }
                self.visit_body(&def.body);
                return;
            }
            walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Name(name) if name.ctx == ast::ExprContext::Load => {
                    self.0.insert(name.id.to_string());
                }
                Expr::Attribute(attr) if attr.ctx == ast::ExprContext::Load => {
                    self.0.insert(attr.attr.to_string());
                }
                Expr::StringLiteral(s) => self.0.extend(s.value.to_str().split('.').map(str::to_string)),
                _ => {}
            }
            walk_expr(self, expr);
        }
    }
    let mut names = Names(FxHashSet::default());
    names.visit_body(body);
    names.0
}

fn self_calls(def: &ast::StmtFunctionDef) -> Vec<String> {
    match receiver_name(def) {
        Some(receiver) => calls_on(receiver, &def.body),
        None => Vec::new(),
    }
}

/// The methods `async def` closures defined in a method (at any depth) call on the method's
/// instance, see [`hydroid_core::facts::Function::closure_self_calls`].
fn closure_self_calls(def: &ast::StmtFunctionDef) -> Vec<String> {
    struct Closures<'a> {
        receiver: &'a str,
        names: Vec<String>,
    }
    impl<'ast> SourceOrderVisitor<'ast> for Closures<'_> {
        fn visit_stmt(&mut self, stmt: &'ast Stmt) {
            match stmt {
                Stmt::ClassDef(_) => {}
                Stmt::FunctionDef(inner) => {
                    // A closure taking a parameter of the same name has its own receiver.
                    let parameters = &inner.parameters;
                    let shadows = parameters.iter().any(|p| p.name().as_str() == self.receiver);
                    if !shadows {
                        if inner.is_async {
                            for name in calls_on(self.receiver, &inner.body) {
                                if !self.names.contains(&name) {
                                    self.names.push(name);
                                }
                            }
                        }
                        walk_stmt(self, stmt);
                    }
                }
                _ => walk_stmt(self, stmt),
            }
        }
    }
    let Some(receiver) = receiver_name(def) else { return Vec::new() };
    let mut closures = Closures { receiver, names: Vec::new() };
    closures.visit_body(&def.body);
    closures.names
}

/// The first parameter of an instance method (`self`).
fn receiver_name(def: &ast::StmtFunctionDef) -> Option<&str> {
    if def.decorator_list.iter().any(|d| matches!(decorator_name(d), Some("staticmethod" | "classmethod"))) {
        return None;
    }
    let parameters = &def.parameters;
    Some(parameters.posonlyargs.iter().chain(&parameters.args).next()?.parameter.name.as_str())
}

/// The methods `body` (not its nested functions and classes) calls on `receiver`
/// (`receiver.name(...)`).
fn calls_on(receiver: &str, body: &[Stmt]) -> Vec<String> {
    struct Finder<'a> {
        receiver: &'a str,
        names: Vec<String>,
    }
    impl<'ast> SourceOrderVisitor<'ast> for Finder<'_> {
        fn visit_stmt(&mut self, stmt: &'ast Stmt) {
            if !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
                walk_stmt(self, stmt);
            }
        }
        fn visit_expr(&mut self, expr: &'ast Expr) {
            if let Expr::Call(call) = expr
                && let Expr::Attribute(attr) = &*call.func
                && matches!(&*attr.value, Expr::Name(n) if n.id.as_str() == self.receiver)
                && !self.names.iter().any(|n| n == attr.attr.as_str())
            {
                self.names.push(attr.attr.to_string());
            }
            if !matches!(expr, Expr::Lambda(_)) {
                walk_expr(self, expr);
            }
        }
    }
    let mut finder = Finder { receiver, names: Vec::new() };
    finder.visit_body(body);
    finder.names
}

/// Whether a body yields (in its own scope: not in nested functions or classes).
pub fn yields(body: &[Stmt]) -> bool {
    struct Finder(bool);
    impl<'ast> SourceOrderVisitor<'ast> for Finder {
        fn visit_stmt(&mut self, stmt: &'ast Stmt) {
            if !self.0 && !matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)) {
                walk_stmt(self, stmt);
            }
        }
        fn visit_expr(&mut self, expr: &'ast Expr) {
            match expr {
                Expr::Yield(_) | Expr::YieldFrom(_) => self.0 = true,
                Expr::Lambda(_) => {}
                _ if !self.0 => walk_expr(self, expr),
                _ => {}
            }
        }
    }
    let mut finder = Finder(false);
    finder.visit_body(body);
    finder.0
}

fn is_protocol_base(base: &Expr) -> bool {
    let base = match base {
        Expr::Subscript(subscript) => &subscript.value,
        other => other,
    };
    match base {
        Expr::Name(name) => name.id == "Protocol",
        Expr::Attribute(attr) => attr.attr.as_str() == "Protocol",
        _ => false,
    }
}

impl<'ast> SourceOrderVisitor<'ast> for Builder<'_, '_> {
    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        if self.index.origin == Origin::Project {
            let rebound = |target: &Expr| match target {
                Expr::Attribute(attr) if !matches!(&*attr.value, Expr::Name(n) if matches!(n.id.as_str(), "self" | "cls")) => {
                    Some(attr.attr.to_string())
                }
                _ => None,
            };
            match stmt {
                Stmt::Global(ast::StmtGlobal { names, .. }) | Stmt::Nonlocal(ast::StmtNonlocal { names, .. }) => {
                    self.index.rebound.extend(names.iter().map(|n| n.to_string()));
                }
                Stmt::Assign(assign) => self.index.rebound.extend(assign.targets.iter().filter_map(rebound)),
                Stmt::AnnAssign(ast::StmtAnnAssign { target, .. }) | Stmt::AugAssign(ast::StmtAugAssign { target, .. }) => {
                    self.index.rebound.extend(rebound(target));
                }
                _ => {}
            }
            match stmt {
                Stmt::Assign(assign) => {
                    for target in &assign.targets {
                        self.state_store(target, &assign.value);
                    }
                }
                Stmt::AnnAssign(ast::StmtAnnAssign { target, value: Some(value), .. }) => {
                    self.state_store(target, value);
                }
                _ => {}
            }
        }
        match stmt {
            Stmt::FunctionDef(def) => {
                let key = def.name.start().to_u32();
                let qualname = format!("{}.{}", self.prefix(), def.name);
                let (params, positional) = Self::params(&def.parameters);
                let class = self.direct_class();
                if let Some(class) = class
                    && let Some(entry) = self.index.classes.get_mut(&class)
                {
                    entry.methods.push((def.name.to_string(), key));
                }
                let library_method = self.index.origin != Origin::Project && class.is_some();
                let (called_params, stored_params, called_attributes) = if self.index.origin != Origin::Project {
                    constructions(def, class.is_some())
                } else {
                    Default::default()
                };
                self.index.functions.insert(
                    key,
                    FnEntry {
                        qualname: qualname.clone(),
                        kind: FunctionKind::Def,
                        location: self.index.location(def.name.start()),
                        is_async: def.is_async,
                        property: accessor(def),
                        // A decorated generator is called through its decorator, which decides
                        // when the body runs (see `Call::deferred_body`).
                        is_generator: !def.is_async && def.decorator_list.is_empty() && yields(&def.body),
                        validates: if self.index.origin == Origin::Project && class.is_some() {
                            orm_validates(self.model, def)
                        } else {
                            Vec::new()
                        },
                        params,
                        positional,
                        parent: self.enclosing_function(),
                        class,
                        self_calls: if library_method { self_calls(def) } else { Vec::new() },
                        closure_self_calls: if library_method { closure_self_calls(def) } else { Vec::new() },
                        called_params,
                        stored_params,
                        called_attributes,
                    },
                );
                // Decorators, defaults and annotations are evaluated in the enclosing scope.
                for decorator in &def.decorator_list {
                    self.visit_decorator(decorator);
                }
                self.visit_parameters(&def.parameters);
                if let Some(returns) = &def.returns {
                    self.visit_annotation(returns);
                }
                self.scopes.push((Scope::Function(key), qualname));
                self.visit_body(&def.body);
                self.scopes.pop();
            }
            Stmt::ClassDef(def) => {
                let key = def.name.start().to_u32();
                let qualname = format!("{}.{}", self.prefix(), def.name);
                let bases = if self.index.origin == Origin::Project {
                    def.bases().iter().filter_map(|b| resolve_base(self.model, b)).collect()
                } else {
                    Vec::new()
                };
                let is_protocol = def.bases().iter().any(is_protocol_base);
                let generic = def.type_params.is_some() || def.bases().iter().any(Expr::is_subscript_expr);
                if self.index.origin == Origin::Project && generic {
                    for name in attribute_calls(def) {
                        if !self.index.generic_attributes.contains(&name) {
                            self.index.generic_attributes.push(name);
                        }
                    }
                }
                self.index.classes.insert(
                    key,
                    ClassEntry {
                        qualname: qualname.clone(),
                        bases,
                        is_protocol,
                        methods: Vec::new(),
                        fields: Vec::new(),
                        lazy_relationships: Vec::new(),
                        registered: !def.decorator_list.is_empty()
                            || def.arguments.as_ref().is_some_and(|a| !a.keywords.is_empty()),
                    },
                );
                self.scopes.push((Scope::Class(key), qualname));
                walk_stmt(self, stmt);
                self.scopes.pop();
            }
            Stmt::Assign(ast::StmtAssign { targets, value, .. }) if self.direct_class().is_some() => {
                if let [Expr::Name(name)] = &targets[..] {
                    self.relationship(name, value);
                }
                if value.is_call_expr() {
                    let names = targets.iter().filter_map(|t| t.as_name_expr()).map(|n| n.id.to_string());
                    self.index.class_attributes.extend(names);
                }
                walk_stmt(self, stmt);
            }
            Stmt::AnnAssign(ast::StmtAnnAssign { target, annotation, value, .. }) if self.direct_class().is_some() => {
                let class = self.direct_class().expect("in a class body");
                if value.as_deref().is_some_and(Expr::is_call_expr)
                    && let Some(name) = target.as_name_expr()
                {
                    self.index.class_attributes.push(name.id.to_string());
                    self.relationship(name, value.as_deref().expect("a call value"));
                }
                // Models neither validate nor dump class variables and private attributes.
                let is_class_var = is_class_variable(self.model, target);
                let is_private = target.as_name_expr().is_some_and(|n| n.id.starts_with('_'));
                if self.index.origin == Origin::Project && !is_class_var && !is_private {
                    let mut fields = Vec::new();
                    annotation_classes(self.model, annotation, &mut fields, &mut |_| {});
                    if let Some(entry) = self.index.classes.get_mut(&class) {
                        entry.fields.extend(fields.into_iter().filter(|f| *f != (self.index.file, class)));
                    }
                }
                walk_stmt(self, stmt);
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        if let Expr::Call(call) = expr
            && self.index.origin == Origin::Project
        {
            self.eager_load(call);
            self.lifespan_state(call);
        }
        if let Expr::Lambda(lambda) = expr {
            let key = lambda.start().to_u32();
            let location = self.index.location(lambda.start());
            let qualname = format!("{}.<lambda:{}>", self.prefix(), location.line);
            let (params, positional) = lambda.parameters.as_deref().map(Self::params).unwrap_or_default();
            self.index.functions.insert(
                key,
                FnEntry {
                    qualname: qualname.clone(),
                    kind: FunctionKind::Lambda,
                    location,
                    is_async: false,
                    property: None,
                    is_generator: false,
                    validates: Vec::new(),
                    params,
                    positional,
                    parent: self.enclosing_function(),
                    class: None,
                    self_calls: Vec::new(),
                    closure_self_calls: Vec::new(),
                    called_params: Vec::new(),
                    stored_params: Vec::new(),
                    called_attributes: Vec::new(),
                },
            );
            self.scopes.push((Scope::Function(key), qualname));
            walk_expr(self, expr);
            self.scopes.pop();
            return;
        }
        walk_expr(self, expr);
    }
}

/// Whether a type is a class object (calling it constructs an instance).
pub fn is_class_object(ty: &Type<'_>) -> bool {
    matches!(ty, Type::ClassLiteral(_) | Type::GenericAlias(_) | Type::SubclassOf(_))
}
