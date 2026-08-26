#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "M5: LSP diagnostics pipeline - first official cjlsp case passes

- read_message tolerates blank lines between frames (lsp_test.py emits them)
- initialize response wrapped with id+jsonrpc (was missing id, broke routing)
- didOpen/didChange store in-memory text; analyze_source prefers it, falls back to disk
- LSP range: 1-based to 0-based, end.character = col+1 (exclusive)
- diagnostics include category/code/codeActions/data keys (harness checks key existence)
- textDocument_diagnostics_003.info PASSES via official lsp_test.py" 2>&1 | tail -1
git log --oneline | head -2
