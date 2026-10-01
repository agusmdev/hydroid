//! Turns a Python project into [`Facts`] using ty's semantic model.
//!
//! Files are extracted in parallel (one database handle per thread); the results are merged
//! sequentially, interning every definition they mention into dense ids.

mod cache;
mod db;
mod extract;
mod index;

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use hydroid_core::catalog::qualname_matches;
use hydroid_core::facts::{Call, Class, ClassId, Facts, FnId, Flow, Function, Origin, Slot, Store, Value};
use parking_lot::Mutex;
use rayon::prelude::*;
use ruff_db::files::{File, system_path_to_file};
use ruff_db::system::{SystemPath, SystemPathBuf};
use rustc_hash::{FxHashMap, FxHashSet};
use ty_module_resolver::{ModuleName, resolve_module_confident};

use crate::db::HydroidDb;
use crate::extract::{FileFacts, RawSlot, RawValue, extract_file};
use crate::index::{FileIndex, Key};

pub struct Extraction {
    pub facts: Facts,
    pub files: usize,
}

pub struct ExtractOptions<'a> {
    pub root: &'a Path,
    pub python: Option<&'a Path>,
    /// Also analyze the bodies of third-party functions reachable from project code, except
    /// those for which `follow` returns `false` (catalog functions).
    pub follow_libs: bool,
    pub follow: &'a (dyn Fn(&str) -> bool + Sync),
    /// Gitignore-style globs, relative to `root`, of project files to skip.
    pub exclude: &'a [String],
    /// Reuse (and save) the facts of an unchanged project in `<root>/.hydroid_cache`.
    /// `cache_salt` must capture everything else the extraction depends on (`follow`).
    pub cache: bool,
    pub cache_salt: &'a str,
}

fn system_path(path: &Path) -> anyhow::Result<SystemPathBuf> {
    let path = path.canonicalize().with_context(|| format!("`{}` does not exist", path.display()))?;
    SystemPathBuf::from_path_buf(path).map_err(|p| anyhow::anyhow!("`{}` is not valid UTF-8", p.display()))
}

/// Absolute, but symlinks kept: `.venv/bin/python` must not become the base interpreter.
fn python_path(path: &Path) -> anyhow::Result<SystemPathBuf> {
    let path = std::path::absolute(path)?;
    SystemPathBuf::from_path_buf(path).map_err(|p| anyhow::anyhow!("`{}` is not valid UTF-8", p.display()))
}

/// Catalog names (or patterns) that match no definition in the environment's modules.
pub fn unknown_names(root: &Path, python: Option<&Path>, names: &[String]) -> anyhow::Result<Vec<String>> {
    let python = python.map(python_path).transpose()?;
    let db = HydroidDb::new(&system_path(root)?, python.as_deref())?;
    let environment = db.program().resolver_environment(&db);
    let mut indexes: FxHashMap<File, FileIndex> = FxHashMap::default();
    let mut unknown = Vec::new();
    for name in names {
        let segments: Vec<&str> = name.split('.').collect();
        let found = (1..segments.len()).rev().any(|split| {
            let Some(module) = ModuleName::new(&segments[..split].join(".")) else { return false };
            let Some(file) = resolve_module_confident(&db, environment, &module).and_then(|m| m.file(&db))
            else {
                return false;
            };
            let index = indexes.entry(file).or_insert_with(|| FileIndex::build(&db, file));
            index.functions.values().any(|f| qualname_matches(name, &f.qualname))
        });
        if !found {
            unknown.push(name.clone());
        }
    }
    Ok(unknown)
}

