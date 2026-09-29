//! Report renderers.

use std::fmt::Write;

use hydroid_core::report::{Frame, Report};

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
    let _ = writeln!(
        out,
        "{} blocking call{} on the event loop, {} unresolved call{} ({} files, {} functions, {} entry points, {:.1}% of {} call sites resolved, {} ms)",
        report.diagnostics.len(),
        if report.diagnostics.len() == 1 { "" } else { "s" },
        report.unresolved.len(),
        if report.unresolved.len() == 1 { "" } else { "s" },
        s.files,
        s.functions_analyzed,
        s.entry_points,
        if s.call_sites == 0 { 100.0 } else { 100.0 * s.call_sites_resolved as f64 / s.call_sites as f64 },
        s.call_sites,
        s.extract_ms + s.analyze_ms,
    );
    out
}
