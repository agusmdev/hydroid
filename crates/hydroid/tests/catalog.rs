//! Every name in the builtin catalog must be a real defining qualname in the fixture environment
//! (stdlib via typeshed, libraries via the locked virtualenv). A misspelled or re-exported name
//! would silently never match.

mod common;

use common::{FIXTURE_VENV, fixtures_dir};
use hydroid_core::catalog::Catalog;

#[test]
fn every_catalog_name_resolves() {
    let names = Catalog::builtin(true).names();
    let unknown = hydroid_ty::unknown_names(&fixtures_dir(), Some(&FIXTURE_VENV), &names).unwrap();
    assert!(unknown.is_empty(), "{} of {} catalog names match no definition:\n  {}", unknown.len(), names.len(), unknown.join("\n  "));
}
