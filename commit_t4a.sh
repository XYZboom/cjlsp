#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_240bded0
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T4a: unused diagnostics tags + codeActions (quickfix.removeUnusedSymbol)

- cj-diag: FixKind enum (Func/Class/Interface/Struct/Enum/Var/Param) driving
  quickfix deletion-range computation
- unused.rs: unused_diag builds Hint-severity diagnostic tagged Unnecessary
  (LSP tags [1]) + code action quickfix.removeUnusedSymbol; FixKind noun for
  title 'Remove unused <noun> '<name>'; records declaration start pos
- server.rs: emits tags + codeActions (deletion range from source text)
- lsp_cov coverage 80.0% -> 84.8% (106/125)
- workspace 80 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline | head -2