// cj-sema: unused-declaration analysis.
//
// Reports declarations that are never referenced:
//   Function 'test01' is declared but never used
//   Variable 'y' is declared but never used
//   Parameter 'a' is declared but never used
//   Class 'B12' is declared but never used
//   Interface 'MyInterface' is declared but never used
//
// Severity is Hint (LSP severity 4), matching the official cjlsp diagnostics
// suite. Detection is per-file: collect every top-level declaration name and
// every identifier reference in the file, then report decls never referenced.

use cj_ast::{Body, Decl, Expr, File, Pattern};
use cj_diag::{Diag, Severity};
use std::collections::HashSet;

/// Kind label used in the diagnostic message ("Function", "Variable", ...).
#[derive(Debug, Clone, Copy)]
enum DeclKind {
    Func,
    Class,
    Interface,
    Struct,
    Enum,
    Var,
}

impl DeclKind {
    fn label(self) -> &'static str {
        match self {
            DeclKind::Func => "Function",
            DeclKind::Class => "Class",
            DeclKind::Interface => "Interface",
            DeclKind::Struct => "Struct",
            DeclKind::Enum => "Enum",
            DeclKind::Var => "Variable",
        }
    }
}

/// Collect identifiers referenced by an expression tree into `out`.
fn collect_refs(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::Name { name, .. } => {
            out.insert(name.clone());
        }
        Expr::Call { callee, args, .. } => {
            collect_refs(callee, out);
            for a in args {
                collect_refs(&a.value, out);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_refs(lhs, out);
            collect_refs(rhs, out);
        }
        Expr::Unary { inner, .. } => collect_refs(inner, out),
        Expr::Paren { inner, .. } => collect_refs(inner, out),
        Expr::Member { object, .. } => collect_refs(object, out),
        Expr::Subscript { object, index, .. } => {
            collect_refs(object, out);
            collect_refs(index, out);
        }
        Expr::Assign { lhs, rhs, .. } => {
            collect_refs(lhs, out);
            collect_refs(rhs, out);
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                collect_refs(s, out);
            }
        }
        Expr::If {
            cond, then, els, ..
        } => {
            collect_refs(cond, out);
            collect_refs(then, out);
            if let Some(e) = els {
                collect_refs(e, out);
            }
        }
        Expr::Lambda { params, body, .. } => {
            for p in params {
                if let Some(d) = &p.default {
                    collect_refs(d, out);
                }
            }
            collect_refs(body, out);
        }
        Expr::LetPatternDestructor {
            initializer,
            patterns,
            ..
        } => {
            collect_refs(initializer, out);
            for p in patterns {
                collect_pattern_refs(p, out);
            }
        }
        _ => {}
    }
}

fn collect_pattern_refs(_p: &Pattern, _out: &mut HashSet<String>) {
    // A Var pattern binds a new name (not a reference); ignore for used-set.
}

