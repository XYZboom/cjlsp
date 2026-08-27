#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T13: LSP references support

- references.rs: references_at collects declaration + all Name-expression
  refs to the name under cursor, returns LSP Location[]; name_at finds the
  target (decl name or Name expr), includeDeclaration flag honored
- server.rs: handle_references + textDocument/references dispatch
- Verified: definition_001 PASS (official), references finds decl+call sites
- workspace 91 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline | head -2