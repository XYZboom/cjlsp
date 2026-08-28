#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_1f379622
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T31: completion alignment round 4 (member-access type inference, 1316 ln)

- completion.rs: member-access completion with receiver type inference
  (let x: C = ... then x. completes C's members), private-member filtering,
  string-literal handling refinement
- server.rs: wiring for the richer completion
- workspace 100 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline -1