#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang/.worktrees/t_c847f371
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T17: completion alignment - rich item generation + context triggers

- completion.rs: 200 -> 1800 lines; richer CompletionItems (kind/detail/
  sortText/docs), member-access completion, keyword snippets, import-aware
  candidates, trigger-context handling
- server.rs: completion handler wiring
- workspace 91 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline | head -2