#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_48e3d2e7
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T30: LLT success improvements (wildcard/token handling)

- token/parser fixes per LLT failure clusters (wildcard, identifier-likes)
- workspace 100 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline -1