# Changelog

## 0.1.0 - 2026-08-29

Initial release.

- Cangjie language support for VSCode via the Rust cj-lsp LSPServer (stdio).
- Features: diagnostics, completion, hover, definition, references.
- Bundles per-platform LSPServer binaries (linux / win32), zero-config install.
- Customizable via `cangjie.lsp.serverPath` / `cangjie.lsp.extraArgs` / `cangjie.lsp.env` (override the bundled default only).
