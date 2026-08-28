#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_ee806827
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T25: LLT success improvements (range-step/expr/ty parsing)

- expr.rs: +112 range-step (: expr) parsing per spec Ch.05
- parser.rs: +41 contextual keyword handling
- ty.rs: +12, generated.rs +1 (AST field)
- resolved decl.rs conflict (kept T24's parse_enum_cases)
- workspace 100 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline -1