//! PEP 561 stub-only packages: a `foo-stubs` package provides the module `foo`. Catalog names
//! (`foo.bar`) must match the definitions whether the stubs are installed or not, fully or
//! partially, and `--follow-libs` must still follow the real package's source.

use std::path::{Path, PathBuf};

use hydroid::Options;

/// What the environment has besides the real package `foo`.
#[derive(Clone, Copy, Debug)]
enum Stubs {
    None,
    /// `foo-stubs` covers `foo` and `foo.io`.
    Full,
    /// `foo-stubs` (`py.typed`: `partial`) covers `foo` only: `foo.io` resolves to the source.
    Partial,
}

/// A virtual environment with `foo` (and maybe `foo-stubs`), and a project calling it from an
/// `async def`.
fn setup(name: &str, stubs: Stubs) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("stub-packages-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    let venv = dir.join("venv");
    let site = venv.join("lib/python3.13/site-packages");
    let write = |path: PathBuf, text: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(venv.join("bin/python"), "");
    write(venv.join("pyvenv.cfg"), &format!("home = {}\nversion = 3.13.0\n", venv.join("bin").display()));
    write(site.join("foo/__init__.py"), "import time\n\n\ndef bar():\n    time.sleep(1)\n");
    write(site.join("foo/io.py"), "import time\n\n\ndef load():\n    time.sleep(1)\n");
    match stubs {
        Stubs::None => {}
        Stubs::Full => {
            write(site.join("foo-stubs/__init__.pyi"), "def bar() -> None: ...\n");
            write(site.join("foo-stubs/io.pyi"), "def load() -> None: ...\n");
        }
        Stubs::Partial => {
            write(site.join("foo-stubs/__init__.pyi"), "def bar() -> None: ...\n");
            write(site.join("foo-stubs/py.typed"), "partial\n");
        }
    }
    let project = dir.join("project");
    write(
        project.join("main.py"),
        "import foo\nimport foo.io\n\n\nasync def handler():\n    foo.bar()\n    foo.io.load()\n",
    );
    (project, venv)
}

const SINKS: &str = r#"
[[sink]]
category = "test"
advice = "do not"
functions = ["foo.bar", "foo.io.load"]
"#;

fn sinks(stubs: Stubs, follow_libs: bool, catalog: bool) -> Vec<String> {
    let name = format!("{stubs:?}-{follow_libs}-{catalog}").to_lowercase();
    let (root, venv) = setup(&name, stubs);
    let report = hydroid::check(&Options {
        root,
        python: Some(venv),
        follow_libs,
        catalogs: if catalog { vec![SINKS.to_string()] } else { Vec::new() },
        ..Options::default()
    })
    .unwrap();
    let mut found: Vec<String> = report.diagnostics.iter().map(|d| d.sink.qualname.clone()).collect();
    found.sort();
    found
}

#[test]
fn catalog_names_match_with_or_without_stubs() {
    for stubs in [Stubs::None, Stubs::Full, Stubs::Partial] {
        assert_eq!(sinks(stubs, false, true), ["foo.bar", "foo.io.load"], "{stubs:?}");
    }
}

#[test]
fn follow_libs_follows_the_source_behind_stubs() {
    for stubs in [Stubs::None, Stubs::Full, Stubs::Partial] {
        assert_eq!(sinks(stubs, true, false), ["time.sleep", "time.sleep"], "{stubs:?}");
    }
}
