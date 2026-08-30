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

use crate::references::{collect_name_refs, decl_name_pos};

/// Identity of a member container (Class / Interface / Struct / Extend).
/// Keyed by the container's own name position (Extend has no name, so its
/// decl position is used) — stable within a file, so a member symbol's scope
/// can be tracked as it flows through the name/ref walkers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ContainerId {
    // 1-based position of the container decl's name token (or its pos).
    line: u32,
    col: u32,
}

/// LSP documentHighlight for the symbol at (line, character) — 0-based.
///
/// Official behavior scopes a member symbol to *its* container: hovering
/// `foo` declared in `class A` must not highlight a same-named `foo` in
/// `class B`. So the target is resolved together with the container it
/// belongs to, and collection filters occurrences to that container.
/// Top-level (file-scope) symbols keep the unfiltered whole-file collection.
pub fn document_highlight_at(file: &File, line: u32, character: u32) -> Value {
    let Some((target, container)) = type_or_expr_name_at_scoped(file, line, character) else {
        return json!([]);
    };
    let locs = collect_occurrences_scoped(file, &target, container.as_ref());
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

/// Name of the declaration/expression at the cursor (unscoped — for
/// references/rename which want the raw symbol; rename.rs imports this).
pub(crate) fn type_or_expr_name_at(file: &File, line: u32, character: u32) -> Option<String> {
    type_or_expr_name_at_scoped(file, line, character).map(|(n, _)| n)
}

/// A (type name → container) index used to resolve `Type.member` receivers
/// to the container that owns `member`.
type TypeContainerIndex = Vec<(String, ContainerId)>;

/// Build the index of top-level Class/Interface/Struct/Enum type names mapping
/// to their container ids (used to resolve `DataRecord.foo` → DataRecord).
fn build_type_index(file: &File) -> TypeContainerIndex {
    let mut idx = Vec::new();
    for d in &file.decls {
        let type_name = match d {
            Decl::Class { name, .. }
            | Decl::Interface { name, .. }
            | Decl::Struct { name, .. }
            | Decl::Enum { name, .. } => name.clone(),
            _ => continue,
        };
        if let Some(cid) = container_id_of(d) {
            idx.push((type_name, cid));
        }
    }
    idx
}

/// Resolve the symbol under the cursor to `(name, container)`. The container
/// is `Some` when the hit is a *member* of a Class/Interface/Struct/Extend —
/// either a member declaration name (`func foo` inside `class A`) or a
/// member-access usage (`A.foo(...)` / bare `foo()` inside A). Top-level
/// symbols (file-scope funcs/vars/types) and type positions yield `None`.
///
/// Priority mirrors the historical resolver: type position → declaration
/// name → name expression. Member descent is *added* (previously these
/// positions resolved to nothing), so it cannot alter existing top-level hits.
fn type_or_expr_name_at_scoped(
    file: &File,
    line: u32,
    character: u32,
) -> Option<(String, Option<ContainerId>)> {
    // 1. Type positions (types are file-scoped — no container).
    let mut type_hit = None;
    for d in &file.decls {
        collect_type_at(d, line, character, &mut type_hit);
        if type_hit.is_some() {
            return type_hit.map(|n| (n, None));
        }
    }
    let type_index = build_type_index(file);
    // 2. Declaration names — top-level and member decls (container-aware).
    let mut decl_hit: Option<(String, Option<ContainerId>)> = None;
    for d in &file.decls {
        decl_name_at_scoped(d, None, line, character, &mut decl_hit);
        if decl_hit.is_some() {
            return decl_hit;
        }
    }
    // 3. Name expressions — incl. member bodies and Member member-name spans.
    let mut expr_hit: Option<(String, Option<ContainerId>)> = None;
    for d in &file.decls {
        expr_name_at_scoped(d, None, line, character, &type_index, &mut expr_hit);
        if expr_hit.is_some() {
            return expr_hit;
        }
    }
    None
}

/// The container id of a Class/Interface/Struct/Extend decl (keyed by its own
/// name position; Extend uses its decl position since it has no name).
fn container_id_of(d: &Decl) -> Option<ContainerId> {
    match d {
        Decl::Class { name_pos, .. }
        | Decl::Interface { name_pos, .. }
        | Decl::Struct { name_pos, .. }
        | Decl::Enum { name_pos, .. } => Some(ContainerId {
            line: name_pos.line,
            col: name_pos.col,
        }),
        Decl::Extend { pos, .. } => Some(ContainerId {
            line: pos.line,
            col: pos.col,
        }),
        _ => None,
    }
}

/// Descend a decl checking its own declaration name (parent container = the
/// container that owns it) and then its members (member decls get the decl
/// itself as container).
fn decl_name_at_scoped(
    d: &Decl,
    parent_container: Option<ContainerId>,
    line: u32,
    character: u32,
    hit: &mut Option<(String, Option<ContainerId>)>,
) {
    if hit.is_some() {
        return;
    }
    if let Some((name, npos)) = decl_name_pos(d) {
        let l0 = npos.line.saturating_sub(1);
        let c0 = npos.col.saturating_sub(1);
        let e0 = npos.end_col.saturating_sub(1);
        if line == l0 && character >= c0 && character <= e0 {
            *hit = Some((name, parent_container));
            return;
        }
    }
    match d {
        Decl::Class { members, .. }
        | Decl::Interface { members, .. }
        | Decl::Struct { members, .. }
        | Decl::Extend { members, .. } => {
            let self_container = container_id_of(d).or(parent_container);
            for m in members {
                decl_name_at_scoped(m, self_container, line, character, hit);
                if hit.is_some() {
                    return;
                }
            }
        }
        _ => {}
    }
}

/// Walk a declaration's bodies (and, for containers, its member bodies)
/// looking for a name expression at the cursor, tracking the enclosing
/// container.
fn expr_name_at_scoped(
    d: &Decl,
    enclosing: Option<ContainerId>,
    line: u32,
    character: u32,
    type_index: &TypeContainerIndex,
    hit: &mut Option<(String, Option<ContainerId>)>,
) {
    if hit.is_some() {
        return;
    }
    match d {
        Decl::Var {
            init: Some(init), ..
        } => expr_name_at_scoped_expr(init, enclosing, line, character, type_index, hit),
        Decl::VarWithPattern {
            init: Some(init), ..
        } => expr_name_at_scoped_expr(init, enclosing, line, character, type_index, hit),
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
                expr_name_at_scoped_expr(s, enclosing, line, character, type_index, hit);
                if hit.is_some() {
                    return;
                }
            }
        }
        Decl::Class { members, .. }
        | Decl::Interface { members, .. }
        | Decl::Struct { members, .. }
        | Decl::Extend { members, .. } => {
            let self_container = container_id_of(d).or(enclosing);
            for m in members {
                expr_name_at_scoped(m, self_container, line, character, type_index, hit);
                if hit.is_some() {
                    return;
                }
            }
        }
        _ => {}
    }
}

/// Find a Name expression (or Member member-name) at the cursor, recording the
/// enclosing container. `Expr::Member.pos` is the *dot*; the member name span
/// is derived: 0-based `(pos.line-1, pos.col, pos.col + name.len())`.
fn expr_name_at_scoped_expr(
    e: &cj_ast::Expr,
    enclosing: Option<ContainerId>,
    line: u32,
    character: u32,
    type_index: &TypeContainerIndex,
    hit: &mut Option<(String, Option<ContainerId>)>,
) {
    use cj_ast::Expr;
    if hit.is_some() {
        return;
    }
    match e {
        Expr::Name { name, pos, .. } => {
            let l0 = pos.line.saturating_sub(1);
            let c0 = pos.col.saturating_sub(1);
            let e0 = pos.end_col.saturating_sub(1);
            if line == l0 && character >= c0 && character <= e0 {
                *hit = Some((name.clone(), enclosing));
            }
        }
        Expr::Member { object, name, pos } => {
            // member-name span: the dot is at (pos.line, pos.col) 1-based, so
            // the member token occupies 0-based [pos.col, pos.col + len).
            let l0 = pos.line.saturating_sub(1);
            let c0 = pos.col; // 1-based dot col == 0-based member start col
            let e0 = pos.col + name.chars().count() as u32;
            if line == l0 && character >= c0 && character <= e0 {
                // Resolve the container from the receiver:
                //   `DataRecord.foo` (receiver is a type name) → that type;
                //   `this.foo`/`self.foo` or a value receiver inside a member
                //   body → the enclosing container; unknown → enclosing (may
                //   be None → whole-file, the pre-existing fallback).
                let container = match object.as_ref() {
                    Expr::Name { name: on, .. } => type_index
                        .iter()
                        .find(|(tn, _)| tn == on)
                        .map(|(_, cid)| *cid)
                        .or(enclosing),
                    _ => enclosing,
                };
                *hit = Some((name.clone(), container));
                return;
            }
            expr_name_at_scoped_expr(object, enclosing, line, character, type_index, hit);
        }
        Expr::Call { callee, args, .. } => {
            expr_name_at_scoped_expr(callee, enclosing, line, character, type_index, hit);
            for a in args {
                expr_name_at_scoped_expr(&a.value, enclosing, line, character, type_index, hit);
            }
        }
        Expr::Subscript { object, index, .. } => {
            expr_name_at_scoped_expr(object, enclosing, line, character, type_index, hit);
            expr_name_at_scoped_expr(index, enclosing, line, character, type_index, hit);
        }
        Expr::Binary { lhs, rhs, .. } => {
            expr_name_at_scoped_expr(lhs, enclosing, line, character, type_index, hit);
            expr_name_at_scoped_expr(rhs, enclosing, line, character, type_index, hit);
        }
        Expr::Assign { lhs, rhs, .. } => {
            expr_name_at_scoped_expr(lhs, enclosing, line, character, type_index, hit);
            expr_name_at_scoped_expr(rhs, enclosing, line, character, type_index, hit);
        }
        Expr::Return { value: Some(v), .. } => {
            expr_name_at_scoped_expr(v, enclosing, line, character, type_index, hit);
        }
        Expr::Paren { inner, .. } | Expr::Unary { inner, .. } => {
            expr_name_at_scoped_expr(inner, enclosing, line, character, type_index, hit);
        }
        Expr::As { inner, .. } | Expr::Is { inner, .. } => {
            expr_name_at_scoped_expr(inner, enclosing, line, character, type_index, hit);
        }
        Expr::If {
            cond, then, els, ..
        } => {
            expr_name_at_scoped_expr(cond, enclosing, line, character, type_index, hit);
            expr_name_at_scoped_expr(then, enclosing, line, character, type_index, hit);
            if let Some(e2) = els {
                expr_name_at_scoped_expr(e2, enclosing, line, character, type_index, hit);
            }
        }
        Expr::LetPatternDestructor {
            initializer,
            patterns,
            ..
        } => {
            expr_name_at_scoped_expr(initializer, enclosing, line, character, type_index, hit);
            for p in patterns {
                if let cj_ast::Pattern::Var { name, name_pos, .. } = p {
                    let l0 = name_pos.line.saturating_sub(1);
                    let c0 = name_pos.col.saturating_sub(1);
                    let e0 = name_pos.end_col.saturating_sub(1);
                    if line == l0 && character >= c0 && character <= e0 {
                        *hit = Some((name.clone(), enclosing));
                    }
                }
            }
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                expr_name_at_scoped_expr(s, enclosing, line, character, type_index, hit);
            }
        }
        _ => {}
    }
}

/// Collect every occurrence of `target` in the file as 0-based LSP ranges,
/// scoped to `container` when Some (member symbol): only occurrences that
/// belong to that container. When None (top-level/type symbol) the unfiltered
/// whole-file collection runs — identical to the pre-scoping behavior, so
/// references/rename and existing top-level highlights are unchanged.
fn collect_occurrences_scoped(
    file: &File,
    target: &str,
    container: Option<&ContainerId>,
) -> Vec<(u32, u32, u32)> {
    match container {
        None => collect_occurrences(file, target),
        Some(c) => collect_member_occurrences(file, target, *c),
    }
}

/// Container-scoped collection for a member symbol: member declarations of
/// `target` inside the container + name usages that resolve to it (bare names
/// inside its member bodies, and `Type.target` member-access spans anywhere).
fn collect_member_occurrences(
    file: &File,
    target: &str,
    container: ContainerId,
) -> Vec<(u32, u32, u32)> {
    let mut locs: Vec<(u32, u32, u32)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // Type-name → container index for resolving `X.target` receivers.
    let type_index = build_type_index(file);
    // 1. Member declarations named `target` whose owning container matches.
    for d in &file.decls {
        collect_member_decls(d, None, container, target, &mut locs, &mut seen);
    }
    // 2. Name usages resolving to the container.
    for d in &file.decls {
        collect_member_usages(
            d,
            None,
            container,
            target,
            &type_index,
            &mut locs,
            &mut seen,
        );
    }
    locs.sort();
    locs
}

/// Descend decls, adding member-declaration name positions of `target` when
/// the enclosing container equals `container`.
fn collect_member_decls(
    d: &Decl,
    parent_container: Option<ContainerId>,
    container: ContainerId,
    target: &str,
    locs: &mut Vec<(u32, u32, u32)>,
    seen: &mut std::collections::HashSet<(u32, u32, u32)>,
) {
    if parent_container == Some(container) {
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
    match d {
        Decl::Class { members, .. }
        | Decl::Interface { members, .. }
        | Decl::Struct { members, .. }
        | Decl::Extend { members, .. } => {
            let self_container = container_id_of(d).or(parent_container);
            for m in members {
                collect_member_decls(m, self_container, container, target, locs, seen);
            }
        }
        _ => {}
    }
}

/// Walk every declaration body (incl. container members), adding usages of
/// `target` that resolve to `container`:
///   - a bare `Name` expression inside one of the container's member bodies;
///   - a member-access `X.target` span whose receiver `X` is a type name that
///     maps to `container` (also `this`/`self` inside the container).
fn collect_member_usages(
    d: &Decl,
    enclosing: Option<ContainerId>,
    container: ContainerId,
    target: &str,
    type_index: &TypeContainerIndex,
    locs: &mut Vec<(u32, u32, u32)>,
    seen: &mut std::collections::HashSet<(u32, u32, u32)>,
) {
    match d {
        Decl::Var {
            init: Some(init), ..
        } => expr_member_usages(init, enclosing, container, target, type_index, locs, seen),
        Decl::VarWithPattern {
            init: Some(init), ..
        } => expr_member_usages(init, enclosing, container, target, type_index, locs, seen),
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
                expr_member_usages(s, enclosing, container, target, type_index, locs, seen);
            }
        }
        Decl::Class { members, .. }
        | Decl::Interface { members, .. }
        | Decl::Struct { members, .. }
        | Decl::Extend { members, .. } => {
            let self_container = container_id_of(d).or(enclosing);
            for m in members {
                collect_member_usages(m, self_container, container, target, type_index, locs, seen);
            }
        }
        _ => {}
    }
}

