# Changelog

## 0.1.5
- Standard library sources read-only: Ctrl+Click jump targets under
  ~/.cangjie-lsp/std/** are auto-folded into `files.readonlyInclude` (Global,
  VSCode >= 1.85) so downloaded std sources can't be accidentally edited; the
  root is resolved at runtime from the home directory (no hard-coded path),
  existing user patterns are preserved, and the write is idempotent (T61).

## 0.1.3
- Signature help: parameter list for func/ctor/method calls (add( → shows
  signature + active parameter)
- LLT improved to 88.2% (crash=0)

## 0.1.2
- Error recovery: lex/syntax errors no longer stop later analysis
- Cross-file hover (cjpm projects): hover on cross-file symbols
- Semantic highlighting enhanced: primitive types + soft keywords
- Completion improved to 85.7% (138/161)
- Normalized cross-package jump URIs

## 0.1.1
- Cross-file definition (cjpm projects): jump to symbols in other files
- Bare main() entry: locals now hoverable/jumpable
- Semantic highlighting (semanticTokens/full)
- Plugin startup fixes

## 0.1.0
- Initial release

## 0.1.4
- Strip release binaries: LSPServer 2.27→1.86MB, .exe 3.25→2.38MB (faster
  plugin startup, issue #1)
