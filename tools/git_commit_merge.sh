#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
git add -A
git -c user.name="cj-rust" -c user.email="cj-rust@local" commit -m "Merge T2+T3: package diagnostics + member-unused/overload conflicts

- T2 (wt/t2-package): package name consistency, circular deps, unused
  import, can-not-find-package (cj-sema/package.rs)
- T3 (wt/t3-unused-members): unused extended to struct/class/interface
  members + function overload-conflict detection (unused.rs rework)
- Conflict resolution:
  * lib.rs keep expander+package+overload mods
  * unused.rs take T3 full version (has name_pos), expose Refs/collect_decl_refs
  * package.rs adapt to Refs API
  * server.rs merge analyze_source(project_root, expected) unified signature,
    restore path var in publish_diagnostics
  * generated.rs keep variable-name doc
- lsp_cov coverage 28.8% -> 63.2% (79/125)
- workspace 68 tests, clippy -D warnings clean" 2>&1 | tail -1
git log --oneline | head -3