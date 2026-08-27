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

use cj_ast::{Decl, Expr, File, Type};
use cj_diag::Diag;

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
}