pub fn extract(options: &ExtractOptions) -> anyhow::Result<Extraction> {
    let root = system_path(options.root)?;
    let python = options.python.map(python_path).transpose()?;
    let db = HydroidDb::new(&root, python.as_deref())?;
    let fingerprint = if options.cache {
        let settings = format!("{:?} {:?} {}", options.follow_libs, options.exclude, options.cache_salt);
        let (layout, version) = (db.layout(), db.python_version());
        // Read the entry while the project is being fingerprinted.
        let (fingerprint, entry) = rayon::join(
            || cache::fingerprint(layout, &version, &settings),
            || cache::load(root.as_std_path()),
        );
        let fingerprint = fingerprint?;
        if let Some((stored, entry)) = entry
            && stored == fingerprint
        {
            return Ok(Extraction { facts: entry.facts, files: entry.files });
        }
        Some(fingerprint)
    } else {
        None
    };
    let files = project_files(&db, &root, options.exclude)?;

    let indexes = Indexes::default();
    let mut merger = Merger::new(&db, &indexes);
    let units: Vec<(File, Option<FxHashSet<u32>>)> = files.iter().map(|&f| (f, None)).collect();
    indexes.prebuild(&db, files.iter().copied().collect());
    let mut round = run(&db, &indexes, &units, options.follow_libs);
    let mut first = true;
    loop {
        // Merging interns every definition the facts mention, indexing its file: index them
        // all in parallel first.
        indexes.prebuild(&db, referenced_files(&round));
        if std::mem::take(&mut first) {
            for &file in &files {
                merger.add_file(file);
            }
        }
        for file_facts in round {
            merger.merge(file_facts);
        }
        if !options.follow_libs {
            break;
        }
        let next = merger.library_frontier(options.follow);
        if next.is_empty() {
            break;
        }
        round = run(&db, &indexes, &next, true);
    }
    let entry = cache::Entry { files: files.len(), facts: merger.finish() };
    // Tearing down ty's caches takes seconds on large projects: do it off the critical path.
    std::thread::spawn(move || drop((db, indexes)));
    if let Some(fingerprint) = fingerprint {
        cache::store(root.as_std_path(), fingerprint, &entry);
    }
    Ok(Extraction { facts: entry.facts, files: entry.files })
}

fn project_files(db: &HydroidDb, root: &SystemPath, exclude: &[String]) -> anyhow::Result<Vec<File>> {
    let site_packages = &db.layout().site_packages;
    let mut overrides = ignore::overrides::OverrideBuilder::new(root.as_std_path());
    for glob in exclude {
        overrides.add(&format!("!{glob}")).with_context(|| format!("invalid exclude glob `{glob}`"))?;
    }
    let mut files = Vec::new();
    for entry in ignore::WalkBuilder::new(root.as_std_path()).overrides(overrides.build()?).build() {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "py") || !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = SystemPathBuf::from_path_buf(path.to_path_buf())
            .map_err(|p| anyhow::anyhow!("`{}` is not valid UTF-8", p.display()))?;
        let excluded = path.components().any(|c| {
            matches!(c.as_str(), ".venv" | "venv" | "site-packages" | "node_modules" | "__pycache__")
        });
        if excluded || site_packages.iter().any(|sp| path.starts_with(sp)) {
            continue;
        }
        files.push(system_path_to_file(db, &path).with_context(|| format!("cannot read `{path}`"))?);
    }
    files.sort_by_key(|f| f.path(db).as_str().to_string());
    Ok(files)
}

/// File indexes shared between extraction threads and the merge.
#[derive(Default)]
struct Indexes(Mutex<FxHashMap<File, Arc<FileIndex>>>);

impl Indexes {
    fn get(&self, db: &HydroidDb, file: File) -> Arc<FileIndex> {
        if let Some(index) = self.0.lock().get(&file) {
            return index.clone();
        }
        let index = Arc::new(FileIndex::build(db, file));
        self.0.lock().entry(file).or_insert(index).clone()
    }

    /// Builds the indexes of `files`, and of the files defining the bases of every indexed
    /// class, in parallel (building one runs type inference on its file).
    fn prebuild(&self, db: &HydroidDb, files: FxHashSet<File>) {
        let mut pending: Vec<File> = {
            let built = self.0.lock();
            let bases = built.values().flat_map(|i| i.classes.values().flat_map(|c| c.bases.iter().map(|b| b.0)));
            let wanted: FxHashSet<File> = files.into_iter().chain(bases).collect();
            wanted.into_iter().filter(|f| !built.contains_key(f)).collect()
        };
        while !pending.is_empty() {
            let shared = Mutex::new(db.clone());
            let built: Vec<(File, Arc<FileIndex>)> = pending
                .par_iter()
                .map_init(|| shared.lock().clone(), |db, &file| (file, Arc::new(FileIndex::build(db, file))))
                .collect();
            let mut map = self.0.lock();
            let mut next = FxHashSet::default();
            for (file, index) in built {
                next.extend(index.classes.values().flat_map(|c| c.bases.iter().map(|b| b.0)));
                map.entry(file).or_insert(index);
            }
            pending = next.into_iter().filter(|f| !map.contains_key(f)).collect();
        }
    }

    /// Names of the properties defined in indexed files: only attribute reads with these names
    /// can run code.
    fn property_names(&self) -> FxHashSet<String> {
        let map = self.0.lock();
        map.values()
            .flat_map(|i| i.functions.values())
            .filter(|f| f.property.is_some())
            .filter_map(|f| f.qualname.rsplit('.').next().map(str::to_string))
            .collect()
    }
}

/// Files defining something the extracted facts refer to.
fn referenced_files(facts: &[FileFacts]) -> FxHashSet<File> {
    let mut files = FxHashSet::default();
    for f in facts {
        files.extend(f.calls.iter().flat_map(|c| c.targets.iter().map(|(key, _)| key.0)));
        files.extend(f.decorators.iter().flat_map(|(_, d)| d.iter().map(|key| key.0)));
        for store in &f.stores {
            files.insert(store.class.0);
            if let RawValue::Function((file, _)) | RawValue::Class((file, _)) = store.value {
                files.insert(file);
            }
        }
        for flow in &f.flows {
            files.insert(flow.into.0);
            if let RawValue::Function((file, _)) | RawValue::Class((file, _)) = flow.value {
                files.insert(file);
            }
        }
    }
    files
}

/// `all_properties`: check every attribute read for a property call (library properties
/// included), not only reads of names defined as properties in the files indexed so far.
fn run(
    db: &HydroidDb,
    indexes: &Indexes,
    units: &[(File, Option<FxHashSet<u32>>)],
    all_properties: bool,
) -> Vec<FileFacts> {
    let properties = (!all_properties).then(|| indexes.property_names());
    let shared = Mutex::new(db.clone());
    units
        .par_iter()
        .map_init(
            || shared.lock().clone(),
            |db, (file, wanted)| {
                let index = indexes.get(db, *file);
                extract_file(db, &index, wanted.as_ref(), properties.as_ref())
            },
        )
        .collect()
}

struct Merger<'a> {
    db: &'a HydroidDb,
    indexes: &'a Indexes,
    fn_ids: FxHashMap<Key, FnId>,
    class_ids: FxHashMap<Key, ClassId>,
    facts: Facts,
}

impl<'a> Merger<'a> {
    fn new(db: &'a HydroidDb, indexes: &'a Indexes) -> Self {
        Self { db, indexes, fn_ids: FxHashMap::default(), class_ids: FxHashMap::default(), facts: Facts::default() }
    }

    fn index(&self, file: File) -> Arc<FileIndex> {
        self.indexes.get(self.db, file)
    }

    /// Registers every definition of a project file; its bodies are analyzed.
    fn add_file(&mut self, file: File) {
        let index = self.index(file);
        for &key in index.functions.keys() {
            if let Some(id) = self.fn_id((file, key)) {
                self.facts.functions[id.0 as usize].analyzed = true;
            }
        }
        for &key in index.classes.keys() {
            self.class_id((file, key));
        }
        let path = index.location(0.into()).path;
        self.facts.suppressed.extend(index.suppressed.iter().map(|&l| (path.clone(), l)));
    }

    fn fn_id(&mut self, key: Key) -> Option<FnId> {
        if let Some(&id) = self.fn_ids.get(&key) {
            return Some(id);
        }
        let index = self.index(key.0);
        let entry = index.functions.get(&key.1)?;
        let id = FnId(self.facts.functions.len() as u32);
        self.fn_ids.insert(key, id);
        self.facts.functions.push(Function {
            qualname: entry.qualname.clone(),
            location: entry.location.clone(),
            origin: index.origin,
            kind: entry.kind,
            is_async: entry.is_async,
            property: entry.property,
            is_generator: entry.is_generator,
            is_validator: entry.is_validator,
            params: entry.params.clone(),
            parent: None,
            class: None,
            decorators: Vec::new(),
            analyzed: false,
        });
        let parent = entry.parent.and_then(|p| self.fn_id((key.0, p)));
        let class = entry.class.and_then(|c| self.class_id((key.0, c)));
        let function = &mut self.facts.functions[id.0 as usize];
        function.parent = parent;
        function.class = class;
        Some(id)
    }

    fn class_id(&mut self, key: Key) -> Option<ClassId> {
        if let Some(&id) = self.class_ids.get(&key) {
            return Some(id);
        }
        let index = self.index(key.0);
        let entry = index.classes.get(&key.1)?;
        let id = ClassId(self.facts.classes.len() as u32);
        self.class_ids.insert(key, id);
        self.facts.classes.push(Class {
            qualname: entry.qualname.clone(),
            bases: Vec::new(),
            is_protocol: entry.is_protocol,
            methods: Vec::new(),
        });
        let bases = entry.bases.iter().filter_map(|&b| self.class_id(b)).collect();
        let methods = entry
            .methods
            .iter()
            .filter_map(|(name, m)| Some((name.clone(), self.fn_id((key.0, *m))?)))
            .collect();
        let class = &mut self.facts.classes[id.0 as usize];
        class.bases = bases;
        class.methods = methods;
        Some(id)
    }

