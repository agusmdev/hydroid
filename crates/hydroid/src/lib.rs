pub mod config;
pub mod render;

use std::path::PathBuf;
use std::time::Instant;

use hydroid_core::analysis::analyze;
use hydroid_core::catalog::Catalog;
pub use hydroid_core::report::Report;
use hydroid_ty::{ExtractOptions, extract};

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub root: PathBuf,
    /// Virtual environment or interpreter; discovered like ty does when `None`.
    pub python: Option<PathBuf>,
    /// Analyze third-party function bodies too.
    pub follow_libs: bool,
    /// Report CPU-bound calls (hashing, key derivation).
    pub cpu: bool,
    /// Extra catalog documents (same schema as the builtin catalog).
    pub catalogs: Vec<String>,
    /// Gitignore-style globs of project files to skip, relative to `root`.
    pub exclude: Vec<String>,
}

pub fn check(options: &Options) -> anyhow::Result<Report> {
    let mut catalog = Catalog::builtin(options.cpu);
    for document in &options.catalogs {
        catalog.extend(document, options.cpu)?;
    }
    let follow = |qualname: &str| catalog.sink(qualname).is_none() && !catalog.is_offload(qualname);
    let started = Instant::now();
    let extraction = extract(&ExtractOptions {
        root: &options.root,
        python: options.python.as_deref(),
        follow_libs: options.follow_libs,
        follow: &follow,
        exclude: &options.exclude,
    })?;
    let extracted = Instant::now();
    let mut report = analyze(&extraction.facts, &catalog);
    report.stats.files = extraction.files;
    report.stats.extract_ms = (extracted - started).as_millis();
    report.stats.analyze_ms = extracted.elapsed().as_millis();
    Ok(report)
}
