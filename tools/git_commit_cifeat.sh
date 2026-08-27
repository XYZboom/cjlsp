#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "ci: add LSP feature gate (completion/hover)

- ci.sh step 7: FEATURE_FULL=1 runs full suite, default smoke (--limit 2)
- Uses T16's run_feature_cases.py; extends CI coverage beyond diagnostics" 2>&1 | tail -1
git log --oneline | head -2