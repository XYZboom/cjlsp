#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_ca649751
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T32: hover alignment round 2 (827 lines, doc-comment/edge cases)

- hover.rs: doc comment capture, edge-case decl rendering, richer detail
- server.rs: hover wiring
- workspace 100 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline -1