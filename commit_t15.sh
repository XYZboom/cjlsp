#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_7aab31ef
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T15: number-literal lexer precision + pattern AST nodes

- lexer.rs: leading-dot floats (.5), hex floats (0x1.fp1), radix prefixes,
  'success' gate suppressing secondary diagnostics on a number (mirrors
  official ProcessDigits/ProcessNumberFloatSuffix)
- gen_ast.py + generated.rs: pattern nodes (CONST_PATTERN/TUPLE_PATTERN etc.)
- parser.rs: pattern support
- workspace 95 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline | head -2