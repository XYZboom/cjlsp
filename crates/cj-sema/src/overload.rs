// cj-sema: function overload-conflict detection.
//
// Spec Ch.10 (函数重载): a scope may hold multiple functions with the same
// name as long as their parameter type sequences differ. Two top-level
// functions with the same name AND identical parameter types are a conflict:
//
//   function 'test05' has overload conflicts   (severity Error)
//
// reported at the LATER declaration's function name (official 017).

use cj_ast::{Decl, File, Param};
use cj_diag::{Diag, DiagId, Severity};

/// Format parameter type for readable candidate signature.
fn format_param_type(t: &cj_ast::Type) -> String {
    use cj_ast::Type;
    match t {
        Type::Ref { name, args, .. } => {
            if args.is_empty() {
                name.clone()
            } else {
                let inner: Vec<String> = args.iter().map(format_param_type).collect();
                format!("{}<{}>", name, inner.join(", "))
            }
        }
        Type::Qualified { name, .. } => name.clone(),
        Type::Option { inner, .. } => format!("{}?", format_param_type(inner)),
        Type::Constant { inner, .. } => format!("const {}", format_param_type(inner)),
        Type::VArray { inner, .. } => format!("VArray<{}>", format_param_type(inner)),
        Type::Primitive { kind, .. } => format!("{kind:?}"),
        Type::Paren { inner, .. } => format!("({})", format_param_type(inner)),
        Type::Func { params, ret, .. } => {
            let inner: Vec<String> = params.iter().map(format_param_type).collect();
            format!("({}) -> {}", inner.join(", "), format_param_type(ret))
        }
        Type::Tuple { elements, .. } => {
            let inner: Vec<String> = elements.iter().map(format_param_type).collect();
            format!("({})", inner.join(", "))
        }
        Type::This(_) => "This".to_string(),
        Type::Invalid(_) => "Invalid".to_string(),
    }
}

fn format_signature(name: &str, params: &[Param]) -> String {
    let p_strs: Vec<String> = params
        .iter()
        .map(|p| {
            let ty_str = format_param_type(&p.ty);
            if p.is_named {
                format!("{}!: {}", p.name, ty_str)
            } else {
                format!("{}: {}", p.name, ty_str)
            }
        })
        .collect();
    format!("func {}({})", name, p_strs.join(", "))
}

/// Detect overload conflicts among top-level functions in one file
/// (same package + same scope).
pub fn detect_overload_conflicts(file: &File) -> Vec<Diag> {
    let mut funcs: Vec<(&str, &[Param], u32, u32)> = Vec::new();
    for d in &file.decls {
        if let Decl::Func {
            name,
            name_pos,
            params,
            ..
        } = d
        {
            funcs.push((name, params, name_pos.line, name_pos.col));
        }
    }

    let mut diags = Vec::new();
    for (i, (name, params, line, col)) in funcs.iter().enumerate() {
        let matching: Vec<&[Param]> = funcs
            .iter()
            .filter(|(n2, p2, _, _)| n2 == name && same_signature(params, p2))
            .map(|(_, p2, _, _)| *p2)
            .collect();

        let conflict = funcs[..i]
            .iter()
            .any(|(n2, p2, _, _)| n2 == name && same_signature(params, p2));
        if conflict {
            let candidates: Vec<String> =
                matching.iter().map(|p| format_signature(name, p)).collect();

            let mut diag = Diag {
                code: None,
                category: None,
                severity: Severity::Error,
                message: format!("function '{name}' has overload conflicts"),
                line: *line,
                col: *col,
                end_line: *line,
                end_col: *col,
                here: None,
                notes: Vec::new(),
                tags: Vec::new(),
                fix: None,
                related_locations: Vec::new(),
                expected: None,
                actual: None,
                candidates,
                suggestions: Vec::new(),
            };
            diag = diag.with_id(DiagId::SEMA_OVERLOAD_CONFLICTS);
            diags.push(diag);
        }
    }
    diags
}

/// Two functions conflict when their parameter type sequences are identical.
/// Types are compared structurally (ignoring source positions).
fn same_signature(a: &[Param], b: &[Param]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| type_key(&x.ty) == type_key(&y.ty))
}

/// Canonical structural rendering of a type, ignoring `CodePos`.
fn type_key(t: &cj_ast::Type) -> String {
    use cj_ast::Type;
    match t {
        Type::Ref { name, args, .. } => {
            if args.is_empty() {
                format!("R({name})")
            } else {
                let inner: Vec<String> = args.iter().map(type_key).collect();
                format!("R({name}<{}>)", inner.join(","))
            }
        }
        Type::Qualified { name, .. } => format!("Q({name})"),
        Type::Option { inner, .. } => format!("O({})", type_key(inner)),
        Type::Constant { inner, .. } => format!("Co({})", type_key(inner)),
        Type::VArray { inner, .. } => format!("V({})", type_key(inner)),
        Type::Primitive { kind, .. } => format!("P({kind:?})"),
        Type::Paren { inner, .. } => format!("Pa({})", type_key(inner)),
        Type::Func { params, ret, .. } => {
            let inner: Vec<String> = params.iter().map(type_key).collect();
            format!("Fn({})->{}", inner.join(","), type_key(ret))
        }
        Type::Tuple { elements, .. } => {
            let inner: Vec<String> = elements.iter().map(type_key).collect();
            format!("T({})", inner.join(","))
        }
        Type::This(_) => "this".to_string(),
        Type::Invalid(_) => "invalid".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cj_parser::parse_source;

    #[test]
    fn identical_signatures_conflict() {
        let (file, _) =
            parse_source("func test05(a: Int8, b: Bool) {}\nfunc test05(a: Int8, b: Bool) {}\n");
        let diags = detect_overload_conflicts(&file);
        assert_eq!(diags.len(), 1);
        assert!(diags[0]
            .message
            .contains("function 'test05' has overload conflicts"));
        assert_eq!(diags[0].severity, Severity::Error);
        assert_eq!(diags[0].code, Some(DiagId::SEMA_OVERLOAD_CONFLICTS.code()));
        assert_eq!(diags[0].candidates.len(), 2);
        assert_eq!(diags[0].candidates[0], "func test05(a: Int8, b: Bool)");
        assert_eq!(diags[0].candidates[1], "func test05(a: Int8, b: Bool)");
        // reported at the SECOND declaration
        assert_eq!(diags[0].line, 2);
    }

    #[test]
    fn different_signatures_ok() {
        let (file, _) =
            parse_source("func f(a: Int8) {}\nfunc f(a: Int64) {}\nfunc f(a: Int8, b: Bool) {}\n");
        let diags = detect_overload_conflicts(&file);
        assert!(diags.is_empty(), "no conflict expected: {diags:?}");
    }
}
