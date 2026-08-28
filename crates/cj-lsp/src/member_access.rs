// cj-lsp: member-access completion — resolve receiver expression, emit members.
//
// Handles `obj.`, `TypeName.`, `this.`, `a.b.c.`, and `enum.` completions,
// distinguishing static vs instance member access according to the official suite.

use cj_ast::{Body, Decl, Expr, File, Pattern, Type};
use std::collections::{HashMap, HashSet};

use super::completion::{
    emit_enum_members, emit_func_items, display_type, push_candidate, vis_prefix,
    Candidate, KIND_METHOD, KIND_VARIABLE, KIND_PROP,
};

/// The kind of member access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessKind {
    /// Receiver is a type name → static members (incl. enum cases)
    Static,
    /// Receiver is an expression → instance members (non-static)
    Instance,
}

/// Resolve a member-access receiver expression to a type name (+ access kind).
///
/// Returns `None` when the receiver can't be resolved (should return null).
pub fn resolve_receiver(
    file: &File,
    source: &str,
    cursor_line: u32,
    raw_receiver: &str,
) -> Option<(String, AccessKind)> {
    let expr = normalize_receiver(raw_receiver)?;
    if expr.is_empty() {
        return None;
    }

    // Build a type-name → decl map
    let type_map = build_type_map(file);

    // Build local var scope (function containing cursor_line)
    let locals = build_local_scope(file, source, cursor_line, &type_map);

    // Resolve the expression chain
    resolve_chain(&expr, &type_map, &locals, file)
}

/// Normalize the receiver text extracted from the source line.
/// Strips `let/var NAME = `, `case `, trailing `?`, pipe segments.
fn normalize_receiver(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }

    // Strip `let x = ` / `var x = ` / `let x: Type = ` / `var x: Type = `
    let s = if let Some(eq) = s.rfind("= ") {
        // But be careful: `a.b = c` has `=` too — we only strip if the `=`
        // is preceded by a decl keyword and a simple name (no `.` before `=`)
        let before_eq = s[..eq].trim();
        if before_eq.starts_with("let ") || before_eq.starts_with("var ") {
            // Strip: "var b = data.b" → "data.b"
            s[eq + 1..].trim().to_string()
        } else {
            s.to_string()
        }
    } else {
        s.to_string()
    };

    let mut s = s;

    // Strip `case ` prefix (e.g. `case TimeUnit.Year` → `TimeUnit.Year`)
    if s.starts_with("case ") {
        s = s[5..].trim().to_string();
    }

    // Strip pipe operators: take the segment after the LAST `|>` or `~>`
    if let Some(pipe) = s.rfind("~>").or_else(|| s.rfind("|>")) {
        s = s[pipe + 2..].trim().to_string();
    }

    // Strip trailing `?` (optional chaining: `c2?` → `c2`)
    if s.ends_with('?') && s.len() > 1 {
        s = s[..s.len() - 1].trim().to_string();
    }

    // Strip `import ` prefix (import path completion — not handled here)
    if s.starts_with("import ") {
        return None;
    }

    // Strip trailing `()` if it's a bare call (not generics)
    if s.ends_with("()") && !s.ends_with(">()") {
        s = s[..s.len() - 2].trim().to_string();
    }

    // Strip `print(this` → `this` (broken receiver extraction for `this.v`)
    if s.ends_with("this") || s == "this" {
        // Find the last `this` in the expression
        if let Some(ti) = s.rfind("this") {
            s = s[ti..].to_string();
        }
    }

    Some(s)
}

/// Build a map: type name → reference to the Decl
fn build_type_map<'a>(file: &'a File) -> HashMap<&'a str, &'a Decl> {
    let mut m = HashMap::new();
    for d in &file.decls {
        match d {
            Decl::Class { name, .. } | Decl::Struct { name, .. }
            | Decl::Interface { name, .. } | Decl::Enum { name, .. } => {
                m.insert(name.as_str(), d);
            }
            _ => {}
        }
    }
    m
}

/// Build a local scope: name → type string, for the function containing the cursor.
fn build_local_scope<'a>(
    file: &'a File,
    source: &str,
    cursor_line: u32,
    type_map: &HashMap<&str, &Decl>,
) -> HashMap<String, String> {
    let mut locals = HashMap::new();

    for d in &file.decls {
        match d {
            Decl::Func { params, body, pos, .. } => {
                if pos.line > cursor_line {
                    continue;
                }
                // Check if this function's body contains the cursor line
                if !body_contains_line(body, cursor_line) {
                    continue;
                }
                // Collect params
                for p in params {
                    locals.insert(p.name.clone(), display_type(&p.ty));
                }
                // Collect let/var statements in the body
                collect_lets_in_body(body, source, cursor_line, &mut locals, type_map, file);
                break; // only one function contains the cursor
            }
            Decl::Main { body, pos } => {
                if pos.line > cursor_line { continue; }
                if !body_contains_line(body, cursor_line) { continue; }
                collect_lets_in_body(body, source, cursor_line, &mut locals, type_map, file);
                break;
            }
            _ => {}
        }
    }

    // Also collect top-level vars (file-level)
    for d in &file.decls {
        if let Decl::Var { name, ty, .. } = d {
            if let Some(t) = ty {
                locals.insert(name.clone(), display_type(t));
            }
        }
    }

    locals
}

fn body_contains_line(body: &Body, line: u32) -> bool {
    match body {
        Body::Block(exprs) => {
            for e in exprs {
                if expr_contains_line(e, line) {
                    return true;
                }
            }
            false
        }
        Body::Empty => false,
    }
}

fn expr_contains_line(e: &Expr, line: u32) -> bool {
    match e {
        Expr::LetPatternDestructor { pos, .. }
        | Expr::Call { pos, .. }
        | Expr::Name { pos, .. }
        | Expr::Member { pos, .. }
        | Expr::Lit { pos, .. }
        | Expr::Paren { pos, .. }
        | Expr::Optional { pos, .. }
        | Expr::OptionalChain { pos, .. }
        | Expr::Block { pos, .. }
        | Expr::If { pos, .. }
        | Expr::While { pos, .. }
        | Expr::DoWhile { pos, .. }
        | Expr::ForIn { pos, .. }
        | Expr::Match { pos, .. }
        | Expr::Try { pos, .. }
        | Expr::Return { pos, .. }
        | Expr::Assign { pos, .. }
        | Expr::Binary { pos, .. }
        | Expr::Unary { pos, .. }
        | Expr::PrimitiveType { pos, .. }
        | Expr::Interpolation { pos, .. }
        | Expr::Quote { pos, .. }
        | Expr::TokenPart { pos, .. }
        | Expr::Wildcard(pos) => pos.line <= line,
        Expr::Perfect { .. } => false,
    }
}

/// Collect `let/var x = <expr>` in a function body, recording the inferred type.
fn collect_lets_in_body(
    body: &Body,
    source: &str,
    cursor_line: u32,
    locals: &mut HashMap<String, String>,
    type_map: &HashMap<&str, &Decl>,
    file: &File,
) {
    match body {
        Body::Block(exprs) => {
            for e in exprs {
                collect_let_in_expr(e, source, cursor_line, locals, type_map, file);
            }
        }
        Body::Empty => {}
    }
}

fn collect_let_in_expr(
    e: &Expr,
    source: &str,
    cursor_line: u32,
    locals: &mut HashMap<String, String>,
    type_map: &HashMap<&str, &Decl>,
    file: &File,
) {
    match e {
        Expr::LetPatternDestructor { patterns, initializer, pos } => {
            if pos.line > cursor_line {
                return;
            }
            let ty = infer_init_type(initializer, type_map, locals, file);
            for p in patterns {
                if let Pattern::Var { name, .. } = p {
                    if let Some(ref t) = ty {
                        locals.insert(name.clone(), t.clone());
                    }
                }
            }
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                collect_let_in_expr(s, source, cursor_line, locals, type_map, file);
            }
        }
        Expr::If { then, els, .. } => {
            collect_let_in_expr(then, source, cursor_line, locals, type_map, file);
            if let Some(e) = els {
                collect_let_in_expr(e, source, cursor_line, locals, type_map, file);
            }
        }
        Expr::ForIn { body, .. } | Expr::While { body, .. } | Expr::DoWhile { body, .. } => {
            collect_let_in_expr(body, source, cursor_line, locals, type_map, file);
        }
        Expr::Match { cases, .. } => {
            for mc in cases {
                collect_let_in_expr(&mc.body, source, cursor_line, locals, type_map, file);
            }
        }
        Expr::Try { body, catches, finally, .. } => {
            collect_let_in_expr(body, source, cursor_line, locals, type_map, file);
            for c in catches {
                collect_let_in_expr(&c.body, source, cursor_line, locals, type_map, file);
            }
            if let Some(f) = finally {
                collect_let_in_expr(f, source, cursor_line, locals, type_map, file);
            }
        }
        _ => {}
    }
}