    fn merge(&mut self, file_facts: FileFacts) {
        let file = file_facts.file;
        for (key, decorators) in file_facts.decorators {
            let Some(id) = self.fn_id((file, key)) else { continue };
            let decorators = decorators.into_iter().filter_map(|d| self.fn_id(d)).collect();
            self.facts.functions[id.0 as usize].decorators = decorators;
        }
        for raw in file_facts.calls {
            let Some(caller) = self.fn_id((file, raw.caller)) else { continue };
            let mut targets: Vec<FnId> = raw.targets.iter().filter_map(|&(k, _)| self.fn_id(k)).collect();
            if let Some(accessor) = raw.accessor {
                targets.retain(|t| self.facts.function(*t).property == Some(accessor));
                if targets.is_empty() {
                    continue;
                }
            }
            let param = raw.param.and_then(|(owner, name)| Some((self.fn_id((file, owner))?, name)));
            let constructs = raw.constructs.and_then(|k| self.class_id(k));
            let attribute = raw.attribute.and_then(|(k, name)| Some((self.class_id(k)?, name)));
            self.facts.calls.push(Call {
                caller,
                location: raw.location,
                text: raw.text,
                unresolved: if targets.is_empty() && param.is_none() {
                    Some(raw.unresolved.unwrap_or_else(|| "the callee is not a function definition".into()))
                } else {
                    None
                },
                targets,
                virtual_dispatch: raw.virtual_dispatch,
                param,
                awaited: raw.awaited,
                iterates: raw.iterates,
                constructs,
                attribute,
            });
        }
        for raw in file_facts.stores {
            let Some(class) = self.class_id(raw.class) else { continue };
            let Some(value) = self.value(file, raw.value) else { continue };
            self.facts.stores.push(Store { class, name: raw.name, value });
        }
        for raw in file_facts.flows {
            let (Some(caller), Some(into)) = (self.fn_id((file, raw.caller)), self.fn_id(raw.into)) else {
                continue;
            };
            let Some(value) = self.value(file, raw.value) else { continue };
            let slot = match raw.slot {
                RawSlot::Decorated => Slot::Decorated,
                RawSlot::Keyword(name) => Slot::Param(name),
                RawSlot::Positional(i) => {
                    let index = self.index(raw.into.0);
                    let entry = &index.functions[&raw.into.1];
                    let i = i + usize::from(raw.bound && entry.class.is_some());
                    if i < entry.positional { Slot::Param(entry.params[i].clone()) } else { Slot::Unknown }
                }
            };
            self.facts.flows.push(Flow { location: raw.location, caller, into, slot, value, literal: raw.literal });
        }
    }

    fn value(&mut self, file: File, value: RawValue) -> Option<Value> {
        match value {
            RawValue::Function(k) => self.fn_id(k).map(Value::Function),
            RawValue::Class(k) => self.class_id(k).map(Value::Class),
            RawValue::Param(owner, name) => self.fn_id((file, owner)).map(|o| Value::Param(o, name)),
        }
    }

    /// Library functions called from analyzed code whose bodies have not been analyzed yet.
    fn library_frontier(&mut self, follow: &(dyn Fn(&str) -> bool + Sync)) -> Vec<(File, Option<FxHashSet<u32>>)> {
        let mut wanted: FxHashMap<File, FxHashSet<u32>> = FxHashMap::default();
        let keys: FxHashMap<FnId, Key> = self.fn_ids.iter().map(|(k, v)| (*v, *k)).collect();
        for call in &self.facts.calls {
            for &t in &call.targets {
                let f = self.facts.function(t);
                if f.analyzed || f.origin != Origin::Library || !f.location.path.ends_with(".py") || !follow(&f.qualname) {
                    continue;
                }
                let (file, key) = keys[&t];
                wanted.entry(file).or_default().insert(key);
            }
        }
        for (file, keys_in_file) in &wanted {
            for &key in keys_in_file {
                if let Some(&id) = self.fn_ids.get(&(*file, key)) {
                    self.facts.functions[id.0 as usize].analyzed = true;
                }
            }
        }
        wanted.into_iter().map(|(f, k)| (f, Some(k))).collect()
    }

    fn finish(self) -> Facts {
        self.facts
    }
}
