// cj-sema: targeted semantic checks driving the remaining cjlsp diagnostics
// suite cases (var-init ordering, undeclared types, super/finalizer rules,
// override parameter names, optional params in abstract functions, and bare
// type-name expressions).
//
// Each check follows the official compiler's message text and anchor position
// (1-based line/col in the Diag; the LSP layer converts to 0-based).

use cj_ast::{Body, Decl, Expr, File, Type};
use cj_diag::Diag;
use std::collections::{HashMap, HashSet};

use crate::PackageTable;

/// A bare-name reference with its source position.
struct NameRef {
    name: String,
    pos: cj_ast::CodePos,
}

/// Walk an expression tree, invoking `on_name` for every bare identifier
/// reference (including call callees, member receivers, etc.).
fn walk_names(e: &Expr, on_name: &mut dyn FnMut(&NameRef)) {
    let mut visit = |e: &Expr| walk_names(e, on_name);
    match e {
        Expr::Name { name, pos, .. } => {
            on_name(&NameRef {
                name: name.clone(),
                pos: *pos,
            });
        }
        Expr::Call { callee, args, .. } => {
            visit(callee);
            for a in args {
                visit(&a.value);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            visit(lhs);
            visit(rhs);
        }
        Expr::Unary { inner, .. } => visit(inner),
        Expr::Paren { inner, .. } => visit(inner),
        Expr::Optional { inner, .. } => visit(inner),
        Expr::OptionalChain { inner, .. } => visit(inner),
        Expr::Return { value: Some(v), .. } => visit(v),
        Expr::IncOrDec { inner, .. } => visit(inner),
        Expr::Subscript { object, index, .. } => {
            visit(object);
            visit(index);
        }
        Expr::Member { object, .. } => visit(object),
        Expr::Is { inner, .. } => visit(inner),
        Expr::As { inner, .. } => visit(inner),
        Expr::Assign { lhs, rhs, .. } => {
            visit(lhs);
            visit(rhs);
        }
        Expr::Range { start, end, .. } => {
            visit(start);
            visit(end);
        }
        Expr::ArrayLit { elements, .. }
        | Expr::Array { elements, .. }
        | Expr::Tuple { elements, .. } => {
            for el in elements {
                visit(el);
            }
        }
        Expr::Pointer { inner, .. } => visit(inner),
        Expr::Match {
            scrutinee, cases, ..
        } => {
            visit(scrutinee);
            for c in cases {
                if let Some(g) = &c.guard {
                    visit(g);
                }
                visit(&c.body);
            }
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                visit(s);
            }
        }
        Expr::If {
            cond, then, els, ..
        } => {
            visit(cond);
            visit(then);
            if let Some(e) = els {
                visit(e);
            }
        }
        Expr::LetPatternDestructor { initializer, .. } => visit(initializer),
        Expr::Interpolation { parts, .. } | Expr::StrInterpolation { parts, .. } => {
            for p in parts {
                if let cj_ast::InterpPart::Expr(x) = p {
                    visit(x);
                }
            }
        }
        Expr::Quote { parts, .. } => {
            for p in parts {
                visit(p);
            }
        }
        Expr::Try {
            body,
            catches,
            finally,
            ..
        } => {
            visit(body);
            for c in catches {
                visit(&c.body);
            }
            if let Some(f) = finally {
                visit(f);
            }
        }
        Expr::While { cond, body, .. } | Expr::DoWhile { cond, body, .. } => {
            visit(cond);
            visit(body);
        }
        Expr::Lambda { body, .. } => visit(body),
        Expr::TrailingClosure { call, closure, .. } => {
            visit(call);
            visit(closure);
        }
        Expr::ForIn { iter, body, .. } => {
            visit(iter);
            visit(body);
        }
        Expr::TypeConv { inner, .. } => visit(inner),
        Expr::Throw { inner, .. } => visit(inner),
        Expr::Perform { inner, .. } => visit(inner),
        Expr::Resume { inner, .. } => visit(inner),
        Expr::Spawn { inner, .. } => visit(inner),
        Expr::Synchronized { inner, .. } => visit(inner),
        Expr::IfAvailable { then, els, .. } => {
            visit(then);
            if let Some(e) = els {
                visit(e);
            }
        }
        _ => {}
    }
}

/// Collect every declared (reference) type name in the file with its position.
/// Skips the builtin type names (the set used by resolver/unused).
fn collect_used_type_names(d: &Decl, out: &mut Vec<NameRef>) {
    match d {
        Decl::Func { params, ret, .. } => {
            for p in params {
                collect_type_refs(&p.ty, out);
            }
            if let Some(r) = ret {
                collect_type_refs(r, out);
            }
        }
        Decl::Macro { params, .. } => {
            for p in params {
                collect_type_refs(&p.ty, out);
            }
        }
        Decl::Var { ty: Some(t), .. } => collect_type_refs(t, out),
        Decl::Prop { ty, .. } => collect_type_refs(ty, out),
        Decl::TypeAlias { target, .. } => collect_type_refs(target, out),
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
                collect_type_refs(p, out);
            }
            for tp in type_params {
                for b in &tp.bounds {
                    collect_type_refs(b, out);
                }
            }
            for m in members {
                collect_used_type_names(m, out);
            }
        }
        Decl::Struct {
            type_params,
            members,
            ..
        } => {
            for tp in type_params {
                for b in &tp.bounds {
                    collect_type_refs(b, out);
                }
            }
            for m in members {
                collect_used_type_names(m, out);
            }
        }
        Decl::Extend {
            target, members, ..
        } => {
            collect_type_refs(target, out);
            for m in members {
                collect_used_type_names(m, out);
            }
        }
        Decl::PrimaryCtor { params, .. } => {
            for p in params {
                collect_type_refs(&p.ty, out);
            }
        }
        _ => {}
    }
}

fn collect_type_refs(t: &Type, out: &mut Vec<NameRef>) {
    match t {
        Type::Ref { name, pos, args } => {
            out.push(NameRef {
                name: name.clone(),
                pos: *pos,
            });
            for a in args {
                collect_type_refs(a, out);
            }
        }
        Type::Qualified { name, pos } => {
            // qualified names (a.b.C) are left to later phases
            let _ = name;
            let _ = pos;
        }
        Type::Option { inner, .. } => collect_type_refs(inner, out),
        Type::Constant { inner, .. } => collect_type_refs(inner, out),
        Type::VArray { inner, .. } => collect_type_refs(inner, out),
        Type::Paren { inner, .. } => collect_type_refs(inner, out),
        Type::Func { params, ret, .. } => {
            for p in params {
                collect_type_refs(p, out);
            }
            collect_type_refs(ret, out);
        }
        Type::Tuple { elements, .. } => {
            for e in elements {
                collect_type_refs(e, out);
            }
        }
        _ => {}
    }
}

/// Walk the initializers of all top-level variable declarations in order.
/// A reference to a variable declared *after* the referencing declaration is a
/// use-before-initialization error (official 004):
///   var y = 1 + testvar4      // testvar4 used before it is declared
///   let testvar4 = 30
fn check_used_before_init(file: &File) -> Vec<Diag> {
    // Collect top-level vars in declaration order: (name, name_pos).
    let mut vars: Vec<(String, cj_ast::CodePos)> = Vec::new();
    for d in &file.decls {
        if let Decl::Var { name, name_pos, .. } = d {
            vars.push((name.clone(), *name_pos));
        }
    }
    let mut diags = Vec::new();
    for d in &file.decls {
        // Only initializers of variables can "use before initialize"
        // (function bodies may legitimately call forward-declared funcs).
        let (cur_name, init) = match d {
            Decl::Var {
                name,
                init: Some(i),
                ..
            } => (name.as_str(), i),
            _ => continue,
        };
        let mut refs: Vec<NameRef> = Vec::new();
        walk_names(init, &mut |r: &NameRef| {
            refs.push(NameRef {
                name: r.name.clone(),
                pos: r.pos,
            })
        });
        for r in &refs {
            // Is `r.name` one of the later-declared top-level vars?
            let later = vars
                .iter()
                .find(|(n, pos)| n == &r.name && pos.offset > r.pos.offset);
            if let Some((later_name, _)) = later {
                diags.push(Diag::error(
                    r.pos.line,
                    r.pos.col,
                    format!(
                        "global/static variable '{}' is used before initialization \
                         during initializing '{}'",
                        later_name, cur_name
                    ),
                ));
                diags.push(Diag::error(
                    r.pos.line,
                    r.pos.col,
                    format!("variable '{}' is used before being defined", later_name),
                ));
            }
        }
    }
    diags
}

