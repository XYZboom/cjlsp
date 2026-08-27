#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_90b4f9b1
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T23: LLT success rate 61.0% -> 75.3% (parser/ty fixes)

- ty.rs: type parsing improvements
- llt_baseline.txt: updated baseline
- 6 files changed, 364 insertions(+), 87 deletions(-)
- workspace 36 tests (base), clippy -D warnings clean" 2>&1 | tail -1
git log --oneline -1