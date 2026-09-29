//! `[tool.hydroid]` in the project's `pyproject.toml`.
//!
//! ```toml
//! [tool.hydroid]
//! python = ".venv"          # virtualenv or interpreter
//! follow-libs = false
//! cpu = false
//! strict = false
//! exclude = ["migrations/**", "tests/**"]
//!
//! # Catalog additions, same schema as hydroid's builtin catalog:
//! [[tool.hydroid.sink]]
//! category = "sdk"
//! advice = "use the async client"
//! functions = ["acme.client.Client.fetch"]
//! ```

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub python: Option<PathBuf>,
    #[serde(default)]
    pub follow_libs: bool,
    #[serde(default)]
    pub cpu: bool,
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Catalog tables (`sink`, `blocking_decorator`, `offload`, `loop_callback`, `entry`),
    /// re-serialized for [`hydroid_core::catalog::Catalog::extend`].
    #[serde(flatten)]
    pub catalog: toml::Table,
}

#[derive(Deserialize)]
struct PyProject {
    tool: Option<Tool>,
}

#[derive(Deserialize)]
struct Tool {
    hydroid: Option<Config>,
}

impl Config {
    /// Reads `<root>/pyproject.toml`; a missing file or table is the default configuration.
    /// Relative paths are resolved against `root`.
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let path = root.join("pyproject.toml");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(Self::default());
        };
        let pyproject: PyProject =
            toml::from_str(&text).with_context(|| format!("invalid `{}`", path.display()))?;
        let mut config = pyproject.tool.and_then(|t| t.hydroid).unwrap_or_default();
        config.python = config.python.map(|p| root.join(p));
        Ok(config)
    }

    /// The catalog additions as a catalog document, if any.
    pub fn catalog_document(&self) -> Option<String> {
        (!self.catalog.is_empty()).then(|| toml::to_string(&self.catalog).expect("tables serialize"))
    }
}
