#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_2b6815e2
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T29: LLT success rate 80.8% -> ~88% (wildcard/if-let/misc parser+sema)

- wildcard recovery classification (verified against official cjc)
- if-let / var pattern handling, enum member parsing
- expr.rs/decl.rs parser fixes
- workspace 100 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline -1