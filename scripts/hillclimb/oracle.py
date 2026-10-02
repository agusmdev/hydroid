#!/usr/bin/env python3
"""A deliberately naive second opinion: blocking-looking calls written directly in `async def`
bodies, found syntactically. Compared with hydroid's report, the calls it finds and hydroid does
not are candidate false negatives to read by hand (or a reason the oracle is wrong: awaited,
shadowed, offloaded...).

Usage: oracle.py PROJECT_DIR HYDROID_REPORT.json
"""

import ast
import json
import re
import sys
from pathlib import Path

# Dotted callee text (as written) that looks blocking.
BLOCKING = re.compile(
    r"""^(
        time\.sleep | sleep
      | requests\.\w+ | httpx\.(get|post|put|patch|delete|head|options|request|stream)
      | urllib\.request\.\w+ | urlopen
      | open | io\.open | input
      | subprocess\.\w+ | os\.(system|popen|remove|unlink|rename|replace|makedirs|mkdir|rmdir|listdir|scandir|walk|stat|chmod|path\.exists|path\.isfile|path\.isdir|path\.getsize)
      | shutil\.\w+ | glob\.glob | tempfile\.\w+
      | socket\.(create_connection|getaddrinfo|gethostbyname)
      | json\.load | pickle\.load | yaml\.(safe_)?load
    )$""",
    re.X,
)
# Method names that look blocking whatever the receiver (pathlib, files, sync clients).
METHODS = re.compile(r"^(read_text|read_bytes|write_text|write_bytes|iterdir|rglob|unlink|mkdir|touch|exists|is_file|is_dir)$")


def dotted(node: ast.expr) -> str | None:
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        base = dotted(node.value)
        return f"{base}.{node.attr}" if base else None
    return None


class Finder(ast.NodeVisitor):
    def __init__(self) -> None:
        self.hits: list[tuple[int, str]] = []
        self.awaited: set[int] = set()

    def visit_AsyncFunctionDef(self, node: ast.AsyncFunctionDef) -> None:
        for stmt in node.body:
            self.scan(stmt)

    def scan(self, node: ast.AST) -> None:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda, ast.ClassDef)):
            self.visit(node)  # nested async defs are scanned on their own
            return
        if isinstance(node, ast.Await):
            self.awaited.add(id(node.value))
        if isinstance(node, ast.Call) and id(node) not in self.awaited:
            name = dotted(node.func)
            if name and BLOCKING.match(name):
                self.hits.append((node.lineno, name))
            elif isinstance(node.func, ast.Attribute) and METHODS.match(node.func.attr):
                self.hits.append((node.lineno, f"….{node.func.attr}"))
        for child in ast.iter_child_nodes(node):
            self.scan(child)


def main() -> int:
    root, report = Path(sys.argv[1]), json.loads(Path(sys.argv[2]).read_text())
    reported = {(d["location"]["path"], d["location"]["line"]) for d in report["diagnostics"]}
    unresolved = {(u["location"]["path"], u["location"]["line"]) for u in report["unresolved"]}
    missed = 0
    for path in sorted(root.rglob("*.py")):
        if any(p in (".venv", "node_modules", "site-packages") for p in path.parts):
            continue
        try:
            tree = ast.parse(path.read_text(), str(path))
        except (SyntaxError, UnicodeDecodeError):
            continue
        finder = Finder()
        finder.visit(tree)
        rel = path.relative_to(root).as_posix()
        for line, name in finder.hits:
            if (rel, line) in reported:
                continue
            missed += 1
            tag = "unresolved" if (rel, line) in unresolved else "silent"
            print(f"{rel}:{line}  {name}  [{tag}]")
    print(f"{missed} oracle hits not reported by hydroid", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
