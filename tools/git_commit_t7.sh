#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T7: macro expansion wired into LSP pipeline (spec Ch.14)

- expander.rs: expand_file_with_cache high-level entry walks decl/expr trees
  for @MacroExpand, ExpandCtx aggregates context (8-arg fn eliminated)
- Expansion priority: builtin macros (sourceFile/sourceLine/sourcePackage)
  -> macro-package compile cache (.so via SDK cjpm) -> unresolved error
- macro_cache.rs: three-layer cache (source-hash .so reuse, LRU expansion
  results, session-scoped) — performance requirement for LSP
- server.rs: analyze_source runs expand_file_with_cache; macro diags chained
  into output; path_or_unknown helper for file name
- E2E verified: @sourceLine() expands clean, @NoSuchMacro(1) -> 'unresolved
  macro' error at correct position, coexists with unused diagnostics
- tools/test_macro_e2e.py: e2e harness (initialize+didOpen, frame parsing,
  asserts unresolved-macro reporting)
- workspace 83 tests, clippy -D warnings clean, lsp_cov 80.0% stable" 2>&1 | tail -1
git log --oneline | head -3