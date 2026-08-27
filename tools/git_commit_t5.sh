#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T5: quote expression + @ macro invocation parsing (spec Ch.14)

- expr.rs: TokenKind::QUOTE parses quote(...) into Expr::Quote{parts} — body
  tokens as TokenPart, \$(expr) interpolation, nested @Foo(...) macro calls
- TokenKind::AT/AT_EXCL parses @Foo(args) into Expr::MacroExpand{name,args}
  (args captured as Tokenish)
- AST already had Quote/MacroExpand/TokenPart variants; wired parser to them
- verified: quote(2+3) -> TokenParts; quote(@SayHi(\"say hi\")) -> MacroExpand
- clippy/parser tests pending full run" 2>&1 | tail -1