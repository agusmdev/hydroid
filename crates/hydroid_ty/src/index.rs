//! Per-file index of function and class definitions: qualified names, parameters, nesting.
//! Built for project files and for every file a call resolves into.

use hydroid_core::facts::{Accessor, FunctionKind, Location, Origin};
use ruff_db::files::{File, FilePath};
use ruff_db::parsed::parsed_module;
use ruff_db::source::{line_index, source_text};
use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, walk_expr, walk_stmt};
use ruff_python_ast::{self as ast, Expr, Stmt};
use ruff_source_file::LineIndex;
use ruff_text_size::{Ranged, TextSize};
use rustc_hash::FxHashMap;
use ty_python_semantic::types::{Type, TypeDefinition};
use ty_python_semantic::{HasType, SemanticModel};

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
    pub is_validator: bool,
    pub params: Vec<String>,
    /// How many of `params` can be passed positionally.
    pub positional: usize,
    pub parent: Option<u32>,
    pub class: Option<u32>,
}

pub struct ClassEntry {
    pub qualname: String,
    pub bases: Vec<Key>,
    pub is_protocol: bool,
    pub methods: Vec<(String, u32)>,
}

pub struct FileIndex {
    pub file: File,
    pub origin: Origin,
    pub functions: FxHashMap<u32, FnEntry>,
    pub classes: FxHashMap<u32, ClassEntry>,
    /// Lines with a `# hydroid: ignore` comment (project files only).
    pub suppressed: Vec<u32>,
    display_path: String,
    lines: LineIndex,
    source: ruff_db::source::SourceText,
}

impl FileIndex {
    pub fn build(db: &HydroidDb, file: File) -> Self {
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
            is_validator: false,
            params: Vec::new(),
            positional: 0,
            parent: None,
            class: None,
        };
        index.functions.insert(MODULE_KEY, module_entry);
        let mut builder = Builder { index: &mut index, model: &model, scopes: Vec::new(), module };
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
fn describe(db: &HydroidDb, file: File) -> (String, Origin, String) {
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
    index: &'a mut FileIndex,
    model: &'a SemanticModel<'db>,
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

    fn resolve_base(&self, base: &Expr) -> Option<Key> {
        let db = self.model.db();
        let ty = base.inferred_type(self.model)?;
        let def = match ty.definition(db, &self.model.program_environment())? {
            TypeDefinition::StaticClass(def) | TypeDefinition::DynamicClass(def) => def,
            _ => return None,
        };
        let module = parsed_module(db, def.python_file(db)).load(db);
        let range = def.focus_range(db, &module);
        Some((range.file(), range.start().to_u32()))
    }
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

fn accessor(def: &ast::StmtFunctionDef) -> Option<Accessor> {
    def.decorator_list.iter().find_map(|d| match decorator_name(d)? {
        "property" | "cached_property" => Some(Accessor::Get),
        "setter" => Some(Accessor::Set),
        "deleter" => Some(Accessor::Delete),
        _ => None,
    })
}

fn is_validator(def: &ast::StmtFunctionDef) -> bool {
    def.decorator_list.iter().any(|d| {
        matches!(decorator_name(d), Some("field_validator" | "model_validator" | "validator" | "root_validator"))
    })
}

/// Whether a body yields (in its own scope: not in nested functions or classes).
fn yields(body: &[Stmt]) -> bool {
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
                self.index.functions.insert(
                    key,
                    FnEntry {
                        qualname: qualname.clone(),
                        kind: FunctionKind::Def,
                        location: self.index.location(def.name.start()),
                        is_async: def.is_async,
                        property: accessor(def),
                        // A decorated generator (`@contextmanager`) is called through its
                        // decorator, which decides when the body runs.
                        is_generator: !def.is_async && def.decorator_list.is_empty() && yields(&def.body),
                        is_validator: is_validator(def),
                        params,
                        positional,
                        parent: self.enclosing_function(),
                        class,
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
                let bases = def.bases().iter().filter_map(|b| self.resolve_base(b)).collect();
                let is_protocol = def.bases().iter().any(is_protocol_base);
                self.index.classes.insert(
                    key,
                    ClassEntry { qualname: qualname.clone(), bases, is_protocol, methods: Vec::new() },
                );
                self.scopes.push((Scope::Class(key), qualname));
                walk_stmt(self, stmt);
                self.scopes.pop();
            }
            _ => walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
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
                    is_validator: false,
                    params,
                    positional,
                    parent: self.enclosing_function(),
                    class: None,
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