/// Infer the type of an initializer expression.
fn infer_init_type(
    init: &Expr,
    type_map: &HashMap<&str, &Decl>,
    locals: &HashMap<String, String>,
    file: &File,
) -> Option<String> {
    match init {
        // Constructor call: `TypeName(...)` or `TypeName<...>(...)`
        Expr::Call { callee, .. } => {
            if let Expr::Name { name, type_args, .. } = callee.as_ref() {
                let base = type_name_with_args(name, type_args);
                if type_map.contains_key(name.as_str()) {
                    return Some(base);
                }
                // Also check if this is a member access call: `this.v(...)` 
                // For `this.v(...)` the type is the return type of v
                // But that's complex; for now, just return None
                return None;
            }
            // Member call: `expr.member(...)` — not a constructor
            None
        }
        // Constructor call with `new`? Not in Cangjie.
        // Simple name: `x` where x is a known type/var
        Expr::Name { name, type_args, .. } => {
            if type_map.contains_key(name.as_str()) {
                return Some(type_name_with_args(name, type_args));
            }
            locals.get(name).cloned()
        }
        // Member access: `a.b` — resolve through member types
        Expr::Member { object, name, .. } => {
            let obj_type = infer_init_type(object, type_map, locals, file)?;
            member_type(&obj_type, name, file)
        }
        // `this` → enclosing class type (too complex for now)
        Expr::Name { name, .. } if name == "this" => {
            // Find enclosing class
            None
        }
        // Optional chain: `a?.b` → resolve `a` then member `b`
        Expr::Optional { inner, .. } | Expr::OptionalChain { inner, .. } => {
            infer_init_type(inner, type_map, locals, file)
        }
        // Parenthesized: `(expr)` → resolve expr
        Expr::Paren { inner, .. } => infer_init_type(inner, type_map, locals, file),
        // Literals: determine type
        Expr::Lit { kind, .. } => {
            match kind {
                cj_ast::LitKind::Integer => Some("Int64".to_string()),
                cj_ast::LitKind::Float => Some("Float64".to_string()),
                cj_ast::LitKind::String => Some("String".to_string()),
                cj_ast::LitKind::Char => Some("Rune".to_string()),
                cj_ast::LitKind::Bool => Some("Bool".to_string()),
                _ => None,
            }
        }
        Expr::PrimitiveType { kind, .. } => Some(format!("{kind:?}")),
        _ => None,
    }
}

/// Get the type name with optional type args as a string.
fn type_name_with_args(name: &str, type_args: &[Type]) -> String {
    if type_args.is_empty() {
        name.to_string()
    } else {
        let args: Vec<String> = type_args.iter().map(display_type).collect();
        format!("{}<{}>", name, args.join(", "))
    }
}

/// Look up a member's type in a class/struct/interface/enum.
fn member_type(type_name: &str, member_name: &str, file: &File) -> Option<String> {
    // Strip generic args from type name: `Data5<Int64>` → `Data5`
    let base = type_name.split('<').next().unwrap_or(type_name).trim();
    for d in &file.decls {
        let members = match d {
            Decl::Class { name, members, .. } if *name == base => members,
            Decl::Struct { name, members, .. } if *name == base => members,
            Decl::Interface { name, members, .. } if *name == base => members,
            _ => continue,
        };
        for m in members {
            match m {
                Decl::Var { name, ty, init, .. } if *name == member_name => {
                    if let Some(t) = ty {
                        return Some(display_type(t));
                    }
                    // Infer from init: `let x = TypeName(...)` → TypeName
                    if let Some(init_e) = init {
                        if let Expr::Call { callee, .. } = init_e {
                            if let Expr::Name { name: cn, .. } = callee.as_ref() {
                                return Some(cn.clone());
                            }
                        }
                    }
                    return None;
                }
                Decl::Prop { name, ty, .. } if *name == member_name => {
                    return Some(display_type(ty));
                }
                // PrimaryCtor params with `let`/`var` become members
                Decl::PrimaryCtor { params, .. } => {
                    for p in params {
                        if p.name == member_name {
                            return Some(display_type(&p.ty));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Resolve a chain expression like `data.b` or `Data7.a` or `t1` to a type.
/// Returns (type_name, access_kind).
fn resolve_chain(
    expr: &str,
    type_map: &HashMap<&str, &Decl>,
    locals: &HashMap<String, String>,
    file: &File,
) -> Option<(String, AccessKind)> {
    let expr = expr.trim();
    if expr.is_empty() {
        return None;
    }

    // Split into base + optional member chain
    let parts = split_member_chain(expr);
    if parts.is_empty() {
        return None;
    }

    // Resolve the base
    let base = parts[0].trim();
    let (mut current_type, mut kind) = resolve_base(base, type_map, locals, file)?;

    // Walk the member chain
    for i in 1..parts.len() {
        let member = parts[i].trim();
        // Get the member's type
        let member_ty = member_type(&current_type, member, file)?;
        current_type = member_ty;
        kind = AccessKind::Instance; // After a member access, we're always in instance context
    }

    Some((current_type, kind))
}

/// Split a dotted expression into parts respecting generics and calls.
/// e.g. `Data5<Int64>` → ["Data5<Int64>"]
/// `data.b` → ["data", "b"]
/// `Data7.a` → ["Data7", "a"]
/// `Data5<Int64>.bar<Float64>(2.0)` → ["Data5<Int64>", "bar<Float64>(2.0)"]
fn split_member_chain(expr: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0u32; // angle bracket depth
    let mut paren_depth = 0u32;
    let mut start = 0;
    let bytes = expr.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => depth = depth.saturating_sub(1),
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'.' if depth == 0 && paren_depth == 0 => {
                parts.push(expr[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < bytes.len() {
        let last = expr[start..].trim().to_string();
        if !last.is_empty() {
            parts.push(last);
        }
    }
    parts
}

/// Resolve a single base expression to a type name + access kind.
fn resolve_base(
    base: &str,
    type_map: &HashMap<&str, &Decl>,
    locals: &HashMap<String, String>,
    file: &File,
) -> Option<(String, AccessKind)> {
    // Handle `this`
    if base == "this" {
        let enclosing = find_enclosing_class(file, 0)?; // FIXME: need cursor line
        return Some((enclosing, AccessKind::Instance));
    }

    // Strip trailing `()` if it's a constructor call: `C6()` → `C6`
    let is_ctor = if let Some(paren) = base.rfind('(') {
        // Check if the part before `(` is a type name
        let name_part = base[..paren].trim();
        // Strip generic args: `Data5<Int64>` → `Data5`
        let name_base = name_part.split('<').next().unwrap_or(name_part).trim();
        if type_map.contains_key(name_base) {
            // It's a constructor call: `TypeName(...)`
            true
        } else {
            false
        }
    } else {
        false
    };

    if is_ctor {
        let paren = base.rfind('(').unwrap();
        let name_part = base[..paren].trim();
        let name_base = name_part.split('<').next().unwrap_or(name_part).trim();
        // Constructor call → instance of the type
        // Reconstruct the full type name with generics
        if let Some(decl) = type_map.get(name_base) {
            let type_params = match decl {
                Decl::Class { type_params, .. } => type_params,
                Decl::Struct { type_params, .. } => type_params,
                _ => &[],
            };
            // Use the exact type from the call (with generics preserved)
            Some((name_part.to_string(), AccessKind::Instance))
        } else {
            None
        }
    }

    // Type name (static context)
    // Strip generic args to get base name
    let name_base = base.split('<').next().unwrap_or(base).trim();
    if type_map.contains_key(name_base) {
        // It's a type name → static access
        // Return the full type name with generics
        return Some((base.to_string(), AccessKind::Static));
    }

    // Known local variable
    if let Some(ty) = locals.get(base) {
        return Some((ty.clone(), AccessKind::Instance));
    }

    // Known top-level variable
    for d in &file.decls {
        if let Decl::Var { name, ty, .. } = d {
            if name == base {
                if let Some(t) = ty {
                    return Some((display_type(t), AccessKind::Instance));
                }
                // Infer from init
                if let Some(init) = &d {
                    // ... 
                }
            }
        }
    }

    None
}

/// Find the enclosing class name given a cursor line.
fn find_enclosing_class(file: &File, cursor_line: u32) -> Option<String> {
    for d in &file.decls {
        match d {
            Decl::Class { name, members, pos, .. } => {
                if pos.line > cursor_line {
                    continue;
                }
                // Check if cursor is inside this class body
                let end_line = members.iter()
                    .filter_map(|m| {
                        let p = match m {
                            Decl::Func { pos, .. } => Some(pos),
                            Decl::Var { pos, .. } => Some(pos),
                            Decl::Prop { pos, .. } => Some(pos),
                            Decl::PrimaryCtor { pos, .. } => Some(pos),
                            _ => None,
                        };
                        p
                    })
                    .map(|p| p.line)
                    .max().unwrap_or(pos.line);
                if cursor_line >= pos.line && cursor_line <= end_line {
                    return Some(name.clone());
                }
            }
            Decl::Struct { name, members, pos, .. } => {
                if pos.line > cursor_line { continue; }
                let end_line = members.iter()
                    .filter_map(|m| {
                        let p = match m {
                            Decl::Func { pos, .. } => Some(pos),
                            Decl::Var { pos, .. } => Some(pos),
                            Decl::Prop { pos, .. } => Some(pos),
                            Decl::PrimaryCtor { pos, .. } => Some(pos),
                            _ => None,
                        };
                        p
                    })
                    .map(|p| p.line)
                    .max().unwrap_or(pos.line);
                if cursor_line >= pos.line && cursor_line <= end_line {
                    return Some(name.clone());
                }
            }
            _ => {}
        }
    }
    None
}

/// Collect members of a type for the given access kind.
/// Handles enums, classes, structs, interfaces.
pub fn collect_members(
    type_name: &str,
    kind: AccessKind,
    file: &File,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    let base = type_name.split('<').next().unwrap_or(type_name).trim();
    for d in &file.decls {
        match d {
            Decl::Enum { name, cases, .. } if *name == base => {
                // Enum type → emit enum cases (regardless of static/instance)
                emit_enum_members(name, cases, cands, seen);
                return;
            }
            Decl::Class { name, members, .. } if *name == base => {
                collect_members_filtered(members, kind, cands, seen);
                return;
            }
            Decl::Struct { name, members, .. } if *name == base => {
                collect_members_filtered(members, kind, cands, seen);
                return;
            }
            Decl::Interface { name, members, .. } if *name == base => {
                collect_members_filtered(members, kind, cands, seen);
                return;
            }
            _ => {}
        }
    }
}

/// Collect members, filtering by static vs instance.
fn collect_members_filtered(
    members: &[Decl],
    kind: AccessKind,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    for m in members {
        match m {
            Decl::Var {
                name,
                is_public,
                is_mutable,
                is_static,
                ty,
                init,
                ..
            } => {
                if !matches_kind(*is_static, kind) {
                    continue;
                }
                let vis = vis_prefix(*is_public, false, false, false, false, true);
                let ty_s = ty.as_ref().map(display_type).unwrap_or_default();
                let init_s = init.as_ref().map(|_| " = ...".to_string()).unwrap_or_default();
                let kw = if *is_mutable { "var" } else { "let" };
                let detail = if ty_s.is_empty() {
                    format!("{vis}{kw} {name}{init_s}")
                } else {
                    format!("{vis}{kw} {name}: {ty_s}{init_s}")
                };
                push_candidate(cands, seen, Candidate {
                    label: name.clone(),
                    kind: KIND_VARIABLE,
                    detail,
                    insert_text: name.clone(),
                    insert_text_format: 1,
                    filter_text: name.clone(),
                });
            }
            Decl::Func {
                name,
                is_public,
                is_static,
                is_abstract,
                params,
                ret,
                type_params,
                ..
            } => {
                if !matches_kind(*is_static, kind) {
                    continue;
                }
                emit_func_items(
                    name,
                    *is_public,
                    *is_static,
                    *is_abstract,
                    true,
                    params,
                    ret,
                    type_params,
                    cands,
                    seen,
                );
            }
            Decl::Prop {
                name,
                is_public,
                is_static,
                ty,
                ..
            } => {
                if !matches_kind(*is_static, kind) {
                    continue;
                }
                let vis = vis_prefix(*is_public, false, false, false, *is_static, true);
                let ty_s = display_type(ty);
                let detail = format!("{vis}prop {name}: {ty_s}");
                push_candidate(cands, seen, Candidate {
                    label: name.clone(),
                    kind: KIND_VARIABLE,
                    detail,
                    insert_text: name.clone(),
                    insert_text_format: 1,
                    filter_text: name.clone(),
                });
            }
            _ => {}
        }
    }
}

fn matches_kind(is_static: bool, access: AccessKind) -> bool {
    match access {
        AccessKind::Static => is_static,
        AccessKind::Instance => !is_static,
    }
}