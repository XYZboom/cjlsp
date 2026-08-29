# Changelog

## 0.1.2
- Error recovery: lex/syntax errors no longer stop later analysis (T55)
- Cross-file hover (cjpm projects): hover on cross-file type/function/member
  shows its declaration info (T54)
- Semantic highlighting enhanced: primitive types (Int64/Float64/...) as
  `type`, soft keywords (mut/unsafe/operator/...) as `modifier`
- Completion improved to 85.7% (138/161) (T57)
- Normalized cross-package jump URIs (no ../ segments)
- Stdlib tooling: tools/stdlib_download.py + tools/stdlib_index.py

## 0.1.1
- Cross-file definition (cjpm projects): jump to symbols in other files
- Bare main() entry: locals now hoverable/jumpable
- Semantic highlighting (semanticTokens/full)
- Plugin startup fixes (output channel + Logger interface)
- Renamed to cj-lang-oss.cangjie-lsp to avoid conflict with official ext

## 0.1.0
- Initial release: completion/hover/definition/references, dual-platform
  bundled binaries
