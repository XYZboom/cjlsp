#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_556d8230
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T26: completion alignment round 3 (AST generator + decl tweaks)

- generated.rs: 145-line reduction (AST gen simplification)
- hover.rs: +2, decl.rs: +6
- workspace 100 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline -1