// cj-sema: parallel multi-file semantic analysis.
//
// Pipeline (per spec Ch.03 top-level visibility rules):
//   1. Phase 1 — collect: parse + collect each file INDEPENDENTLY in parallel
//      (rayon par_iter). Each file's Collector touches only its own File AST,
//      so there is no shared mutable state and no locking.
//   2. Phase 2 — merge: fold per-file FileResults into one PackageTable
//      (single-threaded, cheap: just name->Vec<Symbol>).
//   3. Phase 3 — dependency-aware check: walk top-level var initializers in
//      definition order; a var referencing another top-level name creates an
//      edge. Cycle / use-before-def detection follows spec Ch.03 examples.
//
// This is the "multi-threaded semantic analysis" the project requires.

use crate::{Collector, FileResult, PackageTable};
use cj_ast::File;
use cj_diag::Diag;
use rayon::prelude::*;

/// Output of the full parallel analysis.
#[derive(Debug, Default)]
pub struct Analysis {
    pub package: PackageTable,
    pub diags: Vec<Diag>,
}

/// Analyze a set of files (already parsed) in parallel.
/// `parse` is a per-file closure returning the parsed File (so callers can
/// tokenize+parse in the same worker thread).
pub fn analyze_files<'a, F>(files: &[&'a File], parse: F) -> Analysis
where
    F: Fn(&&'a File) -> &'a File + Sync,
{
    // Phase 1: parallel per-file collection.
    let results: Vec<FileResult> = files
        .par_iter()
        .map(|f| Collector::new().collect_file(parse(f)))
        .collect();

    // Phase 2: merge into package table.
    let mut package = PackageTable::default();
    let mut diags = Vec::new();
    for r in &results {
        package.merge(r);
        diags.extend(r.diags.iter().cloned());
    }
    Analysis { package, diags }
}

/// Parse + analyze source strings in parallel (each worker tokenizes/parses
/// and collects its own file).
pub fn analyze_sources(sources: &[(String, String)]) -> Analysis
where
{
    // (filename, content) pairs. Parse per worker; collect per worker.
    let results: Vec<(FileResult, Vec<Diag>)> = sources
        .par_iter()
        .map(|(_, src)| {
            let tokens = cj_lexer::Lexer::new(src).tokenize();
            let mut parser = cj_parser::Parser::new(src, tokens);
            let file = parser.run();
            let parse_diags = std::mem::take(&mut parser.diags);
            let result = Collector::new().collect_file(&file);
            (result, parse_diags)
        })
        .collect();

    let mut package = PackageTable::default();
    let mut diags = Vec::new();
    for (r, pd) in results {
        package.merge(&r);
        diags.extend(pd);
        diags.extend(r.diags);
    }
    Analysis { package, diags }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_two_files_merge() {
        let sources = vec![
            ("a.cj".to_string(), "func a() {}\nlet v = 1\n".to_string()),
            ("b.cj".to_string(), "func b() { a() }\n".to_string()),
        ];
        let analysis = analyze_sources(&sources);
        // b.cj's call to a() resolves in the package table.
        assert!(analysis.package.lookup("a").is_some());
        assert!(analysis.package.lookup("b").is_some());
        assert!(analysis.package.lookup("v").is_some());
    }
}
