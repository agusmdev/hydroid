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
    /// A pydantic serialization hook (`@computed_field`, `@field_serializer`,
    /// `@model_serializer`): dumping its model runs it.
    pub is_serializer: bool,
    /// The fields a serialization hook is for (`@field_serializer("a", "b")`, the name of a
    /// `@computed_field`): excluding all of them from a dump skips it. Empty: any field.
    pub serializes: Vec<String>,
    /// A serialization hook only used when dumping to JSON (`when_used="json"` or
    /// `"json-unless-none"`): python-mode dumps (`model_dump()`) skip it.
    pub json_only: bool,
    /// A SQLAlchemy attribute validator (`@validates("email")`): setting one of these attributes
    /// on an instance of its class (in the constructor, or `obj.email = ...`) runs it.
    pub validates: Vec<String>,
    pub params: Vec<String>,
    /// Lexically enclosing function (closures read its parameters).
    pub parent: Option<FnId>,
    pub class: Option<ClassId>,
    /// Resolved decorator callables, outermost first.
    pub decorators: Vec<FnId>,
    /// The methods a library method calls on its own instance (`self.name(...)`): a project
    /// subclass overriding one of them runs its override there (template methods).
    pub self_calls: Vec<String>,
    /// Whether the body was analyzed. Unanalyzed functions (stdlib stubs, libraries without
    /// `--follow-libs`) are leaves: only the catalog knows whether they block.
    pub analyzed: bool,
    /// A copy of this method made by the analysis for one kind of `self` (see
    /// [`Instance`]): the method it copies.
    #[serde(default)]
    pub specializes: Option<FnId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Class {
    pub qualname: String,
    pub bases: Vec<ClassId>,
    pub is_protocol: bool,
    /// Methods defined directly in the class body.
    pub methods: Vec<(String, FnId)>,
    /// Classes named in the annotations of its class-level fields (nested models). Project
    /// classes only.
    pub fields: Vec<ClassId>,
    /// Fields declared with `Field(default_factory=f)` (pydantic) or `field(default_factory=f)`
    /// (dataclasses): constructing the class without giving the field calls the factory. Each is
    /// `(the names the field can be given by, the factory)`.
    pub default_factories: Vec<(Vec<String>, Value)>,
    /// The class-level fields of a project class (pydantic model fields), see [`ModelField`].
    pub model_fields: Vec<ModelField>,
}

/// A class-level field of a project class: `name: Annotated[list[Item], AfterValidator(f)]`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelField {
    /// The names the field can be given by (its name and aliases).
    pub names: Vec<String>,
    /// Classes named in its annotation (looking through type aliases): data given to the field
    /// is validated into these models.
    pub classes: Vec<ClassId>,
    /// Validators attached by `Annotated` metadata (`AfterValidator(f)`, ...): validating data
    /// given to the field runs them.
    pub validators: Vec<FnId>,
    /// Serializers attached by `Annotated` metadata (`PlainSerializer(f)`, ...): dumping the
    /// field runs them.
    pub serializers: Vec<FnId>,
    /// The `serializers` only used when dumping to JSON (`when_used="json"`, ...).
    pub json_serializers: Vec<FnId>,
}

/// What FastAPI validates and serializes around a function when it is an endpoint or a
/// dependency (see `fastapi.dependencies.utils.solve_dependencies` and
/// `fastapi.routing.serialize_response`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signature {
    pub function: FnId,
    /// What validating the parameters FastAPI fills from the request (not `Depends(...)` ones)
    /// runs: the classes named in their annotations, and the validators attached to them with
    /// `Annotated[str, AfterValidator(f)]`.
    pub params: Vec<Value>,
    /// Classes named in the return annotation (the default `response_model`). Empty for
    /// generators, whose items are streamed.
    pub annotation: Vec<ClassId>,
    /// Classes of the instances it returns (inferred). Empty for generators.
    pub returned: Vec<ClassId>,
    /// Some value it returns has a type ty does not know (it may be a model instance).
    pub returns_unknown: bool,
    /// Every value it returns is a Starlette `Response`, which FastAPI sends as is (no response
    /// validation, serialization or `response_class`).
    pub returns_response: bool,
}

/// Response options of a route registration (`@app.get(..., response_model=None)`) at a call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResponseOptions {
    pub caller: FnId,
    pub location: Location,
    /// `response_model=None`: no response model, not even the return annotation.
    pub no_model: bool,
    /// `response_model_exclude={"a", "b"}`: the top-level fields left out of the response.
    pub exclude: Vec<String>,
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
    /// The class of the instance a method is called on (`super().name(...)`: the class of the
    /// calling method). A target's `super().name(...)` runs the next `name` in this class's MRO,
    /// which, through cooperative mixins, is not the one the target's own bases define.
    pub receiver: Option<ClassId>,
    /// `super().name(...)`: the name.
    pub super_name: Option<String>,
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
    /// Attributes this call sets on an instance of the class: the keyword arguments of a
    /// construction, or the target of `obj.name = value`. Their ORM validators run (see
    /// [`Function::validates`]).
    pub sets: Option<(ClassId, Vec<String>)>,
    /// The arguments the call gives explicitly, by keyword: a construction's default factories
    /// for these fields do not run, and sinks skip their blocking work for some (catalog
    /// `unless_given`). `None` when that is not known (positional arguments).
    pub given: Option<Vec<String>>,
    /// The call passes `**mapping`: any keyword may be given.
    pub unpacks: bool,
    /// Keyword arguments given model instances (or nothing to validate, like `[]`): pydantic
    /// does not validate them into nested models again.
    pub instances: Vec<String>,
    /// A pydantic dump (`model.model_dump(...)`, see [`Dump`]), when the callee may be one.
    pub dump: Option<Dump>,
    /// Why nothing was resolved (only set when `targets` is empty and `param` is `None`).
    pub unresolved: Option<String>,
    /// The logging level the call passes: its `level=` keyword, or the first argument of
    /// `setLevel(...)` and `log(...)`.
    pub level: Option<Level>,
    /// See [`LogUse`]: set on logging calls, on calls attaching handlers, filters and formatters,
    /// and on constructions whose instance only gets attached.
    pub logging: Option<LogUse>,
    /// See [`Instance`]: set on `obj.name(...)` calls whose dispatch depends on more than the
    /// declared class of `obj`.
    pub instance: Option<Instance>,
}

/// What is known about the instance a method is called on (`obj.name(...)`), beyond the
/// declared class of `obj`, that narrows which `name` runs.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Instance {
    pub name: String,
    /// `obj` is the calling method's `self` (or `cls`): which `name` runs depends on the instance
    /// the method itself was called on.
    pub is_self: bool,
    /// `obj` is `self.<attribute>`, typed by a type variable of the class (`self.repo: RepoT` in
    /// `class Service(Generic[RepoT])`): the class an instance specializes it to (see `binds`)
    /// says which `name` runs.
    pub generic: Option<String>,
    /// `obj` is exactly an instance of one of these classes (`Model()`, or a variable only ever
    /// assigned constructions): `name` is the one their MROs find, never an override in a
    /// subclass.
    pub exact: Vec<ClassId>,
    /// `(attribute, class)`: attributes of `obj` typed by a type variable, and the class `obj`'s
    /// type specializes each to (`TagService(BaseService[TagRepository])`: `repo`,
    /// `TagRepository`).
    pub binds: Vec<(String, ClassId)>,
}

/// The arguments of a pydantic dump that decide which serializers run.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Dump {
    /// Dumps to JSON (`model_dump_json()`, `model_dump(mode="json")`), or the mode is not
    /// known: JSON-only serializers run too.
    pub json: bool,
    /// `exclude={"a"}` (a literal set, or a dict mapping names to `True`): top-level fields
    /// left out.
    pub exclude: Vec<String>,
    /// `include={"a", "b"}` (a literal set or dict): the only top-level fields dumped.
    pub include: Option<Vec<String>>,
}

/// How a call takes part in the logging setup.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LogUse {
    /// The logger the call is made on (`log.info(...)`, `log.addHandler(h)`; the root logger
    /// for `logging.info(...)`), when it is known to be one with a known name.
    pub logger: Option<LoggerName>,
    /// `addHandler(h)`, `addFilter(f)`, `setFormatter(f)`: the class of the instance attached.
    pub attaches: Option<ClassId>,
    /// A construction whose instance is only attached (`log.addHandler(Handler())`, or held in
    /// a variable that is only configured and attached).
    pub attach_only: bool,
}

/// A logger name: `getLogger("audit")`, `getLogger(__name__)` (the module), `""` for the root
/// logger. Not `exact`: only the start of it is known (`getLogger(f"svc.{name}")`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggerName {
    pub name: String,
    pub exact: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Level {
    /// `None`: not a known level (a variable, a setting).
    pub value: Option<u32>,
    /// `obj.setLevel(...)` on an instance of this class.
    pub receiver: Option<ClassId>,
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
    /// The class of the instance a bound method value belongs to (`Depends(scheme)`: the class
    /// of `scheme`, whose `__call__` is the value).
    pub receiver: Option<ClassId>,
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
    /// `(dispatcher, implementation)`: calling the dispatcher may run the implementation
    /// (`@render.register` on a `functools.singledispatch` function).
    pub dispatches: Vec<(FnId, FnId)>,
    /// Signatures of the functions that annotate or return classes, see [`Signature`].
    pub signatures: Vec<Signature>,
    /// See [`ResponseOptions`].
    pub response_options: Vec<ResponseOptions>,
    /// Lines carrying a `# hydroid: ignore` comment.
    pub suppressed: Vec<(String, u32)>,
    /// `logger.propagate = value` on a logger with a known name: `(name, value is False)`.
    pub stops_propagation: Vec<(String, bool)>,
}

impl Facts {
    pub fn function(&self, id: FnId) -> &Function {
        &self.functions[id.0 as usize]
    }

    pub fn class(&self, id: ClassId) -> &Class {
        &self.classes[id.0 as usize]
    }
}
