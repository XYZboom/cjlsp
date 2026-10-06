# Changelog

All notable changes to the `cj-lang` (Cangjie Rust Frontend and LSP Server) project will be documented in this file.

## [Unreleased] - 2026-10-06

### Added
- **Parser Syntax Diagnostics and Error Recovery**:
  - Unclosed delimiter recovery for `(`, `{`, `[`, and `${}` with stable error codes (`parse_unclosed_delimiter`, `parse_expected_right_delimiter`).
  - Robust synchronization and error recovery to declaration boundaries to avoid cascading diagnostics.
  - Recovery and diagnostics for missing conditional expressions in `if` and `while` statements.
  - Diagnostics for invalid top-level and missing declaration keywords (`parse_expected_declaration`).
- **Sema Diagnostics and Structured Analysis**:
  - Overload resolution conflict diagnostics populated with candidate function signatures in `candidates` (`sema_ambiguous_call`).
  - Symbol redefinition diagnostics now provide first-declared origin locations via `related_locations` (`sema_redefinition`).
  - Precise type mismatch reporting with `expected` and `actual` semantic type representations.
  - Boundary and range check diagnostics for integer and character literals.
  - Visibility checks reporting inaccessible or unexported symbols across packages.
- **Structured JSON Diagnostic Projection for IDE & AI**:
  - Standardized JSON payload under `data.cjlsp` (`schemaVersion`, `code`, `category`, `expected`, `actual`, `candidates`, `suggestions`).
  - Full backward compatibility with Cangjie official test suite (preserving top-level `code: null` and `data.codeActions`).
  - Mapping `related_locations` directly to LSP `DiagnosticRelatedInformation`.
- **In-Tree Diagnostic Fixtures & Regression Gates**:
  - Self-contained golden diagnostic fixtures in `tests/diagnostics/` covering parser, sema, overload, type mismatch, and generic scopes.
  - Exact JSON regression testing integration in `crates/cj-frontend/tests/diagnostic_fixtures.rs`.
  - Comprehensive contributing guide in `CONTRIBUTING.md`.
  - Continuous regression gate and Criterion benchmark enforcement (`<= 1.05x` execution time relative to baseline).
- **LSP Feature Enhancements**:
  - Full `textDocument/signatureHelp` implementation passing 100% (32/32) of official HLT test cases.
  - Full `textDocument/completion` suite passing 100% (161/161) of official test cases.
  - Full `textDocument/hover` suite passing 100% (181/181) of official test cases.

### Fixed
- Fixed parser operator function declaration handling when `operator` modifier is used (`operator func +(...)`).
- Fixed package resolution and wildcard imports (`import pkg.*`) in sibling workspace document scanning.
- Fixed comment block line endings normalization for Markdown hovers.
- Fixed memory and CPU overhead in sema overload resolution by reducing redundant allocations.

### Performance
- Full diagnostic dense pipeline: **1.0102x** baseline (1.3858ms vs 1.3718ms).
- Full diagnostic large valid pipeline: **0.9587x** baseline (1.7435ms vs 1.8187ms, faster than baseline).
- Met and passed the `<= 1.05x` performance gate requirement.
