// cj-lsp: textDocument/documentHighlight — highlight all occurrences of the
// symbol under the cursor in the current file (declaration + usages).
//
// LSP DocumentHighlight:
//   [{ range: { start, end }, kind: 1|2|3 }]
//   kind: 1 = Text, 2 = Read, 3 = Write
//
// Strategy reuses the references walker (name_at + collect_name_refs +
// expr_name_refs) to gather every position of the cursor's symbol, then
// emits them as highlight ranges. The declaration is marked Write (3);
// other occurrences Text (1).
//
// In addition to Expr usages, the walker also covers *type* positions
// (e.g. `Any` in `class A1<:Any{}`): Type::Ref name spans are collected so
// Ctrl+click on a type constraint highlights every occurrence, matching the
// official documentHighlight_001.

use cj_ast::{Body, Decl, File, Type};
use serde_json::{json, Value};

use crate::references::{collect_name_refs, decl_name_pos, name_at};

/// LSP documentHighlight for the symbol at (line, character) — 0-based.
pub fn document_highlight_at(file: &File, line: u32, character: u32) -> Value {
    let Some(target) = type_or_expr_name_at(file, line, character) else {
        return json!([]);
    };
    let locs = collect_occurrences(file, &target);
    // Official cangjie LSP emits kind=1 (Text) for *all* occurrences — the
    // declaration is NOT distinguished as Write(3). Verified across 300
    // expected highlights in the official documentHighlight suite.
    let out: Vec<Value> = locs
        .iter()
        .map(|(l0, c0, e0)| {
            json!({
                "range": {
                    "start": {"line": l0, "character": c0},
                    "end": {"line": l0, "character": e0},
                },
                "kind": 1,
            })
        })
        .collect();
    json!(out)
}

/// Collect every occurrence of `target` in the file as 0-based LSP ranges
/// (line, start_col, end_col), de-duplicated and sorted. Shared by
/// documentHighlight (ranges → highlights) and rename (ranges → TextEdits):
/// both features must agree on which positions belong to a symbol, so the
/// collection lives here and callers only shape the output.
///
/// Covers the declaration name (decl_name_pos), Name-expression usages
/// (collect_name_refs) and *type* positions (collect_type_refs, e.g. `Any`
/// in `class A1<:Any{}` — Ctrl+click on a type constraint highlights every
/// occurrence, matching the official documentHighlight_001).
pub(crate) fn collect_occurrences(file: &File, target: &str) -> Vec<(u32, u32, u32)> {
    let mut locs: Vec<(u32, u32, u32)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for d in &file.decls {
        if let Some((name, npos)) = decl_name_pos(d) {
            if name == target {
                let key = (
                    npos.line.saturating_sub(1),
                    npos.col.saturating_sub(1),
                    npos.end_col.saturating_sub(1),
                );
                if seen.insert(key) {
                    locs.push(key);
                }
            }
        }
    }
    let mut use_locs: Vec<(u32, u32, u32, u32)> = Vec::new();
    for d in &file.decls {
        collect_name_refs(d, target, &mut use_locs);
    }
    for (l0, c0, e0, _) in use_locs {
        if seen.insert((l0, c0, e0)) {
            locs.push((l0, c0, e0));
        }
    }
    let mut type_locs: Vec<(u32, u32, u32)> = Vec::new();
    for d in &file.decls {
        collect_type_refs(d, target, &mut type_locs);
    }
    for (l0, c0, e0) in type_locs {
        if seen.insert((l0, c0, e0)) {
            locs.push((l0, c0, e0));
        }
    }

    locs.sort();
    locs
}

/// Extract the statements of a function/macro/main body (empty if none).
fn body_stmts(body: &Body) -> &[cj_ast::Expr] {
    match body {
        Body::Block(stmts) => stmts,
        _ => &[],
    }
}

pub(crate) fn type_or_expr_name_at(file: &File, line: u32, character: u32) -> Option<String> {
    let mut type_hit = None;
    for d in &file.decls {
        collect_type_at(d, line, character, &mut type_hit);
        if type_hit.is_some() {
            return type_hit;
        }
    }
    name_at(file, line, character)
}

fn collect_type_refs(d: &Decl, target: &str, locs: &mut Vec<(u32, u32, u32)>) {
    let push = |locs: &mut Vec<(u32, u32, u32)>, t: &Type| type_refs(t, target, locs);
    match d {
        Decl::Class {
            parents,
            type_params,
            members,
            ..
        }
        | Decl::Interface {
            parents,
            type_params,
            members,
            ..
        } => {
            for p in parents {
                push(locs, p);
            }
            for tp in type_params {
                for b in &tp.bounds {
                    push(locs, b);
                }
            }
            for p in parents {
                push(locs, p);
            }
            for m in members {
                collect_type_refs(m, target, locs);
            }
        }
        Decl::Struct {
            type_params,
            parents,
            members,
            ..
        } => {
            for tp in type_params {
                for b in &tp.bounds {
                    push(locs, b);
                }
            }
            for p in parents {
                push(locs, p);
            }
            for m in members {
                collect_type_refs(m, target, locs);
            }
        }
        Decl::Enum {
            type_params,
            parents,
            cases,
            ..
        } => {
            for tp in type_params {
                for b in &tp.bounds {
                    push(locs, b);
                }
            }
            for p in parents {
                push(locs, p);
            }
            for c in cases {
                for p in &c.payloads {
                    push(locs, p);
                }
            }
        }
        Decl::Extend {
            target: ext_target,
            members,
            ..
        } => {
            push(locs, ext_target);
            for m in members {
                collect_type_refs(m, target, locs);
            }
        }
        Decl::TypeAlias { target, .. } => push(locs, target),
        Decl::Func {
            params,
            ret,
            type_params,
            body,
            ..
        } => {
            for p in params {
                push(locs, &p.ty);
            }
            if let Some(r) = ret {
                push(locs, r);
            }
            for tp in type_params {
                for b in &tp.bounds {
                    push(locs, b);
                }
            }
            for s in body_stmts(body) {
                expr_type_refs(s, target, locs);
            }
        }
        Decl::Macro { params, body, .. } => {
            for p in params {
                push(locs, &p.ty);
            }
            for s in body_stmts(body) {
                expr_type_refs(s, target, locs);
            }
        }
        Decl::Main { body, .. } => {
            for s in body_stmts(body) {
                expr_type_refs(s, target, locs);
            }
        }
        Decl::Prop { ty, .. } => push(locs, ty),
        Decl::Var { ty, init, .. } => {
            if let Some(t) = ty {
                push(locs, t);
            }
            if let Some(init) = init {
                expr_type_refs(init, target, locs);
            }
        }
        Decl::VarWithPattern { ty, init, .. } => {
            if let Some(t) = ty {
                push(locs, t);
            }
            if let Some(init) = init {
                expr_type_refs(init, target, locs);
            }
        }
        Decl::PrimaryCtor { params, .. } => {
            for p in params {
                push(locs, &p.ty);
            }
        }
        _ => {}
    }
}

fn type_refs(t: &Type, target: &str, locs: &mut Vec<(u32, u32, u32)>) {
    match t {
        Type::Ref { name, args, pos } => {
            if name == target {
                locs.push((
                    pos.line.saturating_sub(1),
                    pos.col.saturating_sub(1),
                    pos.end_col.saturating_sub(1),
                ));
            }
            for a in args {
                type_refs(a, target, locs);
            }
        }
        Type::Qualified { name, pos } => {
            let simple = name.rsplit('.').next().unwrap_or(name);
            if simple == target {
                locs.push((
                    pos.line.saturating_sub(1),
                    pos.col.saturating_sub(1),
                    pos.end_col.saturating_sub(1),
                ));
            }
        }
        Type::Option { inner, .. }
        | Type::Constant { inner, .. }
        | Type::VArray { inner, .. }
        | Type::Paren { inner, .. } => type_refs(inner, target, locs),
        Type::Tuple { elements, .. } => {
            for e in elements {
                type_refs(e, target, locs);
            }
        }
        Type::Func { params, ret, .. } => {
            for p in params {
                type_refs(p, target, locs);
            }
            type_refs(ret, target, locs);
        }
        _ => {}
    }
}

fn type_at(t: &Type, line: u32, character: u32, hit: &mut Option<String>) {
    match t {
        Type::Ref { name, pos, args } => {
            let l0 = pos.line.saturating_sub(1);
            let c0 = pos.col.saturating_sub(1);
            let e0 = pos.end_col.saturating_sub(1);
            if line == l0 && character >= c0 && character <= e0 {
                *hit = Some(name.clone());
                return;
            }
            for a in args {
                type_at(a, line, character, hit);
            }
        }
        Type::Qualified { name, pos } => {
            let l0 = pos.line.saturating_sub(1);
            let c0 = pos.col.saturating_sub(1);
            let e0 = pos.end_col.saturating_sub(1);
            if line == l0 && character >= c0 && character <= e0 {
                let simple = name.rsplit('.').next().unwrap_or(name);
                *hit = Some(simple.to_string());
            }
        }
        Type::Option { inner, .. }
        | Type::Constant { inner, .. }
        | Type::VArray { inner, .. }
        | Type::Paren { inner, .. } => type_at(inner, line, character, hit),
        Type::Tuple { elements, .. } => {
            for e in elements {
                type_at(e, line, character, hit);
            }
        }
        Type::Func { params, ret, .. } => {
            for p in params {
                type_at(p, line, character, hit);
            }
            type_at(ret, line, character, hit);
        }
        _ => {}
    }
}

fn expr_type_refs(e: &cj_ast::Expr, target: &str, locs: &mut Vec<(u32, u32, u32)>) {
    use cj_ast::Expr;
    match e {
        Expr::Is { ty, inner, .. } | Expr::As { ty, inner, .. } => {
            type_refs(ty, target, locs);
            expr_type_refs(inner, target, locs);
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                expr_type_refs(s, target, locs);
            }
        }
        Expr::If {
            cond, then, els, ..
        } => {
            expr_type_refs(cond, target, locs);
            expr_type_refs(then, target, locs);
            if let Some(e2) = els {
                expr_type_refs(e2, target, locs);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            expr_type_refs(lhs, target, locs);
            expr_type_refs(rhs, target, locs);
        }
        Expr::Call { callee, args, .. } => {
            expr_type_refs(callee, target, locs);
            for a in args {
                expr_type_refs(&a.value, target, locs);
            }
        }
        Expr::Paren { inner, .. } | Expr::Unary { inner, .. } => {
            expr_type_refs(inner, target, locs);
        }
        Expr::LetPatternDestructor {
            initializer,
            patterns,
            ..
        } => {
            expr_type_refs(initializer, target, locs);
            for p in patterns {
                if let cj_ast::Pattern::Var {
                    name, name_pos, ty, ..
                } = p
                {
                    if name == target {
                        locs.push((
                            name_pos.line.saturating_sub(1),
                            name_pos.col.saturating_sub(1),
                            name_pos.end_col.saturating_sub(1),
                        ));
                    }
                    if let Some(t) = ty {
                        type_refs(t, target, locs);
                    }
                }
            }
        }
        Expr::Return { value: Some(v), .. } => expr_type_refs(v, target, locs),
        Expr::Assign { lhs, rhs, .. } => {
            expr_type_refs(lhs, target, locs);
            expr_type_refs(rhs, target, locs);
        }
        _ => {}
    }
}

fn collect_type_at(d: &Decl, line: u32, character: u32, hit: &mut Option<String>) {
    let mut probe = |t: &Type| {
        if hit.is_none() {
            type_at(t, line, character, hit);
        }
    };
    match d {
        Decl::Class {
            parents,
            type_params,
            members,
            ..
        }
        | Decl::Interface {
            parents,
            type_params,
            members,
            ..
        } => {
            for p in parents {
                probe(p);
            }
            for tp in type_params {
                for b in &tp.bounds {
                    probe(b);
                }
            }
            for p in parents {
                probe(p);
            }
            for m in members {
                collect_type_at(m, line, character, hit);
            }
        }
        Decl::Struct {
            type_params,
            parents,
            members,
            ..
        } => {
            for tp in type_params {
                for b in &tp.bounds {
                    probe(b);
                }
            }
            for p in parents {
                probe(p);
            }
            for m in members {
                collect_type_at(m, line, character, hit);
            }
        }
        Decl::Enum {
            type_params,
            parents,
            cases,
            ..
        } => {
            for tp in type_params {
                for b in &tp.bounds {
                    probe(b);
                }
            }
            for p in parents {
                probe(p);
            }
            for c in cases {
                for p in &c.payloads {
                    probe(p);
                }
            }
        }
        Decl::Extend {
            target: ext_target,
            members,
            ..
        } => {
            probe(ext_target);
            for m in members {
                collect_type_at(m, line, character, hit);
            }
        }
        Decl::TypeAlias { target, .. } => probe(target),
        Decl::Func {
            params,
            ret,
            type_params,
            body,
            ..
        } => {
            for p in params {
                probe(&p.ty);
            }
            if let Some(r) = ret {
                probe(r);
            }
            for tp in type_params {
                for b in &tp.bounds {
                    probe(b);
                }
            }
            for s in body_stmts(body) {
                if hit.is_none() {
                    expr_type_at(s, line, character, hit);
                }
            }
        }
        Decl::Macro { params, body, .. } => {
            for p in params {
                probe(&p.ty);
            }
            for s in body_stmts(body) {
                if hit.is_none() {
                    expr_type_at(s, line, character, hit);
                }
            }
        }
        Decl::Main { body, .. } => {
            for s in body_stmts(body) {
                if hit.is_none() {
                    expr_type_at(s, line, character, hit);
                }
            }
        }
        Decl::Prop { ty, .. } => probe(ty),
        Decl::Var { ty, init, .. } => {
            if let Some(t) = ty {
                probe(t);
            }
            if let Some(init) = init {
                if hit.is_none() {
                    expr_type_at(init, line, character, hit);
                }
            }
        }
        Decl::PrimaryCtor { params, .. } => {
            for p in params {
                probe(&p.ty);
            }
        }
        _ => {}
    }
}

fn expr_type_at(e: &cj_ast::Expr, line: u32, character: u32, hit: &mut Option<String>) {
    use cj_ast::Expr;
    match e {
        Expr::Is { ty, .. } | Expr::As { ty, .. } => type_at(ty, line, character, hit),
        Expr::Block { stmts, .. } => {
            for s in stmts {
                if hit.is_none() {
                    expr_type_at(s, line, character, hit);
                }
            }
        }
        Expr::Call { callee, args, .. } => {
            expr_type_at(callee, line, character, hit);
            for a in args {
                if hit.is_none() {
                    expr_type_at(&a.value, line, character, hit);
                }
            }
        }
        Expr::LetPatternDestructor {
            initializer,
            patterns,
            ..
        } => {
            expr_type_at(initializer, line, character, hit);
            for p in patterns {
                if let cj_ast::Pattern::Var {
                    name, name_pos, ty, ..
                } = p
                {
                    let l0 = name_pos.line.saturating_sub(1);
                    let c0 = name_pos.col.saturating_sub(1);
                    let e0 = name_pos.end_col.saturating_sub(1);
                    if line == l0 && character >= c0 && character <= e0 {
                        *hit = Some(name.clone());
                    }
                    if hit.is_none() {
                        if let Some(t) = ty {
                            type_at(t, line, character, hit);
                        }
                    }
                }
            }
        }
        Expr::Paren { inner, .. } | Expr::Unary { inner, .. } => {
            expr_type_at(inner, line, character, hit);
        }
        Expr::Binary { lhs, rhs, .. } => {
            expr_type_at(lhs, line, character, hit);
            if hit.is_none() {
                expr_type_at(rhs, line, character, hit);
            }
        }
        _ => {}
    }
}
