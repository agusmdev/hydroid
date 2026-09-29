//! Runs hydroid on every project under `tests/fixtures/cases/` and compares its diagnostics with
//! the inline expectations in the sources:
//!
//! - `# expect: time.sleep` — a diagnostic on this line whose blocking function is `time.sleep`
//!   (several qualnames separated by spaces for several diagnostics on one line). A trailing
//!   `via <entry>` also checks the entry point reported as reaching it (`via route GET /x`,
//!   `via nothing` when no entry point does).
//! - `# expect-unresolved` — an unresolved call on the event loop on this line; only checked in
//!   cases whose `case.toml` sets `unresolved = true`.
//!
//! Every diagnostic must be expected and every expectation must be met. The analysis runs for
//! real against a locked virtualenv (`tests/fixtures/uv.lock`) — nothing is mocked.
//! `HYDROID_CASE=<name>` runs a single case.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::{FIXTURE_VENV, fixtures_dir};
use hydroid::Options;

#[derive(Default)]
struct CaseConfig {
    unresolved: bool,
    follow_libs: bool,
}

fn case_config(dir: &Path) -> CaseConfig {
    let Ok(text) = std::fs::read_to_string(dir.join("case.toml")) else {
        return CaseConfig::default();
    };
    let flag = |key: &str| text.lines().any(|l| l.replace(' ', "") == format!("{key}=true"));
    CaseConfig { unresolved: flag("unresolved"), follow_libs: flag("follow_libs") }
}

type Expected = BTreeMap<(String, u32), Vec<String>>;

fn expectations(dir: &Path) -> (Expected, Expected) {
    let mut blocking = Expected::new();
    let mut unresolved = Expected::new();
    let walker = ignore::WalkBuilder::new(dir).build();
    for entry in walker.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "py") {
            continue;
        }
        let rel = path.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
        for (i, line) in std::fs::read_to_string(path).unwrap().lines().enumerate() {
            let key = (rel.clone(), i as u32 + 1);
            if let Some((_, rest)) = line.split_once("# expect: ") {
                // `time.sleep time.sleep via route GET /x`: sinks, then the entry reaching them.
                let (names, via) = match rest.split_once(" via ") {
                    Some((names, entry)) => (names, format!(" via {}", entry.trim())),
                    None => (rest, String::new()),
                };
                let names = names.split_whitespace().map(|n| format!("{n}{via}"));
                blocking.entry(key.clone()).or_default().extend(names);
            }
            if line.contains("# expect-unresolved") {
                unresolved.entry(key).or_default().push(String::new());
            }
        }
    }
    (blocking, unresolved)
}

fn diff(kind: &str, expected: &Expected, actual: &Expected, out: &mut Vec<String>) {
    let keys: std::collections::BTreeSet<_> = expected.keys().chain(actual.keys()).collect();
    for key in keys {
        let mut want = expected.get(key).cloned().unwrap_or_default();
        let mut got = actual.get(key).cloned().unwrap_or_default();
        want.sort();
        got.sort();
        if want != got {
            out.push(format!("  {}:{} {kind}: expected {want:?}, got {got:?}", key.0, key.1));
        }
    }
}

fn run_case(dir: &Path) -> Vec<String> {
    let config = case_config(dir);
    let options = Options {
        root: dir.to_path_buf(),
        python: Some(FIXTURE_VENV.clone()),
        follow_libs: config.follow_libs,
        ..Options::default()
    };
    let report = match hydroid::check(&options) {
        Ok(report) => report,
        Err(err) => return vec![format!("  check failed: {err:#}")],
    };
    let (want_blocking, want_unresolved) = expectations(dir);
    let mut got_blocking = Expected::new();
    for d in &report.diagnostics {
        let key = (d.location.path.clone(), d.location.line);
        let wants_entry = want_blocking.get(&key).is_some_and(|w| w.iter().any(|n| n.contains(" via ")));
        let entry = d.reached_from.as_ref().map_or("nothing", |r| r.entry.split(": ").next().unwrap_or_default());
        let got = if wants_entry { format!("{} via {entry}", d.sink.qualname) } else { d.sink.qualname.clone() };
        got_blocking.entry(key).or_default().push(got);
    }
    let mut mismatches = Vec::new();
    diff("blocking", &want_blocking, &got_blocking, &mut mismatches);
    if config.unresolved {
        let mut got_unresolved = Expected::new();
        for u in &report.unresolved {
            let key = (u.location.path.clone(), u.location.line);
            got_unresolved.entry(key).or_default().push(String::new());
        }
        diff("unresolved", &want_unresolved, &got_unresolved, &mut mismatches);
    }
    if !mismatches.is_empty() {
        let rendered = serde_json::to_string_pretty(&report.diagnostics).unwrap();
        mismatches.push(format!("  diagnostics: {rendered}"));
    }
    mismatches
}

#[test]
fn fixture_cases() {
    let only = std::env::var("HYDROID_CASE").ok();
    let mut cases: Vec<PathBuf> = std::fs::read_dir(fixtures_dir().join("cases"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .filter(|p| only.as_deref().is_none_or(|o| p.file_name().unwrap() == o))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no fixture cases selected");
    let mut failures = Vec::new();
    for case in &cases {
        let mismatches = run_case(case);
        let name = case.file_name().unwrap().to_string_lossy();
        if mismatches.is_empty() {
            println!("ok   {name}");
        } else {
            println!("FAIL {name}");
            failures.push(format!("{name}:\n{}", mismatches.join("\n")));
        }
    }
    assert!(failures.is_empty(), "{} of {} cases failed:\n{}", failures.len(), cases.len(), failures.join("\n"));
}
