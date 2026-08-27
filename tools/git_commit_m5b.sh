#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "M5: unused-declaration diagnostics (18/30 cjlsp cases)

- unused.rs: detect top-level Func/Class/Interface/Struct/Enum/Var and
  function params never referenced; severity Hint (LSP 4), message
  'X is declared but never used'
- skip unused report when a decl body/init references an undefined name
  (function/var not fully analyzable) — matches official suite
- AST: Decl::Func gains name_pos (function-name token position, official
  reports at the name not the func keyword)
- resolver: bare-name call callee unresolved is NOT reported undeclared
  (cross-file/external function like test06)
- Severity gains Hint variant; LSP maps Hint->4, Note->3
- cjlsp diagnostics: 003 passes again, 013 has 3/4 diagnostics exact
  (only type checking 'mismatched types' missing — M3c scope)" 2>&1 | tail -1
git log --oneline | head -3