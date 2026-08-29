#!/usr/bin/env python3
"""Build a symbol -> (file, line, col) index from downloaded Cangjie stdlib
source. The LSP loads this index to resolve jump targets (Ctrl+Click on
standard-library types/functions) into real stdlib source files.

Index format (~/.cangjie-lsp/std/<version>/index.json):
{
  "version": "1.1.3",
  "symbols": {
    "String":     {"file": "std/core/string.cj", "line": 85,  "col": 15, "kind": "struct"},
    "Array":      {"file": "std/core/array.cj",  "line": 37,  "col": 17, "kind": "class"},
    "println":    {"file": "std/core/console...", "line": 12, "col": 9,  "kind": "func"}
  }
}

Usage:
  python3 tools/stdlib_index.py                       # index ~/.cangjie-lsp/std/<latest>
  python3 tools/stdlib_index.py --version 1.1.3
"""

import argparse
import json
import os
import re
import sys

# Declaration line shapes in official stdlib .cj (public modifiers optional):
#   public struct String
#   public open class StringBuilder <: ToString
#   public interface ToString
#   public enum Color { ... }
#   public func println(format: String): Unit
#   public prop val count: Int64
#   public class Foo { public func init() }
DECL_RE = re.compile(
    r"^\s*(?:public\s+|private\s+|internal\s+|protected\s+|static\s+|open\s+|"
    r"sealed\s+|abstract\s+|redef\s+|foreign\s+|macro\s+)*"
    r"(?P<kind>struct|class|interface|enum|func|prop)\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b",
    re.MULTILINE,
)


def index_stdlib(root: str, version: str) -> dict:
    symbols = {}
    for dirpath, _dirs, files in os.walk(root):
        for fn in sorted(files):
            if not fn.endswith(".cj"):
                continue
            path = os.path.join(dirpath, fn)
            try:
                with open(path, encoding="utf-8") as f:
                    lines = f.readlines()
            except Exception:
                continue
            for lineno, line in enumerate(lines, 1):
                m = DECL_RE.match(line)
                if not m:
                    continue
                kind = m.group("kind")
                # ignore func inside class bodies? (std uses top-level funcs
                # mostly; keep all — jumps to the first decl are fine)
                name = m.group("name")
                rel = os.path.relpath(path, root).replace("\\", "/")
                # column of the name token relative to line start
                col = line.find(name) + 1  # 1-based col like AST
                if name not in symbols:
                    symbols[name] = {
                        "file": rel,
                        "line": lineno,
                        "col": col,
                        "kind": kind,
                    }
    return {"version": version, "symbols": symbols}


def main():
    ap = argparse.ArgumentParser(description="Build stdlib symbol index")
    ap.add_argument("--version", default=None, help="SDK version (default: latest downloaded)")
    ap.add_argument("--list", action="store_true", help="list downloaded versions")
    args = ap.parse_args()

    base = os.path.join(os.path.expanduser("~"), ".cangjie-lsp", "std")
    if not os.path.isdir(base):
        print(f"No stdlib download dir: {base}", file=sys.stderr)
        sys.exit(1)

    if args.list:
        print("Downloaded versions:", ", ".join(sorted(os.listdir(base))))
        return

    if args.version:
        ver = args.version
    else:
        vers = [v for v in sorted(os.listdir(base)) if os.path.isdir(os.path.join(base, v))]
        if not vers:
            print("No versions downloaded yet.", file=sys.stderr)
            sys.exit(1)
        ver = vers[-1]

    vroot = os.path.join(base, ver)
    idx = index_stdlib(vroot, ver)
    out = os.path.join(vroot, "index.json")
    with open(out, "w", encoding="utf-8") as f:
        json.dump(idx, f, indent=1, ensure_ascii=False)
    n = len(idx["symbols"])
    print(f"Indexed {n} symbols from {ver} -> {out}")
    # Show a few samples
    for name in ["String", "Array", "println", "print", "Int64", "sort", "len"]:
        if name in idx["symbols"]:
            s = idx["symbols"][name]
            print(f"  {name}: {s['file']}:{s['line']}:{s['col']} ({s['kind']})")


if __name__ == "__main__":
    main()