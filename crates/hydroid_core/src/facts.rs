//! What extraction learned about a program, as plain data.
//!
//! Extraction (`hydroid_ty`) produces [`Facts`]; analysis consumes them. Nothing in here knows
//! about ty, so the analysis can be reasoned about (and tested) as a pure graph problem.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct FnId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct ClassId(pub u32);

/// A 1-based source position. `path` is relative to the project root when inside it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Location {
    pub path: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Project,
    Library,
    Stdlib,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FunctionKind {
    Def,
    Lambda,
    /// Module or class body: runs at import time, never on the event loop, but holds
    /// registrations such as `app.add_api_route(...)`.
    Module,
}

#[derive(Clone, Debug, Serialize)]
pub struct Function {
    /// `module.Class.method`, `module.outer.<locals>.inner`, `module.<lambda:LINE>`.
    pub qualname: String,
    pub location: Location,
    pub origin: Origin,
    pub kind: FunctionKind,
    pub is_async: bool,
    pub is_property: bool,
    pub params: Vec<String>,
    /// Lexically enclosing function (closures read its parameters).
    pub parent: Option<FnId>,
    pub class: Option<ClassId>,
    /// Resolved decorator callables, outermost first.
    pub decorators: Vec<FnId>,
    /// Whether the body was analyzed. Unanalyzed functions (stdlib stubs, libraries without
    /// `--follow-libs`) are leaves: only the catalog knows whether they block.
    pub analyzed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Class {
    pub qualname: String,
    pub bases: Vec<ClassId>,
    pub is_protocol: bool,
    /// Methods defined directly in the class body.
    pub methods: Vec<(String, FnId)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Call {
    pub caller: FnId,
    pub location: Location,
    /// What the user sees: `requests.get`, `with lock`, `self.repo.load`.
    pub text: String,
    pub targets: Vec<FnId>,
    /// `obj.method(...)` on an instance: overrides in subclasses may run instead.
    pub virtual_dispatch: bool,
    /// The callee is a parameter of this or an enclosing function: `(owner, name)`.
    pub param: Option<(FnId, String)>,
    pub awaited: bool,
    /// Why nothing was resolved (only set when `targets` is empty and `param` is `None`).
    pub unresolved: Option<String>,
}

/// A callable flowing into a parameter: `to_thread(f)`, `Depends(get_db)`, `@router.get(...)`.
#[derive(Clone, Debug, Serialize)]
pub struct Flow {
    pub location: Location,
    pub caller: FnId,
    /// The callable receiving the value.
    pub into: FnId,
    pub slot: Slot,
    pub value: Value,
    /// First string-literal argument of the receiving call (route paths, event names).
    pub literal: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Slot {
    Param(String),
    /// Applied as a decorator (the decorated function flows into the first parameter).
    Decorated,
    /// Positional/keyword position that could not be mapped to a parameter name.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Value {
    Function(FnId),
    Class(ClassId),
    /// A parameter of `owner` forwarded as-is (callback plumbing).
    Param(FnId, String),
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Facts {
    pub functions: Vec<Function>,
    pub classes: Vec<Class>,
    pub calls: Vec<Call>,
    pub flows: Vec<Flow>,
    /// Lines carrying a `# hydroid: ignore` comment.
    pub suppressed: Vec<(String, u32)>,
}

impl Facts {
    pub fn function(&self, id: FnId) -> &Function {
        &self.functions[id.0 as usize]
    }

    pub fn class(&self, id: ClassId) -> &Class {
        &self.classes[id.0 as usize]
    }
}
