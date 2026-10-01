//! Persistent cache of extracted facts, so that re-running hydroid on an unchanged project skips
//! type inference (seconds on large projects) and only re-runs the analysis.
//!
//! An entry is valid only if everything extraction reads is unchanged: the hydroid binary, the
//! options, the Python environment (interpreter version and the packages installed in
//! site-packages) and every Python source file under the project root (path and contents).
//! Anything else is a miss and a full extraction.

use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use hydroid_core::facts::{Call, Class, Facts, FnId, Flow, Function, Store, Value};
use rayon::prelude::*;
use rustc_hash::FxHasher;
use serde::{Deserialize, Serialize};

use crate::db::Layout;

const DIR: &str = ".hydroid_cache";
const FILE: &str = "facts.bin";

pub struct Entry {
    pub files: usize,
    pub facts: Facts,
}

/// Calls and flows are stored in chunks that deserialize in parallel.
const CHUNKS: usize = 8;

#[derive(Serialize, Deserialize)]
struct Head {
    files: usize,
    functions: Vec<Function>,
    classes: Vec<Class>,
    stores: Vec<Store>,
    returns: Vec<(FnId, Value)>,
    dispatches: Vec<(FnId, FnId)>,
    suppressed: Vec<(String, u32)>,
}

/// Hash of everything an extraction depends on.
pub fn fingerprint(layout: &Layout, python_version: &str, options: &str) -> anyhow::Result<u64> {
    let mut h = FxHasher::default();
    env!("CARGO_PKG_VERSION").hash(&mut h);
    // A rebuilt binary may extract differently (development builds share a version).
    if let Ok(meta) = std::env::current_exe().and_then(std::fs::metadata) {
        meta.len().hash(&mut h);
        mtime(&meta).hash(&mut h);
    }
    options.hash(&mut h);
    python_version.hash(&mut h);
    // Installing, upgrading or removing a package changes the site-packages listing
    // (`name-version.dist-info`).
    for site_packages in &layout.site_packages {
        site_packages.as_str().hash(&mut h);
        let mut names: Vec<_> = std::fs::read_dir(site_packages.as_std_path())
            .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        names.sort();
        names.hash(&mut h);
    }
    // Every module ty could resolve inside the project: excluded and gitignored files included
    // (generated code is often gitignored), environments and tool caches not.
    let mut sources = Vec::new();
    let walker = ignore::WalkBuilder::new(layout.root.as_std_path())
        .standard_filters(false)
        .filter_entry(|e| {
            !matches!(
                e.file_name().to_str(),
                Some(
                    ".git"
                        | ".hg"
                        | ".venv"
                        | "venv"
                        | "node_modules"
                        | "__pycache__"
                        | ".mypy_cache"
                        | ".ruff_cache"
                        | ".pytest_cache"
                        | ".tox"
                        | ".nox"
                        | DIR
                )
            )
        })
        .build();
    for entry in walker {
        let entry = entry?;
        let path = entry.path();
        if !matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("py" | "pyi")
        ) || layout
            .site_packages
            .iter()
            .any(|sp| path.starts_with(sp.as_std_path()))
        {
            continue;
        }
        sources.push(path.to_path_buf());
    }
    sources.sort();
    // Contents, not mtimes: coarse timestamps can hide a quick same-size edit.
    let hashes: Vec<u64> = sources
        .par_iter()
        .map(|path| {
            let mut h = FxHasher::default();
            std::fs::read(path).unwrap_or_default().hash(&mut h);
            h.finish()
        })
        .collect();
    (sources, hashes).hash(&mut h);
    Ok(h.finish())
}

fn mtime(meta: &std::fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

fn path(root: &Path) -> PathBuf {
    root.join(DIR).join(FILE)
}

/// The cached entry, and the fingerprint it was stored under. Unreadable entries are misses.
pub fn load(root: &Path) -> Option<(u64, Entry)> {
    let bytes = std::fs::read(path(root)).ok()?;
    let (stored, rest) = bytes.split_first_chunk::<8>()?;
    let sections: Vec<&[u8]> = split_sections(rest)?;
    let (head, (calls, flows)) = rayon::join(
        || bincode::deserialize::<Head>(sections.first()?).ok(),
        || {
            rayon::join(
                || parts::<Call>(&sections[1..1 + CHUNKS]),
                || parts::<Flow>(&sections[1 + CHUNKS..]),
            )
        },
    );
    let head = head?;
    let facts = Facts {
        functions: head.functions,
        classes: head.classes,
        calls: calls?,
        flows: flows?,
        stores: head.stores,
        returns: head.returns,
        dispatches: head.dispatches,
        suppressed: head.suppressed,
    };
    Some((
        u64::from_le_bytes(*stored),
        Entry {
            files: head.files,
            facts,
        },
    ))
}

/// `[count: u32][len: u64]*count` followed by the sections themselves.
fn split_sections(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    let (count, mut rest) = bytes.split_first_chunk::<4>()?;
    let count = u32::from_le_bytes(*count) as usize;
    if count != 1 + 2 * CHUNKS {
        return None;
    }
    let mut lens = Vec::with_capacity(count);
    for _ in 0..count {
        let (len, tail) = rest.split_first_chunk::<8>()?;
        lens.push(u64::from_le_bytes(*len) as usize);
        rest = tail;
    }
    let mut sections = Vec::with_capacity(count);
    for len in lens {
        if len > rest.len() {
            return None;
        }
        let (section, tail) = rest.split_at(len);
        sections.push(section);
        rest = tail;
    }
    Some(sections)
}

fn parts<T: for<'de> Deserialize<'de> + Send>(sections: &[&[u8]]) -> Option<Vec<T>> {
    let parts: Option<Vec<Vec<T>>> = sections
        .par_iter()
        .map(|s| bincode::deserialize(s).ok())
        .collect();
    Some(parts?.into_iter().flatten().collect())
}

fn chunks<T: Serialize + Sync>(items: &[T]) -> anyhow::Result<Vec<Vec<u8>>> {
    let size = items.len().div_ceil(CHUNKS).max(1);
    let mut out: Vec<Vec<u8>> = items
        .chunks(size)
        .map(bincode::serialize)
        .collect::<Result<_, _>>()?;
    out.resize_with(CHUNKS, || {
        bincode::serialize(&Vec::<T>::new()).expect("empty vec serializes")
    });
    Ok(out)
}

/// Best effort: a read-only project just runs uncached.
pub fn store(root: &Path, fingerprint: u64, entry: &Entry) {
    let dir = root.join(DIR);
    let write = || -> anyhow::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let ignore = dir.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(ignore, "# Created by hydroid.\n*\n")?;
        }
        let facts = &entry.facts;
        let head = Head {
            files: entry.files,
            functions: facts.functions.clone(),
            classes: facts.classes.clone(),
            stores: facts.stores.clone(),
            returns: facts.returns.clone(),
            dispatches: facts.dispatches.clone(),
            suppressed: facts.suppressed.clone(),
        };
        let mut sections = vec![bincode::serialize(&head)?];
        sections.extend(chunks(&facts.calls)?);
        sections.extend(chunks(&facts.flows)?);
        let tmp = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
        let mut file = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        file.write_all(&fingerprint.to_le_bytes())?;
        file.write_all(&(sections.len() as u32).to_le_bytes())?;
        for section in &sections {
            file.write_all(&(section.len() as u64).to_le_bytes())?;
        }
        for section in &sections {
            file.write_all(section)?;
        }
        file.into_inner()?.sync_all().ok();
        std::fs::rename(&tmp, path(root))?;
        Ok(())
    };
    let _ = write();
}
