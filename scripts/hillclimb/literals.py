#!/usr/bin/env python3
"""String literals a change adds to hydroid's Rust code (crates/**/*.rs), for the gate's library
literal check: library knowledge belongs in crates/hydroid_core/catalog.toml as data, not in Rust.

A literal is added when the change raises its number of occurrences in the non-test code of the
crates (so moving code between files adds nothing). Not counted: comments and doc comments, code
under `#[cfg(test)]` or `#[test]`, and files in a `tests/` or `benches/` directory.

Added literals that look like qualified names fail the check: dotted identifiers (`logging.Handler`,
`starlette.responses.Response.render`) and dotted module prefixes (`fastapi.`, `redis.*`). A file
name (`app.py`: one dot and a file suffix) does not. Every other added literal is printed for a human to review (`emit`,
`response_model`).

Usage: literals.py BASE [HEAD]   (HEAD: default the working tree; exit 1 when the check fails)
"""

import bisect
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
QUALIFIED = re.compile(r"[A-Za-z_]\w*(\.[A-Za-z_]\w*)+(\.\*)?|[A-Za-z_]\w*(\.[A-Za-z_]\w*)*\.\*?")
FILE_NAME = re.compile(r"[\w-]+\.(py|pyi|pyc|toml|json|jsonl|lock|txt|md|rs|cfg|ini|yaml|yml|html|sh)")
TEST_ATTR = re.compile(r"#\s*\[\s*(test|cfg\s*\((?:[^\]]*[(,\s])?(?<!not\()test\s*[,)][^\]]*)\s*\]")
RAW = re.compile(r'b?r(#*)"')


def lex(src: str) -> tuple[list[tuple[int, str]], str]:
    """(string literals as (offset, value), the source with comments and literals blanked)."""
    out, mask, i, n = [], list(src), 0, len(src)

    def blank(a: int, b: int) -> None:
        for k in range(a, min(b, n)):
            if mask[k] != "\n":
                mask[k] = " "

    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c in "br" and (m := RAW.match(src, i)) and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            close = '"' + m.group(1)
            start = m.end()
            j = src.find(close, start)
            j = n if j < 0 else j
            out.append((i, src[start:j]))
            blank(i, j + len(close))
            i = j + len(close)
        elif c == '"' or (c == "b" and src.startswith('b"', i) and (i == 0 or not src[i - 1].isalnum())):
            start = i + (2 if c == "b" else 1)
            j, buf = start, []
            while j < n and src[j] != '"':
                if src[j] == "\\" and j + 1 < n:
                    nxt = src[j + 1]
                    if nxt == "\n":  # line continuation: skips the newline and leading spaces
                        j += 2
                        while j < n and src[j] in " \t\n":
                            j += 1
                        continue
                    buf.append({"n": "\n", "t": "\t", "r": "\r", "0": "\0"}.get(nxt, nxt))
                    j += 2
                    if nxt == "u":
                        k = src.find("}", j)
                        j = k + 1 if k >= 0 else j
                    elif nxt == "x":
                        j += 2
                    continue
                buf.append(src[j])
                j += 1
            out.append((i, "".join(buf)))
            blank(i, j + 1)
            i = j + 1
        elif c == "'":
            if src.startswith("\\", i + 1):  # '\n', '\'', '\u{..}'
                j = src.find("'", i + 3 if src[i + 2:i + 3] == "'" else i + 2)
                j = n if j < 0 else j
                blank(i, j + 1)
                i = j + 1
            elif i + 2 < n and src[i + 2] == "'":  # 'x'
                blank(i, i + 3)
                i += 3
            else:  # a lifetime
                i += 1
        else:
            i += 1
    return out, "".join(mask)


