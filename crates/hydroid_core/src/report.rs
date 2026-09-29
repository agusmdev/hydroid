//! Analysis results.

use serde::Serialize;

use crate::facts::Location;

#[derive(Clone, Debug, Serialize)]
pub struct Frame {
    pub location: Location,
    /// Callee as written at this call site.
    pub text: String,
    /// Qualified name of the function that runs.
    pub callee: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sink {
    pub qualname: String,
    pub category: String,
    pub advice: String,
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
