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

/// Severity of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A single diagnostic message with an optional source range.
#[derive(Debug, Clone)]
pub struct Diag {
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
}

impl Diag {
    pub fn error(line: u32, col: u32, message: impl Into<String>) -> Self {
        Diag {
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
        }
    }

    pub fn warning(line: u32, col: u32, message: impl Into<String>) -> Self {
        Diag {
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
}