/// Add usage positions inside an expression tree that resolve to `container`.
fn expr_member_usages(
    e: &cj_ast::Expr,
    enclosing: Option<ContainerId>,
    container: ContainerId,
    target: &str,
    type_index: &TypeContainerIndex,
    locs: &mut Vec<(u32, u32, u32)>,
    seen: &mut std::collections::HashSet<(u32, u32, u32)>,
) {
    use cj_ast::Expr;
    match e {
        Expr::Name { name, pos, .. } => {
            // Bare name inside this container's own member body → its member.
            if name == target && enclosing == Some(container) {
                let key = (
                    pos.line.saturating_sub(1),
                    pos.col.saturating_sub(1),
                    pos.end_col.saturating_sub(1),
                );
                if seen.insert(key) {
                    locs.push(key);
                }
            }
        }
        Expr::Member { object, name, pos } => {
            if name == target {
                let resolves = match object.as_ref() {
                    Expr::Name { name: on, .. } => {
                        if matches!(on.as_str(), "this" | "self") && enclosing == Some(container) {
                            true
                        } else {
                            type_index
                                .iter()
                                .any(|(tn, cid)| tn == on && *cid == container)
                        }
                    }
                    _ => false,
                };
                if resolves {
                    let l0 = pos.line.saturating_sub(1);
                    let c0 = pos.col;
                    let e0 = pos.col + name.chars().count() as u32;
                    let key = (l0, c0, e0);
                    if seen.insert(key) {
                        locs.push(key);
                    }
                }
            }
            expr_member_usages(object, enclosing, container, target, type_index, locs, seen);
        }
        Expr::Call { callee, args, .. } => {
            expr_member_usages(callee, enclosing, container, target, type_index, locs, seen);
            for a in args {
                expr_member_usages(
                    &a.value, enclosing, container, target, type_index, locs, seen,
                );
            }
        }
        Expr::Subscript { object, index, .. } => {
            expr_member_usages(object, enclosing, container, target, type_index, locs, seen);
            expr_member_usages(index, enclosing, container, target, type_index, locs, seen);
        }
        Expr::Binary { lhs, rhs, .. } => {
            expr_member_usages(lhs, enclosing, container, target, type_index, locs, seen);
            expr_member_usages(rhs, enclosing, container, target, type_index, locs, seen);
        }
        Expr::Assign { lhs, rhs, .. } => {
            expr_member_usages(lhs, enclosing, container, target, type_index, locs, seen);
            expr_member_usages(rhs, enclosing, container, target, type_index, locs, seen);
        }
        Expr::Return { value: Some(v), .. } => {
            expr_member_usages(v, enclosing, container, target, type_index, locs, seen);
        }
        Expr::Paren { inner, .. } | Expr::Unary { inner, .. } => {
            expr_member_usages(inner, enclosing, container, target, type_index, locs, seen);
        }
        Expr::As { inner, .. } | Expr::Is { inner, .. } => {
            expr_member_usages(inner, enclosing, container, target, type_index, locs, seen);
        }
        Expr::If {
            cond, then, els, ..
        } => {
            expr_member_usages(cond, enclosing, container, target, type_index, locs, seen);
            expr_member_usages(then, enclosing, container, target, type_index, locs, seen);
            if let Some(e2) = els {
                expr_member_usages(e2, enclosing, container, target, type_index, locs, seen);
            }
        }
        Expr::LetPatternDestructor {
            initializer,
            patterns,
            ..
        } => {
            expr_member_usages(
                initializer,
                enclosing,
                container,
                target,
                type_index,
                locs,
                seen,
            );
            for p in patterns {
                if let cj_ast::Pattern::Var { name, name_pos, .. } = p {
                    if name == target && enclosing == Some(container) {
                        let key = (
                            name_pos.line.saturating_sub(1),
                            name_pos.col.saturating_sub(1),
                            name_pos.end_col.saturating_sub(1),
                        );
                        if seen.insert(key) {
                            locs.push(key);
                        }
                    }
                }
            }
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                expr_member_usages(s, enclosing, container, target, type_index, locs, seen);
            }
        }
        _ => {}
    }
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
