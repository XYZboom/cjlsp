#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_f5c6aaf6
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T12: diagnostics coverage 84.8% -> 91.2% (114/125)

- +8 diagnostics matched: lexer/parser/expr/ty/sema refinements
- workspace 83 tests, clippy -D warnings clean
- lsp_cov 114/125 (91.2%)" 2>&1 | tail -1
git log --oneline -1