/// Detect unused declarations. `file` is the parsed File; returns diagnostics.
pub fn detect_unused(file: &File) -> Vec<Diag> {
    let mut diags = Vec::new();

    // --- Pass 1: top-level declarations and their positions ---
    // name -> (kind, line, col)
    let mut top_decls: Vec<(String, DeclKind, u32, u32)> = Vec::new();
    // A decl whose body/initializer references an undefined name is not fully
    // analyzable, so its unused report is skipped (matches the official suite).
    let mut unresolved_names: Vec<String> = Vec::new();
    // Known top-level names (for the unresolved-reference check).
    let mut known: HashSet<String> = HashSet::new();
    for d in &file.decls {
        match d {
            Decl::Func { name, .. }
            | Decl::Class { name, .. }
            | Decl::Interface { name, .. }
            | Decl::Struct { name, .. }
            | Decl::Enum { name, .. }
            | Decl::TypeAlias { name, .. }
            | Decl::Var { name, .. } => {
                known.insert(name.clone());
            }
            _ => {}
        }
    }

    for d in &file.decls {
        match d {
            Decl::Func {
                name,
                name_pos,
                body,
                params,
                ..
            } => {
                top_decls.push((name.clone(), DeclKind::Func, name_pos.line, name_pos.col));
                // Params used only within their own function body.
                let mut body_refs: HashSet<String> = HashSet::new();
                if let Body::Block(stmts) = body {
                    for s in stmts {
                        collect_refs(s, &mut body_refs);
                    }
                }
                // If the body references a name not known in this file and not
                // a builtin, treat the function as not fully analyzable and
                // skip its unused report.
                let unresolved: Vec<&String> = body_refs
                    .iter()
                    .filter(|r| !known.contains(*r) && !is_builtin(r))
                    .collect();
                if !unresolved.is_empty() {
                    unresolved_names.push(name.clone());
                }
                for p in params {
                    if !body_refs.contains(&p.name) {
                        diags.push(unused_diag(
                            p.pos.line,
                            p.pos.col,
                            format!("Parameter '{}' is declared but never used", p.name),
                        ));
                    }
                }
            }
            Decl::Class { name, pos, .. } => {
                top_decls.push((name.clone(), DeclKind::Class, pos.line, pos.col));
            }
            Decl::Interface { name, pos, .. } => {
                top_decls.push((name.clone(), DeclKind::Interface, pos.line, pos.col));
            }
            Decl::Struct { name, pos, .. } => {
                top_decls.push((name.clone(), DeclKind::Struct, pos.line, pos.col));
            }
            Decl::Enum { name, pos, .. } => {
                top_decls.push((name.clone(), DeclKind::Enum, pos.line, pos.col));
            }
            Decl::Var {
                name,
                name_pos,
                init,
                ..
            } => {
                top_decls.push((name.clone(), DeclKind::Var, name_pos.line, name_pos.col));
                if let Some(init) = init {
                    let mut init_refs: HashSet<String> = HashSet::new();
                    collect_refs(init, &mut init_refs);
                    let unresolved: Vec<&String> = init_refs
                        .iter()
                        .filter(|r| !known.contains(*r) && !is_builtin(r))
                        .collect();
                    if !unresolved.is_empty() {
                        unresolved_names.push(name.clone());
                    }
                }
            }
            _ => {}
        }
    }

    // --- Pass 2: collect every referenced identifier in the file ---
    let mut used: HashSet<String> = HashSet::new();
    for d in &file.decls {
        collect_decl_refs(d, &mut used);
    }

    // --- Pass 3: report top-level decls not referenced (except main) ---
    for (name, kind, line, col) in &top_decls {
        if name == "main" {
            continue; // entry point is implicitly used
        }
        if !used.contains(name) && !unresolved_names.contains(name) {
            diags.push(unused_diag(
                *line,
                *col,
                format!("{} '{}' is declared but never used", kind.label(), name),
            ));
        }
    }

    diags
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "println"
            | "assert"
            | "String"
            | "Array"
            | "VArray"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "Float32"
            | "Float64"
            | "Bool"
            | "Unit"
            | "Nothing"
            | "Rune"
            | "Char"
            | "Object"
            | "Range"
            | "Pair"
            | "Tuple"
    )
}

/// Collect all identifier references in a declaration (for the used set).
fn collect_decl_refs(d: &Decl, used: &mut HashSet<String>) {
    match d {
        Decl::Func {
            body: Body::Block(stmts),
            ..
        } => {
            for s in stmts {
                collect_refs(s, used);
            }
        }
        Decl::Var {
            init: Some(init), ..
        } => collect_refs(init, used),
        _ => {}
    }
}

fn unused_diag(line: u32, col: u32, message: String) -> Diag {
    Diag {
        severity: Severity::Hint,
        message,
        line,
        col,
        end_line: line,
        end_col: col,
        here: None,
        notes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cj_parser::parse_source;

    #[test]
    fn unused_function_and_params() {
        let (file, _) = parse_source("func test01(a: Int8, b: Bool) {}\n");
        let diags = detect_unused(&file);
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(
            msgs.iter().any(|m| m.contains("Function 'test01'")),
            "expected unused function: {msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("Parameter 'a'")),
            "expected unused param a: {msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("Parameter 'b'")),
            "expected unused param b: {msgs:?}"
        );
    }

    #[test]
    fn used_function_not_reported() {
        let (file, _) = parse_source("func main() { helper() }\nfunc helper() {}\n");
        let diags = detect_unused(&file);
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(
            !msgs.iter().any(|m| m.contains("'helper'")),
            "helper is used: {msgs:?}"
        );
    }

    #[test]
    fn severity_is_hint() {
        let (file, _) = parse_source("func test01() {}\n");
        let diags = detect_unused(&file);
        assert_eq!(diags[0].severity, Severity::Hint);
    }
}
