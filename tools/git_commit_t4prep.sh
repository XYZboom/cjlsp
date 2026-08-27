#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T4-prep: wire literal typecheck into LSP + Var.name_pos

- server.rs: chain cj_sema::typecheck::check_decls into analyze_source
  (011 suite: String=1 / Int8='x' / Int8=999999 now diagnosed)
- AST: Decl::Var gains name_pos (binding-name token position); parser fills
  it for decls and patterns; unused.rs reports Var at the name, not 'let'
- lsp_cov coverage 28.8% -> 39.2% (011 6/6, 013 4/4, 013-calls already in T1)
- workspace 51 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline | head -3