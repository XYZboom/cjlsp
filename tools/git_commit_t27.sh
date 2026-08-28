#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add crates/cj-lsp/Cargo.toml crates/cj-lsp/benches/ crates/cj-lsp/tests/samples/ tools/bench_baseline.txt
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T27: frontend performance baseline (criterion)

- bench_frontend.rs: lexer/parser/pipeline benches (criterion 0.5)
- sample: LLT big_chir.cj (95KB) + synthesized fallback
- baseline: tokenize 1.53ms, parse 4.31ms, pipeline 5.00ms (95KB)
- tools/bench_baseline.txt records baseline + CI regression threshold
- no perf regression from LLT coverage work (61%->80.8%)" 2>&1 | tail -1
git log --oneline -1