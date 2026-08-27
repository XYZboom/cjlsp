#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "T4b: macro SDK compile cache (performance for LSP)

- macro_cache.rs: three-layer cache per user's performance requirement
  1. Macro-package compile cache: sha256(source) -> .so path; source unchanged
     = reuse, no cjc invocation (cjc takes seconds)
  2. Expansion-result cache: (macro name + args hash) -> expanded text, LRU
     capped at 256 entries; deterministic macro expansion hits fast path
  3. LSP-session: in-memory cache keyed by file uri, so didChange only
     recomputes changed file's expansions, reusing .so loads
- compile_macro_package: shell out to cjpm via SDK envsetup, copy .so to
  cache dir <sdk>/../.macro-cache/ keyed by content hash
- expand_cached: LRU with eviction, closure only called once per key
- 3 unit tests (key determinism, LRU cache hit, source hash changes)
- workspace 83 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline | head -3