/// Report references to type names that are neither builtin nor declared in
/// the package (official 016: `func test04(a:Int8,b:myBool)`).
fn check_undeclared_type(file: &File, package: &PackageTable) -> Vec<Diag> {
    // Package-declared type names (this file's top-level symbols).
    let mut declared: HashSet<&str> = HashSet::new();
    for sym in package.symbols.values().flatten() {
        use crate::SymbolKind;
        if matches!(
            sym.kind,
            SymbolKind::Class
                | SymbolKind::Interface
                | SymbolKind::Struct
                | SymbolKind::Enum
                | SymbolKind::TypeAlias
        ) {
            declared.insert(sym.name.as_str());
        }
    }
    let mut diags = Vec::new();
    for d in &file.decls {
        let mut refs: Vec<NameRef> = Vec::new();
        collect_used_type_names(d, &mut refs);
        for r in &refs {
            if !declared.contains(r.name.as_str()) && !is_builtin_type(&r.name) {
                diags.push(Diag::error(
                    r.pos.line,
                    r.pos.col,
                    format!("undeclared type name '{}'", r.name),
                ));
            }
        }
    }
    diags
}

/// `'super'` may not be used to initialize a non-static class member (official
/// 028: `let b: Int32 = super.m1` inside a class body).
fn check_super_init(file: &File) -> Vec<Diag> {
    let mut diags = Vec::new();
    for d in &file.decls {
        collect_super_init(d, &mut diags);
    }
    diags
}

fn collect_super_init(d: &Decl, diags: &mut Vec<Diag>) {
    match d {
        Decl::Class { members, .. } | Decl::Struct { members, .. } => {
            for m in members {
                // Non-static member initializers only.
                if let Decl::Var {
                    init: Some(init), ..
                } = m
                {
                    walk_names(init, &mut |r: &NameRef| {
                        if r.name == "super" {
                            diags.push(Diag::error(
                                r.pos.line,
                                r.pos.col,
                                "'super' is not allowed to initialize non-static member",
                            ));
                        }
                    });
                }
                // `super` used inside init/func bodies is fine; only member
                // initializers are checked (function bodies handle super
                // legally as `super.init()` / `super.method()`).
            }
        }
        _ => {}
    }
}

