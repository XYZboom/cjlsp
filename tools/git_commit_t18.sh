#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T18: hover alignment - reference-site resolution + rich decl rendering

Recovered from T18 worker's master-tree work (stash) which timed out before
committing; repaired to compile + clippy clean.

- hover.rs: 250 -> 1870 lines. Index over all decls + locals + members +
  containers; hover resolves at decl AND reference sites; renders signatures
  (mods, type params, params with defaults, doc comments, container prefix),
  std symbols (String/Array/Any/...), member access via receiver type,
  local func/var parsing from source lines
- server.rs: hover handler passes source
- Verified: hover_001 PASS, definition_001 PASS, workspace 96 tests,
  clippy -D warnings clean" 2>&1 | tail -1
git log --oneline | head -2