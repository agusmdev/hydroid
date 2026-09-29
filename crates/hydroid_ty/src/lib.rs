//! Turns a Python project into [`Facts`] using ty's semantic model.
//!
//! Files are extracted in parallel (one database handle per thread); the results are merged
//! sequentially, interning every definition they mention into dense ids.

mod db;
mod extract;
mod index;

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use hydroid_core::facts::{Call, Class, ClassId, Facts, FnId, Flow, Function, Origin, Slot, Value};
use parking_lot::Mutex;
use rayon::prelude::*;
use ruff_db::files::{File, system_path_to_file};
use ruff_db::system::{SystemPath, SystemPathBuf};
use rustc_hash::{FxHashMap, FxHashSet};

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
}

fn system_path(path: &Path) -> anyhow::Result<SystemPathBuf> {
    let path = path.canonicalize().with_context(|| format!("`{}` does not exist", path.display()))?;
    SystemPathBuf::from_path_buf(path).map_err(|p| anyhow::anyhow!("`{}` is not valid UTF-8", p.display()))
}

pub fn extract(options: &ExtractOptions) -> anyhow::Result<Extraction> {
    let root = system_path(options.root)?;
    let python = options.python.map(system_path).transpose()?;
    let db = HydroidDb::new(&root, python.as_deref())?;
    let files = project_files(&db, &root)?;

    let indexes = Indexes::default();
    let mut merger = Merger::new(&db, &indexes);
    let units: Vec<(File, Option<FxHashSet<u32>>)> = files.iter().map(|&f| (f, None)).collect();
    let mut round = run(&db, &indexes, &units);
    for &file in &files {
        merger.add_file(file);
    }
    loop {
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
        round = run(&db, &indexes, &next);
    }
    Ok(Extraction { facts: merger.finish(), files: files.len() })
}

fn project_files(db: &HydroidDb, root: &SystemPath) -> anyhow::Result<Vec<File>> {
    let site_packages = &db.layout().site_packages;
    let mut files = Vec::new();
    for entry in ignore::WalkBuilder::new(root.as_std_path()).build() {
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
}

fn run(db: &HydroidDb, indexes: &Indexes, units: &[(File, Option<FxHashSet<u32>>)]) -> Vec<FileFacts> {
    let shared = Mutex::new(db.clone());
    units
        .par_iter()
        .map_init(
            || shared.lock().clone(),
            |db, (file, wanted)| {
                let index = indexes.get(db, *file);
                extract_file(db, &index, wanted.as_ref())
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
            is_property: entry.is_property,
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
            if raw.property_access {
                targets.retain(|t| self.facts.function(*t).is_property);
                if targets.is_empty() {
                    continue;
                }
            }
            let param = raw.param.and_then(|(owner, name)| Some((self.fn_id((file, owner))?, name)));
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
            });
        }
        for raw in file_facts.flows {
            let (Some(caller), Some(into)) = (self.fn_id((file, raw.caller)), self.fn_id(raw.into)) else {
                continue;
            };
            let value = match raw.value {
                RawValue::Function(k) => self.fn_id(k).map(Value::Function),
                RawValue::Class(k) => self.class_id(k).map(Value::Class),
                RawValue::Param(owner, name) => self.fn_id((file, owner)).map(|o| Value::Param(o, name)),
            };
            let Some(value) = value else { continue };
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
