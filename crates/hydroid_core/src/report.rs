//! Analysis results.

use serde::{Deserialize, Serialize};

use crate::facts::Location;

#[derive(Clone, Debug, Serialize)]
pub struct Frame {
    pub location: Location,
    /// Callee as written at this call site.
    pub text: String,
    /// Qualified name of the function that runs.
    pub callee: String,
}

/// How much a blocking call harms the server: how long it can stop the loop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Stops the loop for a short, bounded time (local file metadata, small reads), or only
    /// while the server starts or stops.
    Warn,
    /// Can stop the loop for a long or unknown time: waits on the network, another process or
    /// thread, a timer, a person; or is slow by design, or raises on the loop.
    #[default]
    Critical,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sink {
    pub qualname: String,
    pub category: String,
    pub advice: String,
    /// The catalog severity of the blocking function.
    pub severity: Severity,
}

/// How the event loop reaches the diagnostic's function, starting at a FastAPI entry point
/// (or, for library code, at project code).
#[derive(Clone, Debug, Serialize)]
pub struct ReachedFrom {
    pub entry: String,
    pub location: Location,
    pub frames: Vec<Frame>,
}

/// A blocking call reachable on the event loop.
///
/// `location` is the call site inside code that runs on the loop (an `async def`, or a sync
/// callback scheduled on the loop): the place to fix. `chain` walks from that call site down
/// to the blocking function.
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub location: Location,
    pub function: String,
    pub chain: Vec<Frame>,
    pub sink: Sink,
    pub reached_from: Option<ReachedFrom>,
    /// The sink's severity, one level lower (critical to warn) when the call only runs while
    /// the server starts or stops.
    pub severity: Severity,
    /// The call only runs while the server starts or stops (lifespan, startup and shutdown
    /// handlers), never while it serves requests.
    pub runs_once: bool,
}

/// A call on the event loop whose target could not be determined.
#[derive(Clone, Debug, Serialize)]
pub struct Unresolved {
    pub location: Location,
    pub function: String,
    pub text: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub files: usize,
    pub functions_analyzed: usize,
    pub call_sites: usize,
    pub call_sites_resolved: usize,
    pub entry_points: usize,
    pub extract_ms: u128,
    pub analyze_ms: u128,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
    pub unresolved: Vec<Unresolved>,
    pub stats: Stats,
}
