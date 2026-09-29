//! What hydroid knows about functions it does not analyze: which block, which move work off the
//! event loop, which schedule callbacks on it, and which register FastAPI entry points.

use std::collections::HashMap;

use anyhow::Context;
use serde::Deserialize;

use crate::report::Sink;

const BUILTIN: &str = include_str!("../catalog.toml");

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    #[serde(default)]
    sink: Vec<SinkGroup>,
    /// Decorators that make calls to the sync functions they decorate block (retry loops).
    #[serde(default)]
    blocking_decorator: Vec<SinkGroup>,
    #[serde(default)]
    offload: Group,
    #[serde(default)]
    loop_callback: Group,
    #[serde(default)]
    entry: Vec<EntryGroup>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SinkGroup {
    category: String,
    advice: String,
    #[serde(default)]
    opt_in: bool,
    functions: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Group {
    #[serde(default)]
    functions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryGroup {
    kind: String,
    functions: Vec<String>,
}

/// Qualified-name patterns: exact names in a map, `*` patterns scanned.
struct Patterns<T> {
    exact: HashMap<String, T>,
    globs: Vec<(Vec<String>, T)>,
}

impl<T: Clone> Default for Patterns<T> {
    fn default() -> Self {
        Self { exact: HashMap::new(), globs: Vec::new() }
    }
}

impl<T: Clone> Patterns<T> {
    fn insert(&mut self, pattern: &str, value: T) {
        if pattern.contains('*') {
            self.globs.push((pattern.split('.').map(str::to_string).collect(), value));
        } else {
            self.exact.insert(pattern.to_string(), value);
        }
    }

    fn get(&self, qualname: &str) -> Option<&T> {
        if let Some(value) = self.exact.get(qualname) {
            return Some(value);
        }
        if self.globs.is_empty() {
            return None;
        }
        let segments: Vec<&str> = qualname.split('.').collect();
        self.globs.iter().find_map(|(pattern, value)| {
            (pattern.len() == segments.len()
                && pattern.iter().zip(&segments).all(|(p, s)| segment_matches(p, s)))
            .then_some(value)
        })
    }

    fn all(&self) -> impl Iterator<Item = String> + '_ {
        self.exact.keys().cloned().chain(self.globs.iter().map(|(p, _)| p.join(".")))
    }
}

/// Whether `qualname` matches a catalog name (`*` matches within one dotted segment).
pub fn qualname_matches(pattern: &str, qualname: &str) -> bool {
    let (mut p, mut q) = (pattern.split('.'), qualname.split('.'));
    loop {
        match (p.next(), q.next()) {
            (None, None) => return true,
            (Some(p), Some(q)) if segment_matches(p, q) => {}
            _ => return false,
        }
    }
}

/// `*` matches any run of characters inside one segment.
fn segment_matches(pattern: &str, segment: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = segment.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.is_empty()
}

#[derive(Default)]
pub struct Catalog {
    sinks: Patterns<usize>,
    blocking_decorators: Patterns<usize>,
    groups: Vec<(String, String)>,
    offload: Patterns<()>,
    loop_callbacks: Patterns<()>,
    entries: Patterns<usize>,
    entry_kinds: Vec<String>,
}

impl Catalog {
    /// The builtin catalog. `opt_in` enables groups marked `opt_in` (CPU-bound work).
    pub fn builtin(opt_in: bool) -> Self {
        let mut catalog = Self::default();
        catalog.extend(BUILTIN, opt_in).expect("builtin catalog is valid");
        catalog
    }

    /// Adds the groups of a catalog document (same schema as the builtin one).
    pub fn extend(&mut self, toml_text: &str, opt_in: bool) -> anyhow::Result<()> {
        let file: CatalogFile = toml::from_str(toml_text).context("invalid catalog")?;
        for (group, blocking_decorator) in
            file.sink.into_iter().map(|g| (g, false)).chain(file.blocking_decorator.into_iter().map(|g| (g, true)))
        {
            if group.opt_in && !opt_in {
                continue;
            }
            let index = self.groups.len();
            self.groups.push((group.category, group.advice));
            let patterns = if blocking_decorator { &mut self.blocking_decorators } else { &mut self.sinks };
            for name in &group.functions {
                patterns.insert(name, index);
            }
        }
        for name in &file.offload.functions {
            self.offload.insert(name, ());
        }
        for name in &file.loop_callback.functions {
            self.loop_callbacks.insert(name, ());
        }
        for group in file.entry {
            let index = self.entry_kinds.len();
            self.entry_kinds.push(group.kind);
            for name in &group.functions {
                self.entries.insert(name, index);
            }
        }
        Ok(())
    }

    pub fn sink(&self, qualname: &str) -> Option<Sink> {
        let &index = self.sinks.get(qualname)?;
        let (category, advice) = &self.groups[index];
        Some(Sink { qualname: qualname.to_string(), category: category.clone(), advice: advice.clone() })
    }

    /// The blocking behavior a decorator gives the sync functions it decorates.
    pub fn blocking_decorator(&self, qualname: &str) -> Option<Sink> {
        let &index = self.blocking_decorators.get(qualname)?;
        let (category, advice) = &self.groups[index];
        Some(Sink { qualname: qualname.to_string(), category: category.clone(), advice: advice.clone() })
    }

    pub fn is_offload(&self, qualname: &str) -> bool {
        self.offload.get(qualname).is_some()
    }

    pub fn is_loop_callback(&self, qualname: &str) -> bool {
        self.loop_callbacks.get(qualname).is_some()
    }

    /// The entry-point kind registered by passing a callable to `qualname`.
    pub fn entry_kind(&self, qualname: &str) -> Option<&str> {
        self.entries.get(qualname).map(|&i| self.entry_kinds[i].as_str())
    }

    /// Every name and pattern in the catalog.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .sinks
            .all()
            .chain(self.blocking_decorators.all())
            .chain(self.offload.all())
            .chain(self.loop_callbacks.all())
            .chain(self.entries.all())
            .collect();
        names.sort();
        names.dedup();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::segment_matches;

    #[test]
    fn glob_segments() {
        assert!(segment_matches("*Commands", "BasicKeyCommands"));
        assert!(segment_matches("*", "anything"));
        assert!(segment_matches("get*", "get_many"));
        assert!(segment_matches("a*c*e", "abcde"));
        assert!(!segment_matches("*Commands", "CommandsParser"));
        assert!(!segment_matches("get*", "set"));
        assert!(!segment_matches("a*c*e", "abcd"));
    }
}
