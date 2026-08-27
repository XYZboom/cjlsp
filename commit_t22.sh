#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_9bfc9b38
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T22: hover alignment round 2 (parser support + richer resolution)

- hover.rs: +235 lines, richer local/member resolution
- expr.rs: +130, parser support for hover-visible constructs
- parser.rs: +102, local decl parsing
- workspace 100 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline -1