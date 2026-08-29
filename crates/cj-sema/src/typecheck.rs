// cj-sema: minimal literal type checking for typed variable declarations.
//
// Per spec Ch.02 (Types):
//   * integer literals default to Int64; float literals to Float64
//   * Rune literals ('x') are Rune type; String literals ("...") are String
//   * each integer type has a value range (Int8: -128..127, ...)
//   * assigning a literal to a typed variable checks convertibility/range
//
// Diagnostic display names follow the official suite:
//   String/Rune literals report as 'Struct-String'
//   integer literals report as the declared type
//
// Messages (official wording):
//   cannot convert an integer literal to type 'Struct-String'
//   mismatched types expected 'Int8', found 'Struct-String'
//   the number '999999' exceeds the value range of type 'Int8'

use crate::resolver::is_known_builtin;
use crate::FuncSig;
use cj_ast::{Body, Decl, Expr, File, InterpPart, Type};
use cj_diag::Diag;
use std::collections::HashMap;

/// Literal type: what an expression's literal is (for display).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LitType {
    /// integer literal -> depends on range/declared type
    Int,
    /// character/Rune literal 'x'
    Char,
    /// string literal "x"
    Str,
    /// float literal
    Float,
    /// Bool
    Bool,
    /// unknown/untyped expression
    Unknown,
}

/// Check all top-level typed variable declarations with literal initializers.
pub fn check_decls(file: &File) -> Vec<Diag> {
    let mut diags = Vec::new();
    for d in &file.decls {
        if let Decl::Var {
            name,
            ty: Some(ty),
            init: Some(init),
            pos,
            ..
        } = d
        {
            check_typed_var(name, ty, init, *pos, &mut diags);
        }
    }
    diags
}

/// Check function-call arguments against the callee's formal parameter types.
///
/// `funcs` is the package-level signature table (name -> params), collected
/// from this file AND same-package sibling files, so calls to functions
/// defined in other files (e.g. `test06('x', 1)` where `test06` lives in
/// diag_006.cj) are still type-checked. Only bare-name calls are checked;
/// method/member calls are out of scope for this minimal pass.
///
/// Official messages (spec Ch.02 literal types):
///   mismatched types expected 'Int8', found 'Struct-String'
///   cannot convert an integer literal to type 'Struct-String'
///   the number '999999' exceeds the value range of type 'Int8'
///   missing arguments for parameter list '(Int64, Float64)' in call
pub fn check_calls(file: &File, funcs: &HashMap<String, FuncSig>) -> Vec<Diag> {
    let mut diags = Vec::new();
    for d in &file.decls {
        match d {
            Decl::Func {
                body: Body::Block(stmts),
                ..
            }
            | Decl::Main {
                body: Body::Block(stmts),
                pos: _,
            } => {
                for s in stmts {
                    walk_expr(s, funcs, &mut diags);
                }
            }
            _ => {}
        }
    }
    diags
}

