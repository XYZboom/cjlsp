// cj-lsp: textDocument/references support.
//
// Given a cursor position on a name, find all references to that name in the
// file (declaration + usages) and return LSP Location[].

use cj_ast::{Body, Decl, Expr, File};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Collect all references to the name under the cursor.
/// `include_decl`: whether to include the declaration itself.
pub fn references_at(
    file: &File,
    uri: &str,
    line: u32,
    character: u32,
    include_decl: bool,
) -> Value {
    // 1. Find the name at the cursor (declaration name or a Name expression).
    let target = name_at(file, line, character);
    let Some(target) = target else {
        return json!([]);
    };

    // 2. Collect declaration position(s) and all Name-expression positions.
    let mut locs: Vec<(u32, u32, u32, u32)> = Vec::new(); // (line0, col0, end_col0, decl_flag)
    let mut seen: HashSet<(u32, u32, u32)> = HashSet::new();

    let mut push_loc =
        |locs: &mut Vec<(u32, u32, u32, u32)>, line0: u32, col0: u32, end0: u32, is_decl: bool| {
            if seen.insert((line0, col0, end0)) {
                locs.push((line0, col0, end0, is_decl as u32));
            }
        };

    // Declarations (name_pos is 1-based).
    if include_decl {
        for d in &file.decls {
            if let Some((name, npos)) = decl_name_pos(d) {
                if name == target {
                    push_loc(
                        &mut locs,
                        npos.line.saturating_sub(1),
                        npos.col.saturating_sub(1),
                        npos.end_col.saturating_sub(1),
                        true,
                    );
                }
            }
        }
    }

    // Name expressions across the file.
    for d in &file.decls {
        collect_name_refs(d, &target, &mut locs);
    }

    // 3. Sort by position, emit Locations.
    locs.sort();
    let out: Vec<Value> = locs
        .iter()
        .map(|(l0, c0, e0, _)| {
            json!({
                "uri": uri,
                "range": {
                    "start": {"line": l0, "character": c0},
                    "end": {"line": l0, "character": e0},
                }
            })
        })
        .collect();
    json!(out)
}

/// Name of the declaration at the cursor, or the Name-expression at the cursor.
pub(crate) fn name_at(file: &File, line: u32, character: u32) -> Option<String> {
    for d in &file.decls {
        if let Some((name, npos)) = decl_name_pos(d) {
            let l0 = npos.line.saturating_sub(1);
            let c0 = npos.col.saturating_sub(1);
            let e0 = npos.end_col.saturating_sub(1);
            if line == l0 && character >= c0 && character < e0 {
                return Some(name);
            }
        }
    }
    // Fall back: any Name expression at the cursor — walk *every* statement
    // of every declaration (not just the first stmt, which misses calls that
    // appear later in a function body).
    for d in &file.decls {
        let mut hit = None;
        expr_name_at_anywhere(d, line, character, &mut hit);
        if let Some(n) = hit {
            return Some(n);
        }
    }
    None
}

/// Position of a declaration's name (1-based). Returns (name, name_pos).
pub(crate) fn decl_name_pos(d: &Decl) -> Option<(String, cj_ast::CodePos)> {
    match d {
        Decl::Func { name, name_pos, .. }
        | Decl::Class { name, name_pos, .. }
        | Decl::Interface { name, name_pos, .. }
        | Decl::Struct { name, name_pos, .. }
        | Decl::Enum { name, name_pos, .. }
        | Decl::Var { name, name_pos, .. } => Some((name.clone(), *name_pos)),
        Decl::Macro { name, pos, .. } => Some((name.clone(), *pos)),
        _ => None,
    }
}

/// Walk a declaration's body, collecting Name references equal to `target`.
pub(crate) fn collect_name_refs(d: &Decl, target: &str, locs: &mut Vec<(u32, u32, u32, u32)>) {
    match d {
        Decl::Var {
            init: Some(init), ..
        } => {
            expr_name_refs(init, target, locs);
        }
        Decl::Func {
            body: Body::Block(stmts),
            ..
        }
        | Decl::Macro {
            body: Body::Block(stmts),
            ..
        }
        | Decl::Main {
            body: Body::Block(stmts),
            ..
        } => {
            for s in stmts {
                expr_name_refs(s, target, locs);
            }
        }
        _ => {}
    }
}

/// Collect Name expressions equal to `target` in an expression tree.
pub(crate) fn expr_name_refs(e: &Expr, target: &str, locs: &mut Vec<(u32, u32, u32, u32)>) {
    match e {
        Expr::Name { name, pos, .. } => {
            if name == target {
                locs.push((
                    pos.line.saturating_sub(1),
                    pos.col.saturating_sub(1),
                    pos.end_col.saturating_sub(1),
                    0,
                ));
            }
        }
        Expr::Call { callee, args, .. } => {
            expr_name_refs(callee, target, locs);
            for a in args {
                expr_name_refs(&a.value, target, locs);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            expr_name_refs(lhs, target, locs);
            expr_name_refs(rhs, target, locs);
        }
        Expr::Unary { inner, .. } => expr_name_refs(inner, target, locs),
        Expr::Paren { inner, .. } => expr_name_refs(inner, target, locs),
        Expr::Block { stmts, .. } => {
            for s in stmts {
                expr_name_refs(s, target, locs);
            }
        }
        Expr::If {
            cond, then, els, ..
        } => {
            expr_name_refs(cond, target, locs);
            expr_name_refs(then, target, locs);
            if let Some(e2) = els {
                expr_name_refs(e2, target, locs);
            }
        }
        // `let x = add(...)` / `var y = foo(...)`: the initializer may
        // reference the target name.
        Expr::LetPatternDestructor { initializer, .. } => {
            expr_name_refs(initializer, target, locs);
        }
        Expr::Return { value: Some(v), .. } => {
            expr_name_refs(v, target, locs);
        }
        Expr::Assign { lhs, rhs, .. } => {
            expr_name_refs(lhs, target, locs);
            expr_name_refs(rhs, target, locs);
        }
        _ => {}
    }
}

/// Walk every statement of a declaration looking for a Name expression at the
/// cursor (unlike decl_first_expr, this covers calls later in a body).
fn expr_name_at_anywhere(d: &Decl, line: u32, character: u32, hit: &mut Option<String>) {
    match d {
        Decl::Var {
            init: Some(init), ..
        }
        | Decl::VarWithPattern {
            init: Some(init), ..
        } => expr_name_at(init, line, character, hit),
        Decl::Func {
            body: Body::Block(stmts),
            ..
        }
        | Decl::Macro {
            body: Body::Block(stmts),
            ..
        }
        | Decl::Main {
            body: Body::Block(stmts),
            ..
        } => {
            for s in stmts {
                expr_name_at(s, line, character, hit);
                if hit.is_some() {
                    return;
                }
            }
        }
        _ => {}
    }
}

/// Find a Name expression at the cursor (sets `hit`).
fn expr_name_at(e: &Expr, line: u32, character: u32, hit: &mut Option<String>) {
    match e {
        Expr::Name { name, pos, .. } => {
            let l0 = pos.line.saturating_sub(1);
            let c0 = pos.col.saturating_sub(1);
            let e0 = pos.end_col.saturating_sub(1);
            if line == l0 && character >= c0 && character < e0 {
                *hit = Some(name.clone());
            }
        }
        Expr::Call { callee, args, .. } => {
            expr_name_at(callee, line, character, hit);
            for a in args {
                expr_name_at(&a.value, line, character, hit);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            expr_name_at(lhs, line, character, hit);
            expr_name_at(rhs, line, character, hit);
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                expr_name_at(s, line, character, hit);
            }
        }
        _ => {}
    }
}
