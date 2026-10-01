//! The `hydroid` binary: configuration from `pyproject.toml`, output formats, exit status.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::FIXTURE_VENV;
use serde_json::Value;

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, content) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    dir
}

fn hydroid(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hydroid"))
        .arg(dir)
        .args(["--python", FIXTURE_VENV.to_str().unwrap()])
        .args(args)
        .output()
        .unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!("{e}: {}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
    })
}

const PYPROJECT: &str = r#"
[tool.hydroid]
exclude = ["scripts/**"]

[[tool.hydroid.sink]]
category = "sdk"
advice = "use the async client"
functions = ["vendor.client.fetch"]
"#;

const APP: &str = "
from vendor.client import fetch


async def handler():
    fetch()
";

const SCRIPT: &str = "
import time


async def main():
    time.sleep(1)
";

#[test]
fn config_adds_sinks_and_excludes_files() {
    let dir = project(
        "config",
        &[
            ("pyproject.toml", PYPROJECT),
            ("app.py", APP),
            ("vendor/__init__.py", ""),
            ("vendor/client.py", "def fetch():\n    pass\n"),
            ("scripts/tool.py", SCRIPT),
        ],
    );
    let output = hydroid(&dir, &["--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{report:#}");
    assert_eq!(diagnostics[0]["sink"]["qualname"], "vendor.client.fetch");
    assert_eq!(diagnostics[0]["sink"]["category"], "sdk");
    assert_eq!(diagnostics[0]["location"]["path"], "app.py");
    assert_eq!(report["stats"]["files"], 3, "scripts/tool.py is excluded");
}

#[test]
fn sarif_carries_the_chain_as_a_code_flow() {
    let dir = project("sarif", &[("app.py", "import time\n\ndef nap():\n    time.sleep(1)\n\nasync def handler():\n    nap()\n")]);
    let output = hydroid(&dir, &["--format", "sarif"]);
    assert_eq!(output.status.code(), Some(1));
    let sarif = json(&output);
    let results = sarif["runs"][0]["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["ruleId"], "blocking-call");
    let region = &results[0]["locations"][0]["physicalLocation"]["region"];
    assert_eq!((region["startLine"].as_u64(), region["startColumn"].as_u64()), (Some(7), Some(5)));
    let flow = results[0]["codeFlows"][0]["threadFlows"][0]["locations"].as_array().unwrap();
    let lines: Vec<u64> =
        flow.iter().map(|l| l["location"]["physicalLocation"]["region"]["startLine"].as_u64().unwrap()).collect();
    assert_eq!(lines, [7, 4]);
}

#[test]
fn exit_status() {
    let clean = project("clean", &[("app.py", "import asyncio\n\nasync def handler():\n    await asyncio.sleep(1)\n")]);
    assert_eq!(hydroid(&clean, &[]).status.code(), Some(0));

    let unresolved = project("unresolved", &[("app.py", "async def handler(obj, name):\n    getattr(obj, name)()\n")]);
    assert_eq!(hydroid(&unresolved, &[]).status.code(), Some(0));
    assert_eq!(hydroid(&unresolved, &["--strict"]).status.code(), Some(1));

    let missing = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-does-not-exist");
    assert_eq!(hydroid(&missing, &[]).status.code(), Some(2));
}

#[test]
fn unchanged_projects_reuse_cached_facts() {
    let dir = project("cache", &[("app.py", "import time\n\n\nasync def handler():\n    time.sleep(1)\n")]);
    let first = json(&hydroid(&dir, &["--format", "json"]));
    assert!(dir.join(".hydroid_cache/facts.bin").is_file());
    let second = json(&hydroid(&dir, &["--format", "json"]));
    assert_eq!(first["diagnostics"], second["diagnostics"]);
    assert_eq!(second["diagnostics"].as_array().unwrap().len(), 1);

    // Any change to a source file is a miss: the new code is analyzed.
    let fixed = "import asyncio\n\n\nasync def handler():\n    await asyncio.sleep(1)\n";
    std::fs::write(dir.join("app.py"), fixed).unwrap();
    let edited = json(&hydroid(&dir, &["--format", "json"]));
    assert_eq!(edited["diagnostics"].as_array().unwrap().len(), 0);

    // `--no-cache` neither reads nor writes it.
    let dir = project("no-cache", &[("app.py", "async def handler():\n    pass\n")]);
    hydroid(&dir, &["--no-cache"]);
    assert!(!dir.join(".hydroid_cache").exists());
}
