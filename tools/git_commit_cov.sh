#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "tools: lsp_cov.py diagnostics coverage checker

Fast cjlsp coverage: extract expected diagnostics from .info Rev#, drive our
LSPServer with initialize+didOpen (text from real source file), match
message+line+char. Baseline: 36/125 (28.8%) diagnostics matched." 2>&1 | tail -1