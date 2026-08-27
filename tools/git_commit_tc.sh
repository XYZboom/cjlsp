#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "M3c: literal type checking for typed var decls (spec Ch.02)

- typecheck.rs: check typed top-level vars with literal initializers
  * String target + int literal -> 'cannot convert an integer literal to type Struct-String'
  * Rune/String literal into non-string -> 'mismatched types expected X, found Struct-String'
  * int literal out of declared int range -> 'the number X exceeds the value range of type Y'
- String/Rune display as 'Struct-String' (official diagnostics suite naming)
- int ranges per spec Ch.02 (Int8..UInt64, Byte=UInt8)
- 4 typecheck tests; workspace 19 sema tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline | head -2