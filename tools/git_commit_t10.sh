#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T10: LSP completion + hover features

- completion.rs: collect file top-level decls + std core symbols (Any etc.),
  prefix filter, exact-match-only rule (official behavior), LSP items shape
- hover.rs: heterogeneous decl signature rendering (func/class/interface/
  struct/enum/var/macro), name_pos span matching, 0-based LSP range,
  markdown contents 'Declared in / Package info / code block'
- server.rs: handle_completion + handle_hover, dispatch wiring
- Official framework PASS: completion_001, hover_001
- workspace 91 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline | head -3