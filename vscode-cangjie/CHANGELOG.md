# Changelog

## 0.1.1 - 2026-08-29

Cross-platform regression (Linux + Windows GNU cross-build) + refreshed bundled LSPServer binaries.

- Refreshed both bundled binaries from committed master (T46 LLT 86.8%, T48 Windows macro dynamic loading, T47 completion 74.5%): linux `LSPServer`, win32 `LSPServer.exe` (previously stale).
- Renamed extension to `cj-lang-oss.cangjie-lsp` to avoid clashing with the official `cangjie-lang.vscode-cangjie` extension.
- Verified with the full 13-step CI gate (tools/ci.sh) — 13/13 green, including Windows GNU cross-build and clippy on both targets.

## 0.1.0 - 2026-08-29

Initial release.

- Cangjie language support for VSCode via the Rust cj-lsp LSPServer (stdio).
- Features: diagnostics, completion, hover, definition, references.
- Bundles per-platform LSPServer binaries (linux / win32), zero-config install.
- Customizable via `cangjie.lsp.serverPath` / `cangjie.lsp.extraArgs` / `cangjie.lsp.env` (override the bundled default only).