/// Finalizer (destructor `~init`) rules from official 029:
///   - cannot have parameters                    (187, 566)
///   - forbidden in `open` classes               (269)
///   - `this` may not be used in the body        (276)
///   - may not carry access/open modifiers       (633, requires source scan)
fn check_finalizers(file: &File, src: Option<&str>) -> Vec<Diag> {
    let mut diags = Vec::new();
    for d in &file.decls {
        if let Decl::Class {
            name,
            is_open,
            members,
            ..
        } = d
        {
            for m in members {
                if let Decl::Func {
                    name: fname,
                    name_pos,
                    params,
                    body,
                    ..
                } = m
                {
                    if fname != "~init" {
                        continue;
                    }
                    if !params.is_empty() {
                        diags.push(Diag::error(
                            name_pos.line,
                            name_pos.col,
                            "finalizer cannot have parameter",
                        ));
                        // Official anchors "cannot have any parameter" at the
                        // `(` after `~init` (one column past the 5-char name).
                        let paren_col = name_pos.col + 5;
                        diags.push(Diag::error(
                            name_pos.line,
                            paren_col,
                            "finalizer cannot have any parameter",
                        ));
                    }
                    if *is_open {
                        diags.push(Diag::error(
                            name_pos.line,
                            name_pos.col,
                            format!("finalizer is forbidden in class '{name}' that is open"),
                        ));
                    }
                    if let Body::Block(stmts) = body {
                        for s in stmts {
                            walk_names(s, &mut |r: &NameRef| {
                                if r.name == "this" {
                                    diags.push(Diag::error(
                                        r.pos.line,
                                        r.pos.col,
                                        "'this' cannot be used as an expression in the finalizer",
                                    ));
                                }
                            });
                        }
                    }
                }
            }
        }
    }
    // Modifier checks need the raw source text (the parser drops modifiers);
    // find any access/open/static modifier token before `~init` on its line.
    if let Some(src_text) = src {
        let lines: Vec<&str> = src_text.lines().collect();
        for d in &file.decls {
            if let Decl::Class { members, .. } = d {
                for m in members {
                    if let Decl::Func {
                        name: fname,
                        name_pos,
                        ..
                    } = m
                    {
                        if fname != "~init" {
                            continue;
                        }
                        let Some(line_str) = lines.get(name_pos.line.saturating_sub(1) as usize)
                        else {
                            continue;
                        };
                        let search = line_str.as_bytes();
                        // Only scan up to the `~` of `~init` — comments and
                        // code after the finalizer must not trigger the scan.
                        let Some(tilde) = search.iter().position(|b| *b == b'~') else {
                            continue;
                        };
                        for (kw, kw_len) in FINALIZER_MODIFIERS {
                            if tilde < *kw_len || tilde > search.len() {
                                continue;
                            }
                            let mut idx = 0usize;
                            while idx + kw_len <= tilde {
                                if &search[idx..idx + kw_len] == kw.as_bytes() {
                                    let before_ok =
                                        idx == 0 || !search[idx - 1].is_ascii_alphanumeric();
                                    let after_ok = idx + kw_len >= tilde
                                        || !search[idx + kw_len].is_ascii_alphanumeric();
                                    if before_ok && after_ok {
                                        // 1-based col = chars before the keyword + 1
                                        let col = line_str[..idx].chars().count() as u32 + 1;
                                        diags.push(Diag::error(
                                            name_pos.line,
                                            col,
                                            format!(
                                                "unexpected modifier '{}' on finalizer in class body",
                                                kw
                                            ),
                                        ));
                                        break;
                                    }
                                }
                                idx += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    diags
}

const FINALIZER_MODIFIERS: &[(&str, usize)] = &[
    ("private", 7),
    ("protected", 9),
    ("public", 6),
    ("internal", 8),
    ("static", 6),
    ("open", 4),
    ("sealed", 6),
    ("abstract", 8),
];

/// Optional parameters are forbidden in abstract functions (official 030:
/// `interface MyInterface { func f2(a!: Int32 = 1): Unit }`).
fn check_optional_param_abstract(file: &File) -> Vec<Diag> {
    let mut diags = Vec::new();
    for d in &file.decls {
        collect_optional_param_abstract(d, &mut diags);
    }
    diags
}

fn collect_optional_param_abstract(d: &Decl, diags: &mut Vec<Diag>) {
    match d {
        Decl::Func {
            is_abstract,
            params,
            ..
        } => {
            if *is_abstract {
                for p in params {
                    if p.is_named {
                        diags.push(Diag::error(
                            p.pos.line,
                            p.pos.col,
                            "optional parameter cannot be used in abstract function",
                        ));
                    }
                }
            }
        }
        Decl::Interface { members, .. } => {
            // Interface methods are abstract by definition.
            for m in members {
                if let Decl::Func { params, .. } = m {
                    for p in params {
                        if p.is_named {
                            diags.push(Diag::error(
                                p.pos.line,
                                p.pos.col,
                                "optional parameter cannot be used in abstract function",
                            ));
                        }
                    }
                }
            }
        }
        Decl::Class { members, .. } | Decl::Struct { members, .. } => {
            for m in members {
                collect_optional_param_abstract(m, diags);
            }
        }
        _ => {}
    }
}

/// An overriding method must keep the base method's parameter names
/// (official 024: C12 overrides test12 with `b!` instead of `a!`).
fn check_override_param_names(file: &File) -> Vec<Diag> {
    // index top-level class-like decls by name
    let mut types: HashMap<String, &Decl> = HashMap::new();
    for d in &file.decls {
        if let Decl::Class { name, .. } | Decl::Interface { name, .. } | Decl::Struct { name, .. } =
            d
        {
            types.insert(name.clone(), d);
        }
    }
    let mut diags = Vec::new();
    for d in &file.decls {
        let (parents, members) = match d {
            Decl::Class {
                parents, members, ..
            }
            | Decl::Interface {
                parents, members, ..
            } => (parents, members),
            _ => continue,
        };
        for p in parents {
            let parent_name = match p {
                Type::Ref { name, .. } | Type::Qualified { name, .. } => name.as_str(),
                _ => continue,
            };
            let Some(parent) = types.get(parent_name) else {
                continue;
            };
            let parent_members: Vec<&Decl> = match parent {
                Decl::Class { members, .. } | Decl::Struct { members, .. } => {
                    members.iter().collect()
                }
                _ => continue,
            };
            for m in members {
                let child = match m {
                    Decl::Func { .. } => m,
                    _ => continue,
                };
                let cname = decl_str(child, "name").unwrap_or_default();
                // find a same-named parent method
                for pm in &parent_members {
                    let pname = decl_str(pm, "name").unwrap_or_default();
                    if pname != cname || cname.is_empty() {
                        continue;
                    }
                    // B12 overrides its own base A12: names must match.
                    let child_params: Vec<&str> = func_param_names(child);
                    let parent_params: Vec<&str> = func_param_names(pm);
                    if child_params != parent_params {
                        if let Decl::Func { name_pos, .. } = child {
                            // Official anchors at the child method's name
                            // (024 expects L21:22 = C12.test12), not the param.
                            diags.push(Diag::error(
                                name_pos.line,
                                name_pos.col,
                                "parameter name mismatched",
                            ));
                        }
                        break;
                    }
                }
            }
        }
    }
    diags
}

fn func_param_names(d: &Decl) -> Vec<&str> {
    match d {
        Decl::Func { params, .. } => params.iter().map(|p| p.name.as_str()).collect(),
        _ => Vec::new(),
    }
}

fn decl_str<'a>(d: &'a Decl, field: &str) -> Option<&'a str> {
    match (field, d) {
        ("name", Decl::Func { name, .. })
        | ("name", Decl::Class { name, .. })
        | ("name", Decl::Interface { name, .. })
        | ("name", Decl::Struct { name, .. })
        | ("name", Decl::Var { name, .. }) => Some(name.as_str()),
        _ => None,
    }
}

/// A bare type name used as a value expression requires a member access or
/// constructor call after it (official 026: `func foo() { CDomainAccountInfo }`).
fn check_bare_type_expr(file: &File, package: &PackageTable) -> Vec<Diag> {
    let mut type_names: HashSet<String> = HashSet::new();
    for sym in package.symbols.values().flatten() {
        use crate::SymbolKind;
        if matches!(
            sym.kind,
            SymbolKind::Class | SymbolKind::Interface | SymbolKind::Struct | SymbolKind::Enum
        ) {
            type_names.insert(sym.name.clone());
        }
    }
    let mut diags = Vec::new();
    for d in &file.decls {
        collect_bare_type_expr(d, &type_names, &mut diags);
    }
    diags
}

fn collect_bare_type_expr(d: &Decl, type_names: &HashSet<String>, diags: &mut Vec<Diag>) {
    match d {
        Decl::Func {
            body: Body::Block(stmts),
            ..
        }
        | Decl::Main {
            body: Body::Block(stmts),
            ..
        } => {
            for s in stmts {
                diags.extend(check_bare_type_in_expr(s, type_names));
            }
        }
        Decl::Class { members, .. }
        | Decl::Interface { members, .. }
        | Decl::Struct { members, .. }
        | Decl::Extend { members, .. } => {
            for m in members {
                collect_bare_type_expr(m, type_names, diags);
            }
        }
        _ => {}
    }
}

/// Recursively find a bare type-name expression that is not a call callee.
fn check_bare_type_in_expr(e: &Expr, type_names: &HashSet<String>) -> Vec<Diag> {
    let mut diags = Vec::new();
    if let Expr::Name { name, pos, .. } = e {
        // A type name used as a value: reported unless it's a call callee
        // (constructor call) — call sites are handled below.
        if type_names.contains(name) {
            diags.push(Diag::error(
                pos.line,
                pos.col,
                format!(
                    "expected member name or constructor call after '{}' type name",
                    name
                ),
            ));
        }
    }
    match e {
        Expr::Call { callee, args, .. } => {
            // The callee of a call is a legitimate constructor/member use —
            // don't report it. Recurse into its args and non-name callees.
            if !matches!(callee.as_ref(), Expr::Name { .. }) {
                diags.extend(check_bare_type_in_expr(callee, type_names));
            }
            for a in args {
                diags.extend(check_bare_type_in_expr(&a.value, type_names));
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            diags.extend(check_bare_type_in_expr(lhs, type_names));
            diags.extend(check_bare_type_in_expr(rhs, type_names));
        }
        Expr::Unary { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::Paren { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::Optional { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::OptionalChain { inner, .. } => {
            diags.extend(check_bare_type_in_expr(inner, type_names))
        }
        Expr::Return { value: Some(v), .. } => diags.extend(check_bare_type_in_expr(v, type_names)),
        Expr::IncOrDec { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::Subscript { object, index, .. } => {
            diags.extend(check_bare_type_in_expr(object, type_names));
            diags.extend(check_bare_type_in_expr(index, type_names));
        }
        Expr::Member { object, .. } => diags.extend(check_bare_type_in_expr(object, type_names)),
        Expr::Is { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::As { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::Assign { lhs, rhs, .. } => {
            diags.extend(check_bare_type_in_expr(lhs, type_names));
            diags.extend(check_bare_type_in_expr(rhs, type_names));
        }
        Expr::Range { start, end, .. } => {
            diags.extend(check_bare_type_in_expr(start, type_names));
            diags.extend(check_bare_type_in_expr(end, type_names));
        }
        Expr::ArrayLit { elements, .. }
        | Expr::Array { elements, .. }
        | Expr::Tuple { elements, .. } => {
            for el in elements {
                diags.extend(check_bare_type_in_expr(el, type_names));
            }
        }
        Expr::Pointer { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::Match {
            scrutinee, cases, ..
        } => {
            diags.extend(check_bare_type_in_expr(scrutinee, type_names));
            for c in cases {
                if let Some(g) = &c.guard {
                    diags.extend(check_bare_type_in_expr(g, type_names));
                }
                diags.extend(check_bare_type_in_expr(&c.body, type_names));
            }
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                diags.extend(check_bare_type_in_expr(s, type_names));
            }
        }
        Expr::If {
            cond, then, els, ..
        } => {
            diags.extend(check_bare_type_in_expr(cond, type_names));
            diags.extend(check_bare_type_in_expr(then, type_names));
            if let Some(e) = els {
                diags.extend(check_bare_type_in_expr(e, type_names));
            }
        }
        Expr::LetPatternDestructor { initializer, .. } => {
            diags.extend(check_bare_type_in_expr(initializer, type_names))
        }
        Expr::Interpolation { parts, .. } | Expr::StrInterpolation { parts, .. } => {
            for p in parts {
                if let cj_ast::InterpPart::Expr(x) = p {
                    diags.extend(check_bare_type_in_expr(x, type_names));
                }
            }
        }
        Expr::Try {
            body,
            catches,
            finally,
            ..
        } => {
            diags.extend(check_bare_type_in_expr(body, type_names));
            for c in catches {
                diags.extend(check_bare_type_in_expr(&c.body, type_names));
            }
            if let Some(f) = finally {
                diags.extend(check_bare_type_in_expr(f, type_names));
            }
        }
        Expr::While { cond, body, .. } | Expr::DoWhile { cond, body, .. } => {
            diags.extend(check_bare_type_in_expr(cond, type_names));
            diags.extend(check_bare_type_in_expr(body, type_names));
        }
        Expr::Lambda { body, .. } => diags.extend(check_bare_type_in_expr(body, type_names)),
        Expr::TrailingClosure { call, closure, .. } => {
            diags.extend(check_bare_type_in_expr(call, type_names));
            diags.extend(check_bare_type_in_expr(closure, type_names));
        }
        Expr::ForIn { iter, body, .. } => {
            diags.extend(check_bare_type_in_expr(iter, type_names));
            diags.extend(check_bare_type_in_expr(body, type_names));
        }
        Expr::TypeConv { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::Throw { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::Perform { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::Resume { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::Spawn { inner, .. } => diags.extend(check_bare_type_in_expr(inner, type_names)),
        Expr::Synchronized { inner, .. } => {
            diags.extend(check_bare_type_in_expr(inner, type_names))
        }
        _ => {}
    }
    diags
}

/// Names treated as builtin types (never reported as undeclared).
fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "String"
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
            | "CString"
            | "UIntPtr"
            | "IntPtr"
            | "BigInt"
            | "BigFloat"
    )
}

/// Run all targeted semantic checks over a parsed file and return diagnostics.
/// `src` is the raw source text — only used for checks that need token text
/// the parser does not preserve (finalizer modifiers, @Deprecated annotations).
pub fn check_semantics(file: &File, package: &PackageTable, src: Option<&str>) -> Vec<Diag> {
    let mut diags = Vec::new();
    diags.extend(check_used_before_init(file));
    diags.extend(check_undeclared_type(file, package));
    diags.extend(check_super_init(file));
    diags.extend(check_finalizers(file, src));
    diags.extend(check_optional_param_abstract(file));
    diags.extend(check_override_param_names(file));
    diags.extend(check_bare_type_expr(file, package));
    diags.extend(check_deprecated_refs(file, src));
    diags
}

/// References to a `@Deprecated`-annotated top-level variable produce a
/// deprecation warning at the reference (official 027: `var x = PI`).
/// The annotation is not preserved by the parser, so it is recovered from the
/// source text: the line immediately above the variable's declaration.
fn check_deprecated_refs(file: &File, src: Option<&str>) -> Vec<Diag> {
    let Some(src_text) = src else {
        return Vec::new();
    };
    let lines: Vec<&str> = src_text.lines().collect();
    // top-level var name -> deprecated?
    let mut deprecated: HashSet<String> = HashSet::new();
    for d in &file.decls {
        if let Decl::Var { name, name_pos, .. } = d {
            let line_idx = name_pos.line.saturating_sub(1) as usize;
            let mut probe = line_idx;
            // Walk up past blank/comment/annotation lines; the first non-
            // blank content line decides.
            while probe > 0 {
                let l = lines.get(probe - 1).map(|s| s.trim()).unwrap_or("");
                if l.is_empty() || l.starts_with("//") {
                    probe -= 1;
                    continue;
                }
                if l.starts_with("@Deprecated") {
                    deprecated.insert(name.clone());
                }
                break;
            }
        }
    }
    if deprecated.is_empty() {
        return Vec::new();
    }
    let mut diags = Vec::new();
    for d in &file.decls {
        collect_deprecated_refs(d, &deprecated, &mut diags);
    }
    diags
}

fn collect_deprecated_refs(d: &Decl, deprecated: &HashSet<String>, diags: &mut Vec<Diag>) {
    match d {
        Decl::Func {
            body: Body::Block(stmts),
            ..
        }
        | Decl::Main {
            body: Body::Block(stmts),
            ..
        } => {
            for s in stmts {
                walk_names(s, &mut |r: &NameRef| {
                    if deprecated.contains(&r.name) {
                        diags.push(Diag::warning(
                            r.pos.line,
                            r.pos.col,
                            format!(
                                "variable '{}' is deprecated, this warning can be suppressed \
                                 by setting the compiler option `-Woff deprecated`",
                                r.name
                            ),
                        ));
                    }
                });
            }
        }
        Decl::Class { members, .. }
        | Decl::Interface { members, .. }
        | Decl::Struct { members, .. }
        | Decl::Extend { members, .. } => {
            for m in members {
                collect_deprecated_refs(m, deprecated, diags);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Collector;
    use cj_parser::parse_source;

    fn check(src: &str) -> Vec<Diag> {
        let (file, _) = parse_source(src);
        let r = Collector::new().collect_file(&file);
        let mut pkg = PackageTable::default();
        pkg.merge(&r);
        check_semantics(&file, &pkg, Some(src))
    }

    fn has(diags: &[Diag], needle: &str) -> bool {
        diags.iter().any(|d| d.message.contains(needle))
    }

    #[test]
    fn used_before_init() {
        let diags = check("var y = 1 + testvar4\nlet testvar4 = 30\n");
        assert!(
            has(&diags, "used before initialization during initializing 'y'"),
            "{diags:?}"
        );
        assert!(
            has(&diags, "variable 'testvar4' is used before being defined"),
            "{diags:?}"
        );
        // the reference is at `testvar4` (line 1 col 13)
        assert!(
            diags.iter().any(|d| d.line == 1 && d.col == 13),
            "{diags:?}"
        );
    }

    #[test]
    fn undeclared_type_name() {
        let diags = check("func test04(a:Int8,b:myBool) {}\n");
        assert!(has(&diags, "undeclared type name 'myBool'"), "{diags:?}");
        assert!(
            diags.iter().any(|d| d.line == 1 && d.col == 22),
            "{diags:?}"
        );
        // declared types are not reported
        let ok = check("class A {}\nfunc f(a: A) {}\n");
        assert!(!has(&ok, "undeclared type name"), "{ok:?}");
    }

    #[test]
    fn super_not_allowed_to_init_member() {
        let diags = check(
            "open class A {\n    var m1: Int32 = 1\n}\nclass C <: A {\n    let b: Int32 = super.m1\n}\n",
        );
        assert!(
            has(
                &diags,
                "'super' is not allowed to initialize non-static member"
            ),
            "{diags:?}"
        );
        // anchored at `super` (line 5)
        assert!(
            diags.iter().any(|d| d.line == 5 && d.col == 20),
            "{diags:?}"
        );
    }

    #[test]
    fn finalizer_rules() {
        let d1 = check("class C {\n    ~init(x: Int64) {}\n}\n");
        assert!(has(&d1, "finalizer cannot have parameter"), "{d1:?}");
        assert!(has(&d1, "finalizer cannot have any parameter"), "{d1:?}");

        let d2 = check("class C {\n    private ~init() {}\n}\n");
        assert!(
            has(&d2, "unexpected modifier 'private' on finalizer"),
            "{d2:?}"
        );

        let d3 = check("open class C {\n    ~init() {}\n}\n");
        assert!(
            has(&d3, "finalizer is forbidden in class 'C' that is open"),
            "{d3:?}"
        );

        let d4 = check("class C {\n    ~init() {\n        x = this\n    }\n}\n");
        assert!(
            has(
                &d4,
                "'this' cannot be used as an expression in the finalizer"
            ),
            "{d4:?}"
        );
    }

    #[test]
    fn optional_param_in_abstract_func() {
        let diags = check("interface I {\n    func f(a!: Int32): Unit\n}\n");
        assert!(
            has(
                &diags,
                "optional parameter cannot be used in abstract function"
            ),
            "{diags:?}"
        );
        // anchored at the optional param `a`
        assert!(
            diags.iter().any(|d| d.line == 2 && d.col == 12),
            "{diags:?}"
        );
    }

    #[test]
    fn override_param_name_mismatch() {
        let diags = check(
            "open class A {\n    func test12(a: Int32): Int32 { 0 }\n}\n\
             class B <: A {\n    func test12(a: Int32): Int32 { 0 }\n}\n\
             class C <: A {\n    func test12(b: Int32): Int32 { 0 }\n}\n",
        );
        assert!(has(&diags, "parameter name mismatched"), "{diags:?}");
        // exactly one (only C mismatches)
        assert_eq!(
            diags
                .iter()
                .filter(|d| d.message.contains("parameter name mismatched"))
                .count(),
            1,
            "{diags:?}"
        );
    }

    #[test]
    fn bare_type_name_expression() {
        let diags = check("struct K {\n    K() {}\n}\nfunc f() {\n    K\n}\n");
        assert!(
            has(
                &diags,
                "expected member name or constructor call after 'K' type name"
            ),
            "{diags:?}"
        );
        // anchored at the bare `K` (line 5)
        assert!(diags.iter().any(|d| d.line == 5 && d.col == 5), "{diags:?}");
    }

    #[test]
    fn deprecated_reference() {
        let diags = check("@Deprecated\nlet PI = 3.14\nfunc f() {\n    var x = PI\n}\n");
        assert!(has(&diags, "variable 'PI' is deprecated"), "{diags:?}");
        // anchored at the `PI` reference in the func body
        assert!(
            diags.iter().any(|d| d.line == 4 && d.col == 13),
            "{diags:?}"
        );
    }
}
