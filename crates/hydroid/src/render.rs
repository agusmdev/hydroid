//! Report renderers.

use std::fmt::Write;

use hydroid_core::facts::Location;
use hydroid_core::report::{Frame, Report};
use serde_json::{Value, json};

fn frame_line(out: &mut String, frame: &Frame) {
    let at = format!("{}:{}:{}", frame.location.path, frame.location.line, frame.location.column);
    if frame.text == frame.callee {
        let _ = writeln!(out, "       {at}  {}", frame.text);
    } else {
        let _ = writeln!(out, "       {at}  {} -> {}", frame.text, frame.callee);
    }
}

pub fn human(report: &Report, show_unresolved: bool) -> String {
    let mut out = String::new();
    for d in &report.diagnostics {
        let l = &d.location;
        let _ = writeln!(out, "error[blocking-{}]: `{}` blocks the event loop", d.sink.category, d.sink.qualname);
        let _ = writeln!(out, "  --> {}:{}:{} in `{}`", l.path, l.line, l.column, d.function);
        if let Some(reached) = &d.reached_from {
            let r = &reached.location;
            let _ = writeln!(out, "   = reached from {} ({}:{})", reached.entry, r.path, r.line);
            for frame in &reached.frames {
                frame_line(&mut out, frame);
            }
        }
        let _ = writeln!(out, "   = blocking chain ({} calls):", d.chain.len());
        for frame in &d.chain {
            frame_line(&mut out, frame);
        }
        let _ = writeln!(out, "   = help: {}\n", d.sink.advice);
    }
    if show_unresolved {
        for u in &report.unresolved {
            let l = &u.location;
            let _ = writeln!(out, "warning[unresolved-call]: cannot tell what `{}` calls: {}", u.text, u.reason);
            let _ = writeln!(out, "  --> {}:{}:{} in `{}`\n", l.path, l.line, l.column, u.function);
        }
    }
    let s = &report.stats;
    let count = |n: usize, noun: &str| format!("{n} {noun}{}", if n == 1 { "" } else { "s" });
    let resolved =
        if s.call_sites == 0 { 100.0 } else { 100.0 * s.call_sites_resolved as f64 / s.call_sites as f64 };
    let _ = writeln!(
        out,
        "{} on the event loop, {} ({}, {}, {}, {resolved:.1}% of {} resolved, {} ms)",
        count(report.diagnostics.len(), "blocking call"),
        count(report.unresolved.len(), "unresolved call"),
        count(s.files, "file"),
        count(s.functions_analyzed, "function"),
        count(s.entry_points, "entry point"),
        count(s.call_sites, "call site"),
        s.extract_ms + s.analyze_ms,
    );
    out
}

fn sarif_location(location: &Location, message: Option<String>) -> Value {
    let mut value = json!({
        "physicalLocation": {
            "artifactLocation": { "uri": location.path },
            "region": { "startLine": location.line, "startColumn": location.column },
        }
    });
    if let Some(text) = message {
        value["message"] = json!({ "text": text });
    }
    value
}

/// SARIF 2.1.0, with each blocking chain as a code flow (GitHub code scanning renders it).
pub fn sarif(report: &Report, include_unresolved: bool) -> Value {
    let mut results: Vec<Value> = report
        .diagnostics
        .iter()
        .map(|d| {
            let flow: Vec<Value> = d
                .chain
                .iter()
                .map(|f| json!({ "location": sarif_location(&f.location, Some(format!("calls `{}`", f.callee))) }))
                .collect();
            json!({
                "ruleId": "blocking-call",
                "level": "error",
                "message": { "text": format!("`{}` blocks the event loop ({}): {}", d.sink.qualname, d.sink.category, d.sink.advice) },
                "locations": [sarif_location(&d.location, None)],
                "codeFlows": [{ "threadFlows": [{ "locations": flow }] }],
            })
        })
        .collect();
    if include_unresolved {
        results.extend(report.unresolved.iter().map(|u| {
            json!({
                "ruleId": "unresolved-call",
                "level": "warning",
                "message": { "text": format!("cannot tell what `{}` calls: {}", u.text, u.reason) },
                "locations": [sarif_location(&u.location, None)],
            })
        }));
    }
    json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": {
                "name": "hydroid",
                "version": env!("CARGO_PKG_VERSION"),
                "rules": [
                    { "id": "blocking-call", "shortDescription": { "text": "Blocking call on the event loop" } },
                    { "id": "unresolved-call", "shortDescription": { "text": "Call on the event loop with an unknown target" } },
                ],
            }},
            "results": results,
        }],
    })
}
