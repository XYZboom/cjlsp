#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_a5dd8a7e
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T28: completion alignment round 3 (cross-file + param defaults + edits)

- completion.rs: keyword snippets (func params, forward/true/false),
  literal default source in param detail, sibling_candidates() wrapper
- server.rs: collect_same_package_candidates (cross-file completion),
  apply_incremental_change boundary fixes (invalid/over-long ranges)
- workspace 100 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline -1