// cj-sema: package-level diagnostics — name consistency, import checks,
// circular-dependency detection.
//
// These checks are driven by the LSP per-file (the expected package name is
// derived from the file's URI). For single-file analysis the circular-dependency
// check only detects self-imports (package X imports X); multi-file would need
// the full workspace package graph.

use cj_ast::{File, ImportSpec};
use cj_diag::{Diag, Severity};
use std::collections::HashSet;

/// Run all package-level checks on a parsed file.
///
/// `expected` — the expected package name derived from the file's URI
/// (module.dirs, e.g. `diagnosticsTest.pkg_error`).  Pass `None` when the
/// expected package cannot be inferred (e.g. no `src/` in the path).
pub fn check_package(file: &File, expected: Option<&str>) -> Vec<Diag> {
    let mut diags = Vec::new();
    let own = file.package.as_deref();

    // 1. Package-name consistency
    if let Some(exp) = expected {
        let mismatch = match own {
            // No package decl at all: only flag when the file is in a sub-package
            // (expected contains a dot).  Root-package files may omit the decl.
            None => exp.contains('.'),
            Some(name) => name != exp,
        };
        if mismatch {
            let pos = file.package_pos.unwrap_or_default();
            // line/col must be at least 1 (default is 0 → LSP 0:0 after conversion)
            let line = pos.line.max(1);
            let col = pos.col.max(1);
            diags.push(Diag::error(
                line,
                col,
                format!("package name supposed to be '{}'", exp),
            ));
        }
    }

    // 2. Collect all referenced names in the file (for unused-import checks).
    let mut refs = crate::unused::Refs::default();
    for d in &file.decls {
        crate::unused::collect_decl_refs(d, &mut refs);
    }
    let used_names: &std::collections::HashSet<String> = &refs.names;

    // 3. Import checks (unused + can-not-find).
    for imp in &file.imports {
        let display = import_display(imp);
        let pkg = import_package_name(imp);
        let is_self = own.is_some() && pkg.as_deref() == own;

        // 3a. Unused import (skip self-imports — they report as circular instead).
        if !is_self && !import_is_used(imp, used_names) {
            let pos = imp.pos;
            let msg = format!(
                "unused import '{}', this warning can be suppressed by setting \
                 the compiler option `-Woff unused`",
                display
            );
            diags.push(Diag {
                severity: Severity::Hint,
                message: msg,
                line: pos.line,
                col: pos.col,
                end_line: pos.end_line,
                end_col: pos.end_col,
                here: None,
                notes: Vec::new(),
            });
        }

        // 3b. Can not find package (unknown package reference).
        if let Some(p) = &pkg {
            if !is_known_package(p, own) {
                let name_pos = imp.name_pos;
                diags.push(
                    Diag::error(
                        name_pos.line,
                        name_pos.col,
                        format!("can not find package '{}'", p),
                    )
                    .with_span(name_pos.end_line, name_pos.end_col),
                );
            }
        }
    }

    // 4. Circular dependency (from this file's package graph).
    //
    // Build edges: own → import-package for each import that resolves.
    // Single-file: a cycle exists iff own imports itself (self-import).
    // Multi-file would accumulate edges from all open files.
    let mut edges: Vec<(String, String)> = Vec::new();
    if let Some(own_str) = own {
        for imp in &file.imports {
            if let Some(to) = import_package_name(imp) {
                edges.push((own_str.to_string(), to));
            }
        }
        if let Some(cycle) = find_cycle_involving(&edges, own_str) {
            let names: Vec<String> = cycle;
            let msg = format!(
                "packages {} are in circular dependencies.",
                names.join(", "),
            );
            diags.push(Diag::error(1, 1, msg));
        }
    }

    diags
}

/// Display text for an import (e.g. `testpackage.*`, `std.ast.Body`).
fn import_display(imp: &ImportSpec) -> String {
    let base = imp.path.join(".");
    if imp.glob {
        format!("{}.*", base)
    } else if imp.selected.is_empty() {
        base
    } else {
        format!("{}: {}", base, imp.selected.join(", "))
    }
}

/// Resolve the package name referred to by an import.
///
/// For glob imports (`a.b.*`) the package is the full path `a.b`.
/// For single imports of a member (`a.b.C`) the package is the prefix `a.b`.
/// For single-segment imports (`pkg`) the package is the whole path.
fn import_package_name(imp: &ImportSpec) -> Option<String> {
    if imp.path.is_empty() {
        return None;
    }
    if imp.glob {
        // `a.b.*` → package = "a.b"
        Some(imp.path.join("."))
    } else if imp.path.len() >= 2 {
        // `a.b.C` → package = "a.b"
        Some(imp.path[..imp.path.len() - 1].join("."))
    } else {
        // single-segment `pkg` — treat whole path as the package
        Some(imp.path[0].clone())
    }
}

