#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_80eb4c0d
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T21: completion alignment round 2 (860-line refinement)

- completion.rs: +860/-334; richer item generation, context handling,
  statement-level initializer detail, LetPatternDestructor handling
- removed dead expr_pos/expr_source, unused initializer binding
- workspace 100 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline -1