//! Reusable Cangjie frontend diagnostics pipeline.
//!
//! The CLI, benchmarks, and tests use this entry point so performance tests
//! exercise the same lexer, parser, semantic checks, and text formatting.

use cj_diag::{Diag, Severity, TextFormatter};
use cj_lexer::Lexer;
use cj_parser::Parser;

/// Complete output from one source-file diagnostic pass.
#[derive(Debug)]
pub struct DiagnosticOutput {
    pub diagnostics: Vec<Diag>,
    pub rendered: String,
    pub errors: usize,
    pub warnings: usize,
}

/// Run the complete in-process frontend diagnostic pipeline for one file.
///
/// This excludes project scanning and cross-file signature collection. Macro
/// expansion still runs; the checked-in benchmark corpus has no external macro
/// dependency, so measurements remain hermetic.
pub fn analyze_source(src: &str, file_name: &str) -> DiagnosticOutput {
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize();
    let lex_diags = std::mem::take(&mut lexer.errors)
        .into_iter()
        .map(|error| Diag::error(error.pos.line, error.pos.column, error.message));

    let mut parser = Parser::new(src, tokens);
    let file = parser.run();

    let sema_result = cj_sema::Collector::new().collect_file(&file);
    let mut package = cj_sema::PackageTable::default();
    package.merge(&sema_result);

    let mut resolver = cj_sema::resolver::Resolver::new(&package);
    resolver.resolve_file(&file);

    let mut diagnostics: Vec<Diag> = lex_diags.collect();
    diagnostics.extend(parser.diags.iter().cloned());
    diagnostics.extend(sema_result.diags.iter().cloned());
    diagnostics.extend(resolver.take_diags());
    diagnostics.extend(cj_sema::dep_graph::DepGraph::build(&[&file]).detect_cycles());
    diagnostics.extend(cj_sema::unused::detect_unused(&file));
    diagnostics.extend(cj_sema::typecheck::check_decls(&file));
    diagnostics.extend(cj_sema::typecheck::check_calls(
        &file,
        &sema_result.func_sigs,
    ));
    diagnostics.extend(cj_sema::package::check_package(&file, None));
    diagnostics.extend(cj_sema::overload::detect_overload_conflicts(&file));
    diagnostics.extend(cj_sema::checks::check_semantics(&file, &package, Some(src)));
    let mut macro_cache = cj_sema::macro_cache::MacroCache::new();
    let (_, macro_diags) =
        cj_sema::expander::expand_file_with_cache(&file, file_name, &mut macro_cache, None);
    diagnostics.extend(macro_diags);

    let source_lines: Vec<String> = src.lines().map(String::from).collect();
    let formatter = TextFormatter {
        file_name,
        source_lines: &source_lines,
    };
    let mut rendered = String::new();
    let mut errors = 0;
    let mut warnings = 0;
    for diagnostic in &diagnostics {
        match diagnostic.severity {
            Severity::Warning => warnings += 1,
            Severity::Error | Severity::Fatal => errors += 1,
            Severity::Note | Severity::Hint => {}
        }
        rendered.push_str(&formatter.render(diagnostic));
    }
    rendered.push_str(&cj_diag::render_summary(errors, warnings));

    DiagnosticOutput {
        diagnostics,
        rendered,
        errors,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_runs_sema_and_formats_diagnostics() {
        let output = analyze_source(
            "package perf\nfunc duplicate(): Int64 { return 1 }\nfunc duplicate(): Int64 { return 2 }\n",
            "broken.cj",
        );

        assert!(!output.diagnostics.is_empty());
        assert!(output.rendered.contains("==> broken.cj:"));
        assert!(output.errors > 0);
    }
}
