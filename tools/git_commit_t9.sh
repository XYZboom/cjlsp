#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T9: automated test pipeline - tools/ci.sh (all gates in one command)

- tools/ci.sh: fmt check -> clippy -D warnings -> unit tests -> lsp_cov ->
  macro E2E -> SCAN Parser alignment; each step prints PASS/FAIL with the
  measurable number; exits non-zero on first failing gate
- SCAN_DIR default: LLT/compiler/Parser (282 SCAN-block files)
- Verified: 6/6 PASS (fmt, clippy 0 warnings, 83 tests, lsp_cov 80.0%,
  macro E2E, scan_compare 85.7%)
- Usage: ./tools/ci.sh [-j N]; developer runs one command before pushing" 2>&1 | tail -1
git log --oneline | head -2