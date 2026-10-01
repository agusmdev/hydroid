//! What extraction learned about a program, as plain data.
//!
//! Extraction (`hydroid_ty`) produces [`Facts`]; analysis consumes them. Nothing in here knows
//! about ty, so the analysis can be reasoned about (and tested) as a pure graph problem.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FnId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ClassId(pub u32);

/// A 1-based source position. `path` is relative to the project root when inside it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Location {
    pub path: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Project,
    Library,
    Stdlib,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FunctionKind {
    Def,
    Lambda,
    /// Module or class body: runs at import time, never on the event loop, but holds
    /// registrations such as `app.add_api_route(...)`.
    Module,
}

/// The role of a function in a property: which attribute access runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accessor {
    /// `obj.attr` (`@property`, `@cached_property`).
    Get,
    /// `obj.attr = value` (`@attr.setter`).
    Set,
    /// `del obj.attr` (`@attr.deleter`).
    Delete,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Function {
    /// `module.Class.method`, `module.outer.<locals>.inner`, `module.<lambda:LINE>`.
    pub qualname: String,
    pub location: Location,
    pub origin: Origin,
    pub kind: FunctionKind,
    pub is_async: bool,
    pub property: Option<Accessor>,
    /// An undecorated generator function: calling it only creates the generator, iterating the
    /// generator runs the body.
    pub is_generator: bool,
    /// A pydantic validator: constructing or validating its model runs it.
    pub is_validator: bool,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Class {
    pub qualname: String,
    pub bases: Vec<ClassId>,
    pub is_protocol: bool,
    /// Methods defined directly in the class body.
    pub methods: Vec<(String, FnId)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    /// The generator created by this call is iterated right away (`for x in gen()`,
    /// `list(gen())`), or the call is an implicit `__iter__` of an iteration.
    pub iterates: bool,
    /// Calling a class (or validating into a pydantic model): its `__post_init__` and
    /// validators run too.
    pub constructs: Option<ClassId>,
    /// `obj.name(...)` on an instance of the class, where `name` is not a method: whatever
    /// callables the class stores in that attribute (see [`Facts::stores`]).
    pub attribute: Option<(ClassId, String)>,
    /// The callee is the value returned by calling these functions (`get_loader()()`,
    /// `nap = make_sleeper(); nap()`), see [`Facts::returns`].
    pub returned_by: Vec<FnId>,
    /// Why nothing was resolved (only set when `targets` is empty and `param` is `None`).
    pub unresolved: Option<String>,
}

/// `self.name = value` in a method of `class`, with a callable (or a parameter) as the value.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Store {
    pub class: ClassId,
    pub name: String,
    pub value: Value,
}

/// A callable flowing into a parameter: `to_thread(f)`, `Depends(get_db)`, `@router.get(...)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Slot {
    Param(String),
    /// Applied as a decorator (the decorated function flows into the first parameter).
    Decorated,
    /// Positional/keyword position that could not be mapped to a parameter name.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Value {
    Function(FnId),
    Class(ClassId),
    /// A parameter of `owner` forwarded as-is (callback plumbing).
    Param(FnId, String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Facts {
    pub functions: Vec<Function>,
    pub classes: Vec<Class>,
    pub calls: Vec<Call>,
    pub flows: Vec<Flow>,
    pub stores: Vec<Store>,
    /// Callables (or parameters) a function returns.
    pub returns: Vec<(FnId, Value)>,
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
