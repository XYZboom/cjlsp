// cj-diag: diagnostics — text (SCAN) formatting matching official cjc output.
//
// Format (from official SCAN blocks):
//   error: <message>
//    ==> <file>:<line>:<col>:
//     |
//   N | <source line>
//     | ^^ <here message>          (^ count = span width)
//     |
//     # note: <note message>
//
//   <n> errors generated, <n> errors printed.

pub mod templates;

pub use templates::DiagId;

use std::fmt::Write;

use serde::Serialize;

/// Severity of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Note,
    Hint,
    Warning,
    Error,
    Fatal,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Note => "note",
            Severity::Hint => "hint",
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Fatal => "fatal error",
        }
    }
}

/// What kind of declaration an unused-symbol quickfix deletes. Drives the
/// source-text computation of the deletion range in the LSP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixKind {
    /// `func` with a braced body (incl. `main`, finalizer `~init`)
    Func,
    Class,
    Interface,
    Struct,
    Enum,
    /// `let`/`var` — top-level, member or local statement (single-line)
    Var,
    /// a function parameter (range covers name + type + adjacent comma)
    Param,
    /// type-named constructor inside a class-like body (title is "symbol")
    Symbol,
}

/// A quickfix attached to a diagnostic (official: `quickfix.removeUnusedSymbol`).
/// The LSP server fills in the concrete deletion range from the source text.
#[derive(Debug, Clone)]
pub struct DiagFix {
    pub title: String,
    pub kind: FixKind,
    /// 1-based start of the declaration (keyword/name; incl. any modifiers —
    /// the server extends backward over modifier tokens when needed).
    pub start_line: u32,
    pub start_col: u32,
}

/// A 1-based source position, as used by text diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// A source range whose end follows the existing exclusive span convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SourceRange {
    pub start: Position,
    pub end: Position,
}

/// A typed source location suitable for machine consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceLocation {
    pub file: String,
    pub range: SourceRange,
}

/// A secondary location related to a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelatedLocation {
    pub location: SourceLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A source edit proposed by a diagnostic suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextEdit {
    pub location: SourceLocation,
    pub replacement: String,
}

/// A general-purpose suggestion, optionally carrying applicable edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<TextEdit>,
}

/// A single diagnostic message with an optional source range.
#[derive(Debug, Clone)]
pub struct Diag {
    /// Stable string identifier; never an enum ordinal.
    pub code: Option<&'static str>,
    /// Stable producer category such as `parser` or `lexer`.
    pub category: Option<&'static str>,
    pub severity: Severity,
    pub message: String,
    /// 1-based line/col of the start of the highlighted range.
    pub line: u32,
    pub col: u32,
    /// 1-based end position (exclusive-ish; `^`s span from col to end_col-1).
    pub end_line: u32,
    pub end_col: u32,
    /// "expected X here"-style suffix shown after the carets (parser style).
    pub here: Option<String>,
    /// Notes appended under the caret block (# note: ...).
    pub notes: Vec<String>,
    /// LSP DiagnosticTag values (1 = Unnecessary); empty for non-tagged diags.
    pub tags: Vec<i32>,
    /// Optional quickfix (unused-symbol removal) the editor can apply.
    pub fix: Option<DiagFix>,
    pub related_locations: Vec<RelatedLocation>,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub candidates: Vec<String>,
    pub suggestions: Vec<Suggestion>,
}

impl Diag {
    pub fn error(line: u32, col: u32, message: impl Into<String>) -> Self {
        Diag {
            code: None,
            category: None,
            severity: Severity::Error,
            message: message.into(),
            line,
            col,
            end_line: line,
            end_col: col,
            here: None,
            notes: Vec::new(),
            tags: Vec::new(),
            fix: None,
            related_locations: Vec::new(),
            expected: None,
            actual: None,
            candidates: Vec::new(),
            suggestions: Vec::new(),
        }
    }

    pub fn warning(line: u32, col: u32, message: impl Into<String>) -> Self {
        Diag {
            code: None,
            category: None,
            severity: Severity::Warning,
            message: message.into(),
            line,
            col,
            end_line: line,
            end_col: col,
            here: None,
            notes: Vec::new(),
            tags: Vec::new(),
            fix: None,
            related_locations: Vec::new(),
            expected: None,
            actual: None,
            candidates: Vec::new(),
            suggestions: Vec::new(),
        }
    }

    pub fn with_span(mut self, end_line: u32, end_col: u32) -> Self {
        self.end_line = end_line;
        self.end_col = end_col;
        self
    }

    pub fn with_here(mut self, here: impl Into<String>) -> Self {
        self.here = Some(here.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_id(mut self, id: DiagId) -> Self {
        self.code = Some(id.code());
        self.category = Some(id.category());
        self
    }

    pub fn source_range(&self) -> SourceRange {
        SourceRange {
            start: Position {
                line: self.line,
                column: self.col,
            },
            end: Position {
                line: self.end_line,
                column: self.end_col,
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonDiagnostic<'a> {
    severity: Severity,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'static str>,
    location: SourceLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    here: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: &'a Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actual: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    candidates: &'a Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related_locations: &'a Vec<RelatedLocation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suggestions: &'a Vec<Suggestion>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonDocument<'a> {
    schema_version: u32,
    diagnostics: Vec<JsonDiagnostic<'a>>,
}

/// Renders a deterministic compact JSON document for IDE and AI consumers.
pub struct JsonFormatter<'a> {
    pub file_name: &'a str,
}

impl JsonFormatter<'_> {
    pub fn render(&self, diagnostics: &[Diag]) -> serde_json::Result<String> {
        let diagnostics = diagnostics
            .iter()
            .map(|diag| JsonDiagnostic {
                severity: diag.severity,
                message: &diag.message,
                code: diag.code,
                category: diag.category,
                location: SourceLocation {
                    file: self.file_name.to_string(),
                    range: diag.source_range(),
                },
                here: diag.here.as_deref(),
                notes: &diag.notes,
                expected: diag.expected.as_deref(),
                actual: diag.actual.as_deref(),
                candidates: &diag.candidates,
                related_locations: &diag.related_locations,
                suggestions: &diag.suggestions,
            })
            .collect();
        serde_json::to_string(&JsonDocument {
            schema_version: 1,
            diagnostics,
        })
    }
}

/// Formats diagnostics in the official text/SCAN format.
pub struct TextFormatter<'a> {
    pub file_name: &'a str,
    /// Source lines (1-indexed by position). Needed to render the code line.
    pub source_lines: &'a [String],
}

impl<'a> TextFormatter<'a> {
    /// Render one diagnostic as the official multi-line SCAN block.
    pub fn render(&self, d: &Diag) -> String {
        let mut out = String::new();
        // header: "error: <message>"
        let _ = writeln!(out, "{}: {}", d.severity.label(), d.message);

        // location: " ==> file:line:col:"
        let _ = writeln!(out, " ==> {}:{}:{}:", self.file_name, d.line, d.col);

        let line_text = self
            .source_lines
            .get((d.line as usize).saturating_sub(1))
            .map(String::as_str)
            .unwrap_or("");

        // caret width: at least 1, from col to end_col
        let width = if d.end_line == d.line && d.end_col > d.col {
            (d.end_col - d.col).max(1) as usize
        } else {
            1
        };

        // "  | " empty spacer line
        let _ = writeln!(out, "  | ");
        // "N | <source>"
        let _ = writeln!(out, "{} | {}", d.line, line_text);
        // "  | ^^^ <here>"
        let here_suffix = match &d.here {
            Some(h) => format!(" {h}"),
            None => String::new(),
        };
        let _ = writeln!(out, "  | {}{}", "^".repeat(width), here_suffix);
        // "  | " closing spacer
        let _ = writeln!(out, "  | ");

        // notes
        for note in &d.notes {
            let _ = writeln!(out, "  # note: {note}");
        }

        out
    }
}

/// Renders the final summary line(s) after a batch of diagnostics.
pub fn render_summary(errors: usize, warnings: usize) -> String {
    let mut out = String::new();
    if errors > 0 {
        let _ = writeln!(out, "{errors} errors generated, {errors} errors printed.");
    }
    if warnings > 0 {
        let _ = writeln!(out, "{warnings} warnings generated.");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_simple_error() {
        let lines: Vec<String> = vec!["@!C struct Foo {}".to_string()];
        let f = TextFormatter {
            file_name: "test.cj",
            source_lines: &lines,
        };
        let d = Diag::error(1, 1, "expected declaration, found '@!'")
            .with_span(1, 3)
            .with_here("expected declaration here")
            .with_note("only declarations or macro expressions can be used in the top-level");
        let out = f.render(&d);
        let expected = "\
error: expected declaration, found '@!'
 ==> test.cj:1:1:
  | 
1 | @!C struct Foo {}
  | ^^ expected declaration here
  | 
  # note: only declarations or macro expressions can be used in the top-level
";
        assert_eq!(out, expected);
    }

    #[test]
    fn render_multi_diag_with_summary() {
        let lines: Vec<String> = vec!["a".to_string(), "b".to_string()];
        let f = TextFormatter {
            file_name: "x.cj",
            source_lines: &lines,
        };
        let d1 = Diag::error(1, 1, "first");
        let d2 = Diag::error(2, 1, "second");
        let mut out = f.render(&d1);
        out.push_str(&f.render(&d2));
        out.push_str(&render_summary(2, 0));
        assert!(out.contains("2 errors generated, 2 errors printed."));
        assert!(out.matches("error:").count() >= 2);
    }

    #[test]
    fn json_is_structured_and_omits_absent_fields() {
        let mut d = Diag::error(2, 3, "expected declaration")
            .with_span(2, 5)
            .with_id(DiagId::PARSE_EXPECTED_DECL);
        d.expected = Some("declaration".into());
        d.actual = Some("expression".into());
        d.candidates.push("func declaration".into());
        d.suggestions.push(Suggestion {
            message: "replace the token".into(),
            edits: vec![TextEdit {
                location: SourceLocation {
                    file: "test.cj".into(),
                    range: d.source_range(),
                },
                replacement: "func".into(),
            }],
        });

        let json = JsonFormatter {
            file_name: "test.cj",
        }
        .render(&[d])
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["diagnostics"][0]["code"], "parse_expected_decl");
        assert_eq!(value["diagnostics"][0]["category"], "parser");
        assert_eq!(
            value["diagnostics"][0]["location"]["range"]["start"]["line"],
            2
        );
        assert!(value["diagnostics"][0].get("relatedLocations").is_none());
        assert!(!json.contains(":null"));
    }
}