def test_spans(mask: str) -> list[tuple[int, int]]:
    """Source ranges of items marked `#[cfg(test)]` or `#[test]` (in the blanked source)."""
    spans = []
    for m in TEST_ATTR.finditer(mask):
        j = m.end()
        while True:  # further attributes on the same item
            a = re.compile(r"\s*#\s*\[").match(mask, j)
            if not a:
                break
            depth, j = 1, a.end()
            while j < len(mask) and depth:
                depth += {"[": 1, "]": -1}.get(mask[j], 0)
                j += 1
        while j < len(mask) and mask[j] not in "{;":
            j += 1
        if j < len(mask) and mask[j] == "{":
            depth, j = 1, j + 1
            while j < len(mask) and depth:
                depth += {"{": 1, "}": -1}.get(mask[j], 0)
                j += 1
        spans.append((m.start(), j))
    return spans


def literals(src: str) -> list[tuple[int, str]]:
    """(line, value) of the string literals in non-test code."""
    found, mask = lex(src)
    spans = test_spans(mask)
    newlines = [k for k, ch in enumerate(src) if ch == "\n"]
    return [(bisect.bisect_left(newlines, off) + 1, value) for off, value in found
            if not any(a <= off < b for a, b in spans)]


def counted(path: str) -> bool:
    parts = path.split("/")
    return path.startswith("crates/") and path.endswith(".rs") and "tests" not in parts and "benches" not in parts


def tree_sources(ref: str | None) -> dict[str, str]:
    """crates/**/*.rs at a ref, or in the working tree (ref None)."""
    if ref is None:
        return {p.relative_to(ROOT).as_posix(): p.read_text() for p in (ROOT / "crates").rglob("*.rs")
                if counted(p.relative_to(ROOT).as_posix()) and "/target/" not in p.as_posix()}
    names = subprocess.run(["git", "ls-tree", "-r", "--name-only", ref, "--", "crates"], cwd=ROOT,
                           capture_output=True, text=True, check=True).stdout.split()
    return {n: subprocess.run(["git", "show", f"{ref}:{n}"], cwd=ROOT, capture_output=True, text=True,
                              check=True).stdout for n in names if counted(n)}


def added(base: str, head: str | None = None) -> dict[str, list[str]]:
    """Added literal -> the head locations (file:line) in files where its count grew."""
    before = {p: literals(s) for p, s in tree_sources(base).items()}
    after = {p: literals(s) for p, s in tree_sources(head).items()}
    total_before = Counter(v for lits in before.values() for _, v in lits)
    total_after = Counter(v for lits in after.values() for _, v in lits)
    out = {}
    for value, count in total_after.items():
        if count <= total_before[value]:
            continue
        locs = []
        for path, lits in sorted(after.items()):
            here = [ln for ln, v in lits if v == value]
            if len(here) > sum(v == value for _, v in before.get(path, [])):
                locs += [f"{path}:{ln}" for ln in here]
        out[value] = locs
    return out


def qualified(value: str) -> bool:
    return bool(QUALIFIED.fullmatch(value)) and not FILE_NAME.fullmatch(value)


def report(base: str, head: str | None = None) -> tuple[bool, str]:
    """(passes, text) for the gate."""
    new = added(base, head)
    bad = sorted(v for v in new if qualified(v))
    lines = []
    if bad:
        lines.append(f"{len(bad)} added literal(s) name library code (move this knowledge to "
                     "crates/hydroid_core/catalog.toml, or derive it by analysis):")
        for v in bad:
            locs = new[v]
            lines.append(f"  {v!r:48} {', '.join(locs[:3])}" + (f" (+{len(locs) - 3})" if len(locs) > 3 else ""))
    other = sorted(v for v in new if not qualified(v))
    if other:
        lines.append(f"{len(other)} other added literal(s), for review (not a failure):")
        names = [v for v in other if re.fullmatch(r"[\w./*-]+", v)]
        text = [v for v in other if v not in names]
        for chunk in range(0, len(names), 8):
            lines.append("  " + "  ".join(repr(v) for v in names[chunk:chunk + 8]))
        for v in text:
            shown = v if len(v) <= 70 else v[:67] + "..."
            lines.append(f"  {shown!r}  {new[v][0] if new[v] else ''}")
    if not new:
        lines.append("no string literals added to crates/**/*.rs")
    return not bad, "\n".join(lines)


def main() -> int:
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    ok, text = report(sys.argv[1], sys.argv[2] if len(sys.argv) == 3 else None)
    print(text)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
