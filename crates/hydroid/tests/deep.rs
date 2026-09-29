//! Generated projects: a blocking call at the bottom of very deep call chains, split across many
//! modules, must be found with the whole chain.

mod common;

use std::fmt::Write;
use std::path::Path;

use common::FIXTURE_VENV;
use hydroid::Options;

/// `depth` sync functions `f0 -> f1 -> ... -> f{depth-1} -> time.sleep`, `per_module` per file.
fn write_chain(dir: &Path, depth: usize, per_module: usize) {
    let pkg = dir.join("chain");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("__init__.py"), "").unwrap();
    let modules = depth.div_ceil(per_module);
    for m in 0..modules {
        let mut src = String::from("import time\n");
        if m + 1 < modules {
            writeln!(src, "from chain import m{}", m + 1).unwrap();
        }
        for i in m * per_module..((m + 1) * per_module).min(depth) {
            let next = i + 1;
            let body = if next == depth {
                "time.sleep(1)".to_string()
            } else if next % per_module == 0 {
                format!("m{}.f{next}()", m + 1)
            } else {
                format!("f{next}()")
            };
            writeln!(src, "\n\ndef f{i}():\n    {body}").unwrap();
        }
        std::fs::write(pkg.join(format!("m{m}.py")), src).unwrap();
    }
    let app = "from fastapi import FastAPI\n\nfrom chain.m0 import f0\n\napp = FastAPI()\n\n\n@app.get(\"/\")\nasync def root():\n    f0()\n";
    std::fs::write(dir.join("main.py"), app).unwrap();
}

fn check_depth(depth: usize, per_module: usize) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("deep-{depth}-{per_module}"));
    let _ = std::fs::remove_dir_all(&dir);
    write_chain(&dir, depth, per_module);
    let report = hydroid::check(&Options {
        root: dir.clone(),
        python: Some(FIXTURE_VENV.clone()),
        ..Options::default()
    })
    .unwrap();
    assert_eq!(report.diagnostics.len(), 1, "{:#?}", report.diagnostics);
    let d = &report.diagnostics[0];
    assert_eq!((d.location.path.as_str(), d.location.line), ("main.py", 10));
    assert_eq!(d.sink.qualname, "time.sleep");
    assert_eq!(d.chain.len(), depth + 1, "chain: f0 .. f{} then time.sleep", depth - 1);
    assert_eq!(d.chain[0].callee, "chain.m0.f0");
    let last_module = (depth - 1) / per_module;
    assert_eq!(d.chain[depth - 1].callee, format!("chain.m{last_module}.f{}", depth - 1));
    assert!(d.reached_from.as_ref().is_some_and(|r| r.entry == "route GET /: main.root"));
}

#[test]
fn chain_of_400_in_one_module() {
    check_depth(400, 400);
}

#[test]
fn chain_of_5000_across_100_modules() {
    check_depth(5000, 50);
}