/// Recursively walk an expression tree, checking every bare-name call.
fn walk_expr(e: &Expr, funcs: &HashMap<String, FuncSig>, diags: &mut Vec<Diag>) {
    match e {
        Expr::Call {
            callee, args, pos, ..
        } => {
            check_call(callee, args, *pos, funcs, diags);
            match callee.as_ref() {
                Expr::Name { .. } => {}
                other => walk_expr(other, funcs, diags),
            }
            for a in args {
                walk_expr(&a.value, funcs, diags);
            }
        }
        Expr::Paren { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::Member { object, .. } => walk_expr(object, funcs, diags),
        Expr::Optional { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::OptionalChain { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::Return { value: Some(v), .. } => walk_expr(v, funcs, diags),
        Expr::Unary { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, funcs, diags);
            walk_expr(rhs, funcs, diags);
        }
        Expr::IncOrDec { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::Subscript { object, index, .. } => {
            walk_expr(object, funcs, diags);
            walk_expr(index, funcs, diags);
        }
        Expr::Is { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::As { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::Range { start, end, .. } => {
            walk_expr(start, funcs, diags);
            walk_expr(end, funcs, diags);
        }
        Expr::ArrayLit { elements, .. }
        | Expr::Array { elements, .. }
        | Expr::Tuple { elements, .. } => {
            for el in elements {
                walk_expr(el, funcs, diags);
            }
        }
        Expr::Pointer { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::Match {
            scrutinee, cases, ..
        } => {
            walk_expr(scrutinee, funcs, diags);
            for c in cases {
                if let Some(g) = &c.guard {
                    walk_expr(g, funcs, diags);
                }
                walk_expr(&c.body, funcs, diags);
            }
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                walk_expr(s, funcs, diags);
            }
        }
        Expr::If {
            cond, then, els, ..
        } => {
            walk_expr(cond, funcs, diags);
            walk_expr(then, funcs, diags);
            if let Some(e) = els {
                walk_expr(e, funcs, diags);
            }
        }
        Expr::LetPatternDestructor { initializer, .. } => walk_expr(initializer, funcs, diags),
        Expr::Interpolation { parts, .. } | Expr::StrInterpolation { parts, .. } => {
            for p in parts {
                if let InterpPart::Expr(x) = p {
                    walk_expr(x, funcs, diags);
                }
            }
        }
        Expr::Quote { parts, .. } => {
            for p in parts {
                walk_expr(p, funcs, diags);
            }
        }
        Expr::Try {
            body,
            catches,
            finally,
            ..
        } => {
            walk_expr(body, funcs, diags);
            for c in catches {
                walk_expr(&c.body, funcs, diags);
            }
            if let Some(f) = finally {
                walk_expr(f, funcs, diags);
            }
        }
        Expr::While { cond, body, .. } | Expr::DoWhile { cond, body, .. } => {
            walk_expr(cond, funcs, diags);
            walk_expr(body, funcs, diags);
        }
        Expr::Lambda { body, .. } => walk_expr(body, funcs, diags),
        Expr::TrailingClosure { call, closure, .. } => {
            walk_expr(call, funcs, diags);
            walk_expr(closure, funcs, diags);
        }
        Expr::ForIn { iter, body, .. } => {
            walk_expr(iter, funcs, diags);
            walk_expr(body, funcs, diags);
        }
        Expr::TypeConv { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::Throw { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::Perform { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::Resume { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::Spawn { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::Synchronized { inner, .. } => walk_expr(inner, funcs, diags),
        Expr::IfAvailable { then, els, .. } => {
            walk_expr(then, funcs, diags);
            if let Some(e) = els {
                walk_expr(e, funcs, diags);
            }
        }
        _ => {}
    }
}

/// Check one bare-name call: arity + per-argument literal type compatibility.
fn check_call(
    callee: &Expr,
    args: &[cj_ast::FuncArg],
    call_pos: cj_ast::CodePos,
    funcs: &HashMap<String, FuncSig>,
    diags: &mut Vec<Diag>,
) {
    let name = match callee {
        Expr::Name { name, .. } => name,
        _ => return, // member/method calls are out of scope here
    };
    let Some(sig) = funcs.get(name) else {
        // Bare-name callee not found anywhere in the package (current file +
        // same-package siblings merged into `funcs`): genuinely undeclared —
        // report it, unless it's a known std builtin (print/println/Int64...).
        // Cross-file sibling functions ARE in `funcs`, so they never hit this.
        if !is_known_builtin(name) {
            if let Expr::Name { pos, .. } = callee {
                diags.push(Diag::error(
                    pos.line,
                    pos.col,
                    format!("undeclared identifier '{name}'"),
                ));
            }
        }
        return;
    };

    // Arity: too few arguments -> official "missing arguments for parameter
    // list '(T1, T2)' in call" reported at the call's '(' position.
    if args.len() < sig.params.len() {
        let list: Vec<String> = sig.params.iter().map(|p| type_display(&p.ty)).collect();
        diags.push(Diag::error(
            call_pos.line,
            call_pos.col,
            format!(
                "missing arguments for parameter list '({})' in call",
                list.join(", ")
            ),
        ));
    }

    // Named parameters (`b!: T`) must be passed with their name prefix;
    // a positional argument for a named parameter reports the prefix error.
    // The parser leaves FuncArg.pos unset, so anchor at the argument value.
    for (i, arg) in args.iter().enumerate() {
        let Some(psig) = sig.params.get(i) else {
            break;
        };
        if psig.is_named && arg.name.is_none() {
            let arg_pos = expr_pos(arg);
            diags.push(Diag::error(
                arg_pos.line,
                arg_pos.col,
                format!(
                    "missing argument prefix '{}:' for named parameter",
                    psig.name
                ),
            ));
        }
    }

    // Per-argument literal checks (official reports these at the literal).
    for (i, arg) in args.iter().enumerate() {
        let Some(pty) = sig.params.get(i) else {
            break; // too many arguments — not checked in this pass
        };
        let lit = match &arg.value {
            Expr::Lit { kind, value, pos } => Some((lit_type_of(kind, value), *pos)),
            _ => None,
        };
        let Some((lit_ty, lit_pos)) = lit else {
            continue;
        };
        let declared = type_display(&pty.ty);
        match (lit_ty, declared.as_str()) {
            // Rune/String literal into a non-string param: mismatched types.
            (LitType::Char | LitType::Str, d) if d != "Struct-String" => {
                diags.push(Diag::error(
                    lit_pos.line,
                    lit_pos.col,
                    format!("mismatched types expected '{d}', found 'Struct-String'"),
                ));
            }
            // Integer literal into a string param: cannot convert.
            (LitType::Int, "Struct-String") => {
                diags.push(Diag::error(
                    lit_pos.line,
                    lit_pos.col,
                    "cannot convert an integer literal to type 'Struct-String'",
                ));
            }
            // Integer literal into an integer param: value range check.
            (LitType::Int, d) if is_int_type(&declared) => {
                if let Expr::Lit { value, .. } = &arg.value {
                    if let Some(n) = parse_integer(value) {
                        if let Some((lo, hi)) = int_range(&declared) {
                            if n < lo || n > hi {
                                diags.push(Diag::error(
                                    lit_pos.line,
                                    lit_pos.col,
                                    format!(
                                        "the number '{value}' exceeds the value range of type '{d}'"
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
            // Integer literal into Bool/Rune/Float/other params: the official
            // suite reports nothing at this layer — leave for later phases.
            _ => {}
        }
    }
}

/// Position of a call argument (the parser leaves FuncArg.pos unset).
fn expr_pos(arg: &cj_ast::FuncArg) -> cj_ast::CodePos {
    match &arg.value {
        Expr::Lit { pos, .. } | Expr::Name { pos, .. } => *pos,
        _ => arg.pos,
    }
}

fn check_typed_var(
    name: &str,
    ty: &Type,
    init: &Expr,
    pos: cj_ast::CodePos,
    diags: &mut Vec<Diag>,
) {
    let _ = name;
    // Only literal initializers are checked in this minimal pass.
    let lit = match init {
        Expr::Lit { kind, value, pos } => Some((lit_type_of(kind, value), *pos)),
        _ => None,
    };
    let Some((lit_ty, lit_pos)) = lit else { return };

    let declared = type_display(ty);
    match (lit_ty, declared.as_str()) {
        // String target with int literal: cannot convert an integer literal.
        (LitType::Int, "Struct-String") => {
            diags.push(Diag::error(
                lit_pos.line,
                lit_pos.col,
                "cannot convert an integer literal to type 'Struct-String'",
            ));
        }
        // Char/Str literal into a non-string type: mismatched types.
        (LitType::Char | LitType::Str, d) if d != "Struct-String" => {
            diags.push(Diag::error(
                lit_pos.line,
                lit_pos.col,
                format!("mismatched types expected '{d}', found 'Struct-String'"),
            ));
        }
        // Int literal into fixed-width int type: range check.
        (LitType::Int, d) if is_int_type(&declared) => {
            if let Expr::Lit { value, .. } = init {
                if let Some(n) = parse_integer(value) {
                    if let Some((lo, hi)) = int_range(&declared) {
                        if n < lo || n > hi {
                            diags.push(Diag::error(
                                lit_pos.line,
                                lit_pos.col,
                                format!(
                                    "the number '{value}' exceeds the value range of type '{d}'"
                                ),
                            ));
                        }
                    }
                }
            }
        }
        // Int literal into a non-int non-string typed var is an error we
        // approximate as mismatched (official may say conversion).
        (LitType::Int, d) => {
            diags.push(Diag::error(
                lit_pos.line,
                lit_pos.col,
                format!("mismatched types expected '{d}', found 'Int64'"),
            ));
        }
        _ => {}
    }
    let _ = pos;
}

fn lit_type_of(kind: &cj_ast::LitKind, value: &str) -> LitType {
    match kind {
        cj_ast::LitKind::Integer => LitType::Int,
        cj_ast::LitKind::Rune | cj_ast::LitKind::RuneByte => LitType::Char,
        cj_ast::LitKind::String | cj_ast::LitKind::JString => LitType::Str,
        cj_ast::LitKind::Float => LitType::Float,
        cj_ast::LitKind::Bool => LitType::Bool,
        _ => {
            let _ = value;
            LitType::Unknown
        }
    }
}

fn parse_integer(s: &str) -> Option<i128> {
    let t = s.trim();
    let (neg, body) = if let Some(rest) = t.strip_prefix('-') {
        (true, rest)
    } else {
        (false, t)
    };
    let (radix, digits) =
        if let Some(rest) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            (16, rest)
        } else if let Some(rest) = body.strip_prefix("0b").or_else(|| body.strip_prefix("0B")) {
            (2, rest)
        } else if let Some(rest) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
            (8, rest)
        } else {
            (10, body)
        };
    let cleaned: String = digits.chars().filter(|c| *c != '_').collect();
    let v = i128::from_str_radix(&cleaned, radix).ok()?;
    Some(if neg { -v } else { v })
}

fn int_range(ty: &str) -> Option<(i128, i128)> {
    let r = match ty {
        "Int8" => (i8::MIN as i128, i8::MAX as i128),
        "Int16" => (i16::MIN as i128, i16::MAX as i128),
        "Int32" => (i32::MIN as i128, i32::MAX as i128),
        "Int64" | "Int" | "IntNative" => (i64::MIN as i128, i64::MAX as i128),
        "UInt8" | "Byte" => (0, u8::MAX as i128),
        "UInt16" => (0, u16::MAX as i128),
        "UInt32" => (0, u32::MAX as i128),
        "UInt64" | "UInt" | "UIntNative" => (0, u64::MAX as i128),
        _ => return None,
    };
    Some(r)
}

fn is_int_type(ty: &str) -> bool {
    matches!(
        ty,
        "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "Int"
            | "IntNative"
            | "UInt8"
            | "Byte"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "UInt"
            | "UIntNative"
    )
}

/// Display name of a declared type (official uses 'Struct-String' for String).
fn type_display(ty: &Type) -> String {
    match ty {
        Type::Ref { name, .. } | Type::Qualified { name, .. } => {
            if name == "String" || name == "Rune" {
                "Struct-String".to_string()
            } else {
                name.clone()
            }
        }
        Type::Primitive { kind, .. } => match kind {
            cj_ast::PrimitiveKind::String | cj_ast::PrimitiveKind::Rune => {
                "Struct-String".to_string()
            }
            _ => format!("{kind:?}"),
        },
        _ => format!("{ty:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cj_parser::parse_source;

    #[test]
    fn string_target_int_literal() {
        let (file, _) = parse_source("let var0041: String = 1\n");
        let diags = check_decls(&file);
        assert_eq!(diags.len(), 1);
        assert!(diags[0]
            .message
            .contains("cannot convert an integer literal"));
    }

    #[test]
    fn char_literal_mismatch() {
        let (file, _) = parse_source("let var0042: Int8 = 'x'\n");
        let diags = check_decls(&file);
        assert_eq!(diags.len(), 1);
        assert!(diags[0]
            .message
            .contains("mismatched types expected 'Int8', found 'Struct-String'"));
    }

    #[test]
    fn int_range_exceeded() {
        let (file, _) = parse_source("let var0043: Int8 = 999999\n");
        let diags = check_decls(&file);
        assert_eq!(diags.len(), 1);
        assert!(diags[0]
            .message
            .contains("exceeds the value range of type 'Int8'"));
    }

    #[test]
    fn valid_int_literal_ok() {
        let (file, _) = parse_source("let x: Int8 = 42\n");
        let diags = check_decls(&file);
        assert!(diags.is_empty(), "42 fits Int8: {:?}", diags);
    }

    fn sigs_of(src: &str) -> HashMap<String, crate::FuncSig> {
        let (file, _) = parse_source(src);
        let r = crate::Collector::new().collect_file(&file);
        r.func_sigs
    }

    #[test]
    fn call_rune_into_int8_mismatch() {
        let (file, _) = parse_source("func caller() { target('x', 1) }\n");
        let funcs = sigs_of("func target(a: Int8, b: Bool) {}\n");
        let diags = check_calls(&file, &funcs);
        assert!(
            diags.iter().any(|d| d
                .message
                .contains("mismatched types expected 'Int8', found 'Struct-String'")),
            "expected Int8/Struct-String mismatch: {:?}",
            diags
        );
        // integer literal 1 -> Bool is NOT reported (matches official 013).
        assert!(diags.len() == 1, "only the rune mismatch: {:?}", diags);
    }

    #[test]
    fn call_int_range_exceeded() {
        let (file, _) = parse_source("func caller() { target(999999) }\n");
        let funcs = sigs_of("func target(a: Int8) {}\n");
        let diags = check_calls(&file, &funcs);
        assert!(diags
            .iter()
            .any(|d| d.message.contains("exceeds the value range of type 'Int8'")));
    }

    #[test]
    fn call_missing_arguments() {
        let (file, _) = parse_source("func caller() { target() }\n");
        let funcs = sigs_of("func target(a: Int64, b: Float64) {}\n");
        let diags = check_calls(&file, &funcs);
        assert!(diags.iter().any(|d| d
            .message
            .contains("missing arguments for parameter list '(Int64, Float64)' in call")));
    }

    #[test]
    fn named_param_without_prefix() {
        // `b!` is a named parameter: a positional arg must carry the prefix.
        let (file, _) = parse_source("func caller() { target(1, 2) }\n");
        let funcs = sigs_of("func target(a: Int32, b!: Int32) {}\n");
        let diags = check_calls(&file, &funcs);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("missing argument prefix 'b:'")),
            "{diags:?}"
        );
        // anchored at the offending argument (the literal 2, line 1 col 27)
        assert!(
            diags.iter().any(|d| d.line == 1 && d.col == 27),
            "{diags:?}"
        );
        // a named arg with a prefix is fine
        let (file2, _) = parse_source("func caller() { target(1, b: 2) }\n");
        let diags2 = check_calls(&file2, &funcs);
        assert!(
            !diags2
                .iter()
                .any(|d| d.message.contains("missing argument prefix")),
            "{diags2:?}"
        );
    }

    #[test]
    fn call_unknown_function_reported() {
        let (file, _) = parse_source("func caller() { ghost(1) }\n");
        let funcs = sigs_of("func target(a: Int8) {}\n");
        let diags = check_calls(&file, &funcs);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("undeclared identifier 'ghost'")),
            "unknown bare-name callee must be reported as undeclared: {diags:?}"
        );
    }

    #[test]
    fn call_known_builtin_not_undeclared() {
        // print/println are std builtins — calling them must NOT report
        // `undeclared identifier` even though they are not in the funcs table.
        let (file, _) = parse_source("func caller() { println(1) }\n");
        let funcs = sigs_of("func target(a: Int8) {}\n");
        let diags = check_calls(&file, &funcs);
        assert!(
            !diags
                .iter()
                .any(|d| d.message.contains("undeclared identifier 'println'")),
            "{diags:?}"
        );
    }
}