/// Is `pkg` a package that is known to exist in the environment?
///
/// Known packages: the file's own package + the standard library (`std.*`).
fn is_known_package(pkg: &str, own: Option<&str>) -> bool {
    if own.is_some_and(|o| o == pkg) {
        return true;
    }
    // The standard library packages are always available.
    pkg == "std" || pkg.starts_with("std.")
}

/// Is an import "used" — i.e. does any name reference in the file come from it?
fn import_is_used(imp: &ImportSpec, used: &HashSet<String>) -> bool {
    if imp.path.is_empty() {
        return true; // malformed import, don't report
    }
    let full = imp.path.join(".");
    if imp.glob {
        // Glob `a.b.*`: used if any reference is qualified through the package
        // (`a.b.X`) or equals a segment of the path.
        used.iter()
            .any(|n| n == &full || n.starts_with(&format!("{}.", full)) || imp.path.contains(n))
    } else {
        // Single import `a.b.C`: used if the member name (last segment) or the
        // full path appears in references.
        let member = imp.path.last().expect("non-empty path");
        used.iter()
            .any(|n| n == member || n == &full || n.starts_with(&format!("{}.", full)))
    }
}

/// Find a cycle involving `start` in the directed graph `edges`.
///
/// Returns the cycle nodes in order (starting and ending with `start`) if a
/// cycle is found, or `None` otherwise.
fn find_cycle_involving(edges: &[(String, String)], start: &str) -> Option<Vec<String>> {
    // Build adjacency list.
    let mut adj: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for (from, to) in edges {
        adj.entry(from.as_str()).or_default().push(to);
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut path: Vec<String> = Vec::new();

    fn dfs<'a>(
        node: &'a str,
        start: &str,
        adj: &std::collections::HashMap<&'a str, Vec<&'a str>>,
        visited: &mut HashSet<&'a str>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        if node == start && !path.is_empty() {
            // Found a cycle: `path` already starts with `start` (e.g. for a
            // self-import it is [start] — the doubled name must NOT be added).
            return Some(path.clone());
        }
        if !visited.insert(node) {
            return None; // already visited on this path, no new cycle
        }
        path.push(node.to_string());

        if let Some(neighbors) = adj.get(node) {
            for n in neighbors {
                if let Some(cycle) = dfs(n, start, adj, visited, path) {
                    return Some(cycle);
                }
            }
        }
        path.pop();
        visited.remove(node);
        None
    }

    dfs(start, start, &adj, &mut visited, &mut path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cj_parser::parse_source;

    #[test]
    fn package_name_ok() {
        // package name matches expected → no diagnostic.
        let (f, _) = parse_source("package a.b\n");
        let diags = check_package(&f, Some("a.b"));
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(diags.is_empty(), "expected no diagnostics, got: {msgs:?}");
    }

    #[test]
    fn package_name_mismatch() {
        let (f, _) = parse_source("package a.b\n");
        let diags = check_package(&f, Some("a.c"));
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(
            msgs.iter()
                .any(|m| m.contains("package name supposed to be")),
            "expected name mismatch: {msgs:?}"
        );
    }

    #[test]
    fn empty_file_no_package_decl() {
        let (f, _) = parse_source("");
        let diags = check_package(&f, Some("a.b"));
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(
            msgs.iter()
                .any(|m| m.contains("package name supposed to be")),
            "expected missing package diagnostic: {msgs:?}"
        );
    }

    #[test]
    fn unused_import_detected() {
        let (f, _) = parse_source("package pkg\nimport testpkg.*\n");
        let diags = check_package(&f, Some("pkg"));
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(
            msgs.iter().any(|m| m.contains("unused import 'testpkg.*'")),
            "expected unused import: {msgs:?}"
        );
    }

    #[test]
    fn can_not_find_package() {
        let (f, _) = parse_source("package pkg\nimport unknown.*\n");
        let diags = check_package(&f, Some("pkg"));
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(
            msgs.iter()
                .any(|m| m.contains("can not find package 'unknown'")),
            "expected can-not-find: {msgs:?}"
        );
    }

    #[test]
    fn self_import_circular() {
        let (f, _) = parse_source("package pkg\nimport pkg.*\n");
        let diags = check_package(&f, Some("pkg"));
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(
            msgs.iter().any(|m| m.contains("circular dependencies")),
            "expected circular-dependency: {msgs:?}"
        );
        // Self-import should NOT also report unused.
        let unused = msgs.iter().filter(|m| m.contains("unused import")).count();
        assert_eq!(
            unused, 0,
            "self-import should not be flagged as unused: {msgs:?}"
        );
    }

    #[test]
    fn known_std_package_not_reported() {
        let (f, _) = parse_source("package pkg\nimport std.ast.Body\n");
        let diags = check_package(&f, Some("pkg"));
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        // std.ast is known → no can-not-find.
        let cnf = msgs.iter().filter(|m| m.contains("can not find")).count();
        assert_eq!(
            cnf, 0,
            "stdlib package should not be reported as unknown: {msgs:?}"
        );
    }
}
