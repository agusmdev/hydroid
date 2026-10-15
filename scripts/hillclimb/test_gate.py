#!/usr/bin/env python3
"""Unit tests for gate.py's pure parts. Usage: python3 scripts/hillclimb/test_gate.py"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gate  # noqa: E402
import literals  # noqa: E402


class HoldoutBaseline(unittest.TestCase):
    def test_set_hash_ignores_order(self):
        self.assertEqual(gate.set_hash(sorted(["b", "a"])), gate.set_hash(["a", "b"]))
        self.assertNotEqual(gate.set_hash(["a", "b"]), gate.set_hash(["a", "b", "c"]))

    def test_recorded_round_needs_the_same_set(self):
        old, new = gate.set_hash(["a"]), gate.set_hash(["a", "b"])
        state = {"rounds": [
            {"round": 1, "holdout_recall": "1/2", "holdout_set": old},
            {"round": 2, "holdout_recall": "3/9"},  # before sets were recorded: never a baseline
            {"round": 3, "holdout_recall": "2/2", "holdout_set": old},
        ]}
        self.assertEqual(gate.recorded_holdout(state, old), (2, 2, 3))
        self.assertIsNone(gate.recorded_holdout(state, new))


RUST = r"""
//! Module doc: "logging.Doc" is not code.
/// Item doc: "logging.ItemDoc".
const BASES: &[&str] = &["logging.Handler", r"logging.Raw", r#"say "hi""#, b"bytes"];
/* block "logging.Block" /* nested "logging.Nested" */ still "logging.Comment" */
fn f<'a>(x: &'a str) -> char {
    let q = '"'; let e = '\''; let u = '\u{41}';
    if x == "emit" { 'x' } else { q }
}
#[cfg(test)]
mod tests {
    #[test]
    fn t() { assert_eq!("logging.InTest", "x"); }
}
#[test]
fn top_level_test() { let _ = "logging.TopTest"; }
#[cfg(not(test))]
fn real() { let _ = "logging.NotTest"; }
"""


class Literals(unittest.TestCase):
    def test_skips_comments_docs_and_test_code(self):
        values = [v for _, v in literals.literals(RUST)]
        self.assertEqual(values, ["logging.Handler", "logging.Raw", 'say "hi"', "bytes", "emit",
                                  "logging.NotTest"])

    def test_lines(self):
        self.assertEqual(dict((v, ln) for ln, v in literals.literals(RUST))["emit"], 8)

    def test_qualified(self):
        for v in ("logging.Handler", "starlette.responses.Response.render", "fastapi.", "redis.*",
                  "pydantic.main.BaseModel.json"):
            self.assertTrue(literals.qualified(v), v)
        for v in ("emit", "app.py", "__init__.py", "{module}.{name}", "a call value", "/sqlalchemy/", ""):
            self.assertFalse(literals.qualified(v), v)


if __name__ == "__main__":
    unittest.main()
