#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T5: macro expander (spec Ch.14) - builtin dispatch + user-macro template splicing

- expander.rs: collect_macros (Decl::Macro per file), expand_builtins walks
  decl/expr trees for MacroExpand, builtin macros (@sourceFile/@sourceLine/
  @sourcePackage) expand to literals, unresolved macro => diag
- expand_user_macro: substitute call args into quote template token parts
  (param-name identifier replacement); arity check diag
- Expansions trace records call site + generated text (for LSP preview and
  diagnostics context - macro-expansion preview requirement)
- parser: macro decl now optional return type ': Tokens'; quote(@Foo) nested
  macro calls parse into MacroExpand parts
- 4 expander tests; workspace 55 tests, clippy -D warnings clean" 2>&1 | tail -1