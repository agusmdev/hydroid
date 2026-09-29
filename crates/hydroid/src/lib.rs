use std::path::PathBuf;

pub use hydroid_core::report::Report;

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub root: PathBuf,
    /// Virtual environment or interpreter; discovered like ty does when `None`.
    pub python: Option<PathBuf>,
    pub follow_libs: bool,
}

pub fn check(_options: &Options) -> anyhow::Result<Report> {
    Ok(Report::default())
}
