use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").canonicalize().unwrap()
}

/// The shared locked virtualenv of `tests/fixtures`, synced once per test binary.
pub static FIXTURE_VENV: LazyLock<PathBuf> = LazyLock::new(|| {
    let dir = fixtures_dir();
    let status = Command::new("uv")
        .args(["sync", "--frozen", "--quiet"])
        .current_dir(&dir)
        .status()
        .expect("`uv` must be installed to run the fixture tests");
    assert!(status.success(), "uv sync failed in {}", dir.display());
    dir.join(".venv")
});
