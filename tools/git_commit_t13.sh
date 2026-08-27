#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T13: LSP definition support (Location at cursor decl)

- hover.rs: definition_at reuses decl_hover name-span matching, returns
  LSP Location (uri + name range)
- server.rs: handle_definition wires textDocument/definition dispatch
- Official framework PASS: definition_001 (uri + range exact match)
- workspace 91 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline | head -2