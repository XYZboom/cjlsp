// cj-sema: unused-declaration analysis.
//
// Reports declarations that are never referenced:
//   Function 'test01' is declared but never used
//   Variable 'y' is declared but never used
//   Parameter 'a' is declared but never used
//   Class 'B12' is declared but never used
//   Interface 'MyInterface' is declared but never used
//   Struct 'CDomainAccountInfo' member funcs 'toDomainAccountInfo'/'free'
//
// Severity is Hint (LSP severity 4), matching the official cjlsp diagnostics
// suite. Detection is per-file: collect every declared name (top-level AND
// class/struct/interface members, recursively) and every reference in the
// file, then report decls never referenced.
//
// Reference model (three disjoint sets, because the same spelling can name
// different symbols):
//   * names  — bare identifier references (`foo`, including call callees)
//   * members — member-access names (`this.x`, `super.m1`, `obj.method()`)
//   * calls  — bare-name call callees (`T(...)` -> type-named constructor use)
// A type-named constructor member is used only when `T(...)` is called; a
// bare `T` reference is a *type* use, not a constructor use (official 026).

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

/// A declared name collected for the unused check.
#[derive(Debug)]
struct DeclInfo {
    name: String,
    kind: DeclKind,
    line: u32,
    col: u32,
    /// true for a type-named constructor member (`T(...)` inside type T).
    is_type_ctor: bool,
}

/// Identifier references collected across the whole file.
#[derive(Debug, Default)]
pub struct Refs {
    /// bare identifier references (including call callees)
    pub names: HashSet<String>,
    /// member-access names on `this`/`super` (`this.x`, `super.m1`)
    members: HashSet<String>,
    /// bare-name call callees (`T(...)` uses the type-named constructor)
    calls: HashSet<String>,
    /// type-name references (parent types, type annotations, generic args)
    types: HashSet<String>,
}

/// Collect identifier references from an expression tree into `refs`.
fn collect_refs(e: &Expr, refs: &mut Refs) {
    match e {
        Expr::Name { name, .. } => {
            refs.names.insert(name.clone());
        }
        Expr::Call { callee, args, .. } => {
            if let Expr::Name { name, .. } = callee.as_ref() {
                refs.calls.insert(name.clone());
            }
            collect_refs(callee, refs);
            for a in args {
                collect_refs(&a.value, refs);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_refs(lhs, refs);
            collect_refs(rhs, refs);
        }
        Expr::Unary { inner, .. } => collect_refs(inner, refs),
        Expr::Paren { inner, .. } => collect_refs(inner, refs),
        Expr::Return {
            value: Some(value), ..
        } => {
            collect_refs(value, refs);
        }
        Expr::Member { object, name, .. } => {
            // A member access marks the *member name* as used only when the
            // receiver is `this`/`super`: `super.m1` uses m1, but `LibC.free`
            // must NOT mark a user method `free` as used (official 026).
            // Calls through named variables cannot be resolved by name analysis.
            if let Expr::Name { name: on, .. } = object.as_ref() {
                if on == "this" || on == "super" {
                    refs.members.insert(name.clone());
                }
            }
            collect_refs(object, refs);
        }
        Expr::Subscript { object, index, .. } => {
            collect_refs(object, refs);
            collect_refs(index, refs);
        }
        Expr::Assign { lhs, rhs, .. } => {
            collect_refs(lhs, refs);
            collect_refs(rhs, refs);
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                collect_refs(s, refs);
            }
        }
        Expr::If {
            cond, then, els, ..
        } => {
            collect_refs(cond, refs);
            collect_refs(then, refs);
            if let Some(e) = els {
                collect_refs(e, refs);
            }
        }
        Expr::Lambda { params, body, .. } => {
            for p in params {
                if let Some(d) = &p.default {
                    collect_refs(d, refs);
                }
            }
            collect_refs(body, refs);
        }
        Expr::LetPatternDestructor {
            initializer,
            patterns,
            ..
        } => {
            collect_refs(initializer, refs);
            for p in patterns {
                collect_pattern_refs(p, refs);
            }
        }
        _ => {}
    }
}

fn collect_pattern_refs(_p: &Pattern, _refs: &mut Refs) {
    // A Var pattern binds a new name (not a reference); ignore for used-set.
}

/// Collect type names referenced by a type node (parent types, annotations...).
/// `class C <: A` must mark `A` as used even though it is not an expression.
fn collect_type_names(t: &cj_ast::Type, out: &mut HashSet<String>) {
    use cj_ast::Type;
    match t {
        Type::Ref { name, args, .. } => {
            out.insert(name.clone());
            for a in args {
                collect_type_names(a, out);
            }
        }
        Type::Qualified { name, .. } => {
            out.insert(name.clone());
        }
        Type::Option { inner, .. } => collect_type_names(inner, out),
        Type::Constant { inner, .. } => collect_type_names(inner, out),
        Type::VArray { inner, .. } => collect_type_names(inner, out),
        Type::Paren { inner, .. } => collect_type_names(inner, out),
        Type::Func { params, ret, .. } => {
            for p in params {
                collect_type_names(p, out);
            }
            collect_type_names(ret, out);
        }
        Type::Tuple { elements, .. } => {
            for e in elements {
                collect_type_names(e, out);
            }
        }
        _ => {}
    }
}

/// Recursively collect declared names. `is_member` marks class-like body
/// members; `enclosing` is the innermost enclosing type name (to recognize
/// type-named constructors).
fn collect_decls(
    d: &Decl,
    is_member: bool,
    enclosing: Option<&str>,
    out: &mut Vec<DeclInfo>,
    known: &mut HashSet<String>,
) {
    match d {
        Decl::Func { name, name_pos, .. } => {
            out.push(DeclInfo {
                name: name.clone(),
                kind: DeclKind::Func,
                line: name_pos.line,
                col: name_pos.col,
                is_type_ctor: is_member && Some(name.as_str()) == enclosing,
            });
            if !is_member {
                known.insert(name.clone());
            }
        }
        Decl::Class {
            name,
            name_pos,
            members,
            ..
        } => {
            out.push(DeclInfo {
                name: name.clone(),
                kind: DeclKind::Class,
                line: name_pos.line,
                col: name_pos.col,
                is_type_ctor: false,
            });
            if !is_member {
                known.insert(name.clone());
            }
            for m in members {
                collect_decls(m, true, Some(name.as_str()), out, known);
            }
        }
        Decl::Interface {
            name,
            name_pos,
            members,
            ..
        } => {
            out.push(DeclInfo {
                name: name.clone(),
                kind: DeclKind::Interface,
                line: name_pos.line,
                col: name_pos.col,
                is_type_ctor: false,
            });
            if !is_member {
                known.insert(name.clone());
            }
            for m in members {
                collect_decls(m, true, Some(name.as_str()), out, known);
            }
        }
        Decl::Struct {
            name,
            name_pos,
            members,
            ..
        } => {
            out.push(DeclInfo {
                name: name.clone(),
                kind: DeclKind::Struct,
                line: name_pos.line,
                col: name_pos.col,
                is_type_ctor: false,
            });
            if !is_member {
                known.insert(name.clone());
            }
            for m in members {
                collect_decls(m, true, Some(name.as_str()), out, known);
            }
        }
        Decl::Enum { name, name_pos, .. } => {
            out.push(DeclInfo {
                name: name.clone(),
                kind: DeclKind::Enum,
                line: name_pos.line,
                col: name_pos.col,
                is_type_ctor: false,
            });
            if !is_member {
                known.insert(name.clone());
            }
        }
        Decl::Var { name, name_pos, .. } => {
            out.push(DeclInfo {
                name: name.clone(),
                kind: DeclKind::Var,
                line: name_pos.line,
                col: name_pos.col,
                is_type_ctor: false,
            });
            if !is_member {
                known.insert(name.clone());
            }
        }
        Decl::Extend { members, .. } => {
            for m in members {
                collect_decls(m, true, enclosing, out, known);
            }
        }
        // Prop / TypeAlias / PrimaryCtor / Macro / others: either reported
        // elsewhere or not part of the unused check (init is never unused).
        _ => {}
    }
}

/// Detect unused declarations. `file` is the parsed File; returns diagnostics.
pub fn detect_unused(file: &File) -> Vec<Diag> {
    let mut diags = Vec::new();

    // --- Pass 1: every declared name (top-level + members) + known top-level ---
    let mut decls: Vec<DeclInfo> = Vec::new();
    let mut known: HashSet<String> = HashSet::new();
    for d in &file.decls {
        collect_decls(d, false, None, &mut decls, &mut known);
    }

    // --- Pass 1b: top-level decls whose body/initializer references an
    // undefined name are not fully analyzable -> skip their unused report
    // (matches the official suite; member decls are exempt, official 026).
    let mut unresolved: HashSet<String> = HashSet::new();
    for d in &file.decls {
        let refs = decl_refs_of(d);
        let any_unresolved = refs
            .names
            .iter()
            .any(|r| !known.contains(r) && !is_builtin(r));
        if any_unresolved {
            if let Some(name) = decl_name_of(d) {
                unresolved.insert(name);
            }
        }
    }

    // --- Pass 2: collect every reference in the file (bodies + inits) ---
    let mut used = Refs::default();
    for d in &file.decls {
        collect_decl_refs(d, &mut used);
    }

    // --- Pass 3: report decls never referenced (except main) ---
    for info in &decls {
        if info.name == "main" {
            continue; // entry point is implicitly used
        }
        let used = if info.is_type_ctor {
            // Type-named constructor: only a `T(...)` call uses it.
            used.calls.contains(&info.name)
        } else {
            used.names.contains(&info.name)
                || used.members.contains(&info.name)
                || used.types.contains(&info.name)
        };
        if used || unresolved.contains(&info.name) {
            continue;
        }
        diags.push(unused_diag(
            info.line,
            info.col,
            format!(
                "{} '{}' is declared but never used",
                info.kind.label(),
                info.name
            ),
        ));
    }

    // --- Pass 4: unused function parameters (functions only, incl. members). ---
    for d in &file.decls {
        collect_unused_params(d, None, &mut diags);
    }

    diags
}

/// Report parameters not referenced inside their own function body
/// (top-level funcs and class-like member funcs alike). Parameters of a
/// type-named constructor are skipped: they double as instance properties
/// (`public let x: T`) and are used across the whole type body.
fn collect_unused_params(d: &Decl, enclosing: Option<&str>, diags: &mut Vec<Diag>) {
    match d {
        Decl::Func {
            name, body, params, ..
        } => {
            let is_type_ctor = enclosing == Some(name.as_str());
            if !is_type_ctor {
                // Params are checked even when the body failed to parse
                // (Body::Empty) — matches the original top-level behavior.
                let mut body_refs = Refs::default();
                if let Body::Block(stmts) = body {
                    for s in stmts {
                        collect_refs(s, &mut body_refs);
                    }
                }
                for p in params {
                    if !body_refs.names.contains(&p.name) && !body_refs.members.contains(&p.name) {
                        diags.push(unused_diag(
                            p.pos.line,
                            p.pos.col,
                            format!("Parameter '{}' is declared but never used", p.name),
                        ));
                    }
                }
            }
        }
        Decl::Class { name, members, .. }
        | Decl::Interface { name, members, .. }
        | Decl::Struct { name, members, .. } => {
            for m in members {
                collect_unused_params(m, Some(name.as_str()), diags);
            }
        }
        Decl::Extend { members, .. } => {
            for m in members {
                collect_unused_params(m, enclosing, diags);
            }
        }
        _ => {}
    }
}

/// Names referenced by a single top-level decl (for the unresolved check).
fn decl_refs_of(d: &Decl) -> Refs {
    let mut refs = Refs::default();
    match d {
        Decl::Func {
            body: Body::Block(stmts),
            ..
        } => {
            for s in stmts {
                collect_refs(s, &mut refs);
            }
        }
        Decl::Var {
            init: Some(init), ..
        } => collect_refs(init, &mut refs),
        _ => {}
    }
    refs
}

fn decl_name_of(d: &Decl) -> Option<String> {
    match d {
        Decl::Func { name, .. }
        | Decl::Class { name, .. }
        | Decl::Interface { name, .. }
        | Decl::Struct { name, .. }
        | Decl::Enum { name, .. }
        | Decl::Var { name, .. } => Some(name.clone()),
        _ => None,
    }
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

/// Collect all identifier references in a declaration (top-level and members),
/// including type-name references (parents, annotations, generic args).
pub fn collect_decl_refs(d: &Decl, refs: &mut Refs) {
    match d {
        Decl::Func {
            params,
            ret,
            type_params,
            body: Body::Block(stmts),
            ..
        } => {
            for p in params {
                collect_type_names(&p.ty, &mut refs.types);
            }
            if let Some(r) = ret {
                collect_type_names(r, &mut refs.types);
            }
            for tp in type_params {
                for b in &tp.bounds {
                    collect_type_names(b, &mut refs.types);
                }
            }
            for s in stmts {
                collect_refs(s, refs);
            }
        }
        Decl::PrimaryCtor { params, body, .. } => {
            for p in params {
                collect_type_names(&p.ty, &mut refs.types);
            }
            if let Body::Block(stmts) = body {
                for s in stmts {
                    collect_refs(s, refs);
                }
            }
        }
        Decl::Var { ty, init, .. } => {
            if let Some(t) = ty {
                collect_type_names(t, &mut refs.types);
            }
            if let Some(i) = init {
                collect_refs(i, refs);
            }
        }
        Decl::Prop { ty, .. } => collect_type_names(ty, &mut refs.types),
        Decl::TypeAlias { target, .. } => collect_type_names(target, &mut refs.types),
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
                collect_type_names(p, &mut refs.types);
            }
            for tp in type_params {
                for b in &tp.bounds {
                    collect_type_names(b, &mut refs.types);
                }
            }
            for m in members {
                collect_decl_refs(m, refs);
            }
        }
        Decl::Struct {
            type_params,
            members,
            ..
        } => {
            for tp in type_params {
                for b in &tp.bounds {
                    collect_type_names(b, &mut refs.types);
                }
            }
            for m in members {
                collect_decl_refs(m, refs);
            }
        }
        Decl::Extend {
            target, members, ..
        } => {
            collect_type_names(target, &mut refs.types);
            for m in members {
                collect_decl_refs(m, refs);
            }
        }
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

    #[test]
    fn unused_struct_member_funcs() {
        let (file, _) = parse_source(
            "struct S {\n  S() {}\n  func a() {}\n  func b() { c() }\n  func c() {}\n}\n",
        );
        let diags = detect_unused(&file);
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        // a() unused; b() unused (nothing calls it); c() used by b's body;
        // ctor S() unused (never called as S(...)).
        assert!(msgs.iter().any(|m| m.contains("Function 'a'")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Function 'b'")), "{msgs:?}");
        assert!(!msgs.iter().any(|m| m.contains("Function 'c'")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Function 'S'")), "{msgs:?}");
    }

    #[test]
    fn init_never_reported_unused() {
        let (file, _) = parse_source("class C {\n  init() {}\n}\n");
        let diags = detect_unused(&file);
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(!msgs.iter().any(|m| m.contains("'init'")), "{msgs:?}");
    }

    #[test]
    fn finalizer_reported_unused() {
        let (file, _) = parse_source("class C {\n  ~init() {}\n}\n");
        let diags = detect_unused(&file);
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(
            msgs.iter().any(|m| m.contains("Function '~init'")),
            "{msgs:?}"
        );
    }

    #[test]
    fn member_var_used_via_super() {
        let (file, _) = parse_source(
            "open class A {\n  var m1: Int32 = 1\n}\nclass C <: A {\n  var a: Int32 = 10\n  let b: Int32 = super.m1\n}\n",
        );
        let diags = detect_unused(&file);
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        // m1 used via super.m1; a/b unused; C unused
        assert!(!msgs.iter().any(|m| m.contains("'m1'")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Variable 'a'")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Variable 'b'")), "{msgs:?}");
    }

    #[test]
    fn type_named_ctor_used_only_by_call() {
        let (file, _) = parse_source("struct T {\n  T() {}\n}\nfunc f() {\n  T\n}\n");
        let diags = detect_unused(&file);
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        // bare `T` is a type use -> struct not unused, ctor still unused
        assert!(!msgs.iter().any(|m| m.contains("Struct 'T'")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("Function 'T'")), "{msgs:?}");
    }

    #[test]
    fn type_named_ctor_used_by_call() {
        let (file, _) = parse_source("struct T {\n  T() {}\n}\nfunc f() {\n  T()\n}\n");
        let diags = detect_unused(&file);
        let msgs: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(!msgs.iter().any(|m| m.contains("'T'")), "{msgs:?}");
    }
}
