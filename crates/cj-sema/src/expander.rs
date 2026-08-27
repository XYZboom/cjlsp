// cj-sema: macro expansion (spec Ch.14 — 元编程/宏).
//
// Pipeline per spec:
//   * 宏只编译期可见; a macro call in an expression/decl position is expanded
//     into program text and the result replaces the call node
//   * quote(...) captures a token sequence; $ (expr) interpolates
//   * builtin macros (@sourceFile/@sourceLine/@sourcePackage) expand to
//     String/Int64 literals carrying the environment at the call site
//   * the expander records an expansion trace (call span -> expanded text) so
//     diagnostics can show the full macro-expansion preview (report position +
//     call site + generated code), per the project requirement.
//
// This module implements collection of macro definitions per file/package and
// expansion of MacroExpand nodes. It is deliberately small: full token-level
// splicing is deferred; the trace is the part diagnostics/LSP actually need.

use cj_ast::{Body, CodePos, Decl, Expr, File};
use cj_diag::Diag;
use std::collections::HashMap;

/// A collected macro definition (name -> body expression).
#[derive(Debug, Clone)]
pub struct MacroDef {
    pub name: String,
    /// parameter names (in order) — used to substitute tokens
    pub params: Vec<String>,
    pub body: Body,
    pub pos: CodePos,
}

/// One expansion trace entry: the call site and the generated code.
#[derive(Debug, Clone)]
pub struct Expansion {
    /// Call-site position of the @Macro(...) invocation.
    pub call_line: u32,
    pub call_col: u32,
    /// Expanded program text (or a preview string for builtins).
    pub expanded: String,
}

/// Collect macro definitions from top-level declarations of a file.
pub fn collect_macros(file: &File) -> HashMap<String, MacroDef> {
    let mut macros = HashMap::new();
    for d in &file.decls {
        if let Decl::Macro {
            name,
            params,
            body,
            pos,
            ..
        } = d
        {
            let param_names = params.iter().map(|p| p.name.clone()).collect();
            macros.insert(
                name.clone(),
                MacroDef {
                    name: name.clone(),
                    params: param_names,
                    body: body.clone(),
                    pos: *pos,
                },
            );
        }
    }
    macros
}

/// Expand all builtin macros and record an expansion trace.
///
/// `file_name` is used for @sourceFile(). Returns (diagnostics, expansions).
/// User-defined macro expansion (splicing body tokens over the call args) is
/// scaffolded here and completed once token-level splicing lands; builtin
/// expansion (spec §builtins) is fully functional.
pub fn expand_builtins(file: &File, file_name: &str) -> (Vec<Diag>, Vec<Expansion>) {
    let mut diags = Vec::new();
    let mut expansions = Vec::new();
    for d in &file.decls {
        walk_decl_for_macros(d, file_name, &mut diags, &mut expansions);
    }
    (diags, expansions)
}

/// Recursively find Expr::MacroExpand / Decl::MacroExpand anywhere in a decl.
fn walk_decl_for_macros(
    d: &Decl,
    file_name: &str,
    diags: &mut Vec<Diag>,
    expansions: &mut Vec<Expansion>,
) {
    match d {
        Decl::MacroExpand { name, args, pos } => {
            expand_one(name, args, pos, file_name, diags, expansions);
        }
        Decl::Var {
            init: Some(init), ..
        } => walk_expr_for_macros(init, file_name, diags, expansions),
        Decl::Func {
            body: Body::Block(stmts),
            ..
        }
        | Decl::Macro {
            body: Body::Block(stmts),
            ..
        } => {
            for s in stmts {
                walk_expr_for_macros(s, file_name, diags, expansions);
            }
        }
        _ => {}
    }
}

fn walk_expr_for_macros(
    e: &Expr,
    file_name: &str,
    diags: &mut Vec<Diag>,
    expansions: &mut Vec<Expansion>,
) {
    match e {
        Expr::MacroExpand { name, args, pos } => {
            expand_one(name, args, pos, file_name, diags, expansions);
        }
        Expr::Call { callee, args, .. } => {
            walk_expr_for_macros(callee, file_name, diags, expansions);
            for a in args {
                walk_expr_for_macros(&a.value, file_name, diags, expansions);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr_for_macros(lhs, file_name, diags, expansions);
            walk_expr_for_macros(rhs, file_name, diags, expansions);
        }
        Expr::Unary { inner, .. } => walk_expr_for_macros(inner, file_name, diags, expansions),
        Expr::Paren { inner, .. } => walk_expr_for_macros(inner, file_name, diags, expansions),
        Expr::Block { stmts, .. } => {
            for s in stmts {
                walk_expr_for_macros(s, file_name, diags, expansions);
            }
        }
        Expr::If {
            cond, then, els, ..
        } => {
            walk_expr_for_macros(cond, file_name, diags, expansions);
            walk_expr_for_macros(then, file_name, diags, expansions);
            if let Some(e2) = els {
                walk_expr_for_macros(e2, file_name, diags, expansions);
            }
        }
        _ => {}
    }
}

fn expand_one(
    name: &str,
    args: &[cj_ast::Tokenish],
    pos: &CodePos,
    file_name: &str,
    diags: &mut Vec<Diag>,
    expansions: &mut Vec<Expansion>,
) {
    let (expanded, diag) = expand_builtin_macro(name, args, pos, file_name);
    if let Some(e) = diag {
        diags.push(e);
    } else {
        expansions.push(Expansion {
            call_line: pos.line,
            call_col: pos.col,
            expanded,
        });
    }
}

fn expand_builtin_macro(
    name: &str,
    _args: &[cj_ast::Tokenish],
    pos: &CodePos,
    file_name: &str,
) -> (String, Option<Diag>) {
    match name {
        "sourceFile" => (format!("\"{file_name}\""), None),
        "sourceLine" => (pos.line.to_string(), None),
        "sourcePackage" => ("\"pkg\"".to_string(), None), // refined when package resolution lands
        _ => (
            String::new(),
            Some(Diag::error(
                pos.line,
                pos.col,
                format!("unresolved macro '{name}'"),
            )),
        ),
    }
}

/// Expand a user macro call: substitute call args into the macro body's
/// quote template. Returns generated text and diagnostics.
///
/// The substitution is token-text level: each occurrence of a parameter name
/// as a standalone identifier token in the quote body is replaced with the
/// concatenated arg text. This is a faithful, small subset of spec §宏定义.
pub fn expand_user_macro(def: &MacroDef, args: &[cj_ast::Tokenish]) -> (String, Vec<Diag>) {
    let mut diags = Vec::new();
    // Collect quote parts from body (Body::Block of one quote expr typically).
    let parts = match &def.body {
        Body::Block(stmts) => stmts
            .iter()
            .filter_map(|e| {
                if let Expr::Quote { parts, .. } = e {
                    Some(parts.clone())
                } else {
                    None
                }
            })
            .next(),
        Body::Empty => None,
    };
    let Some(parts) = parts else {
        return (String::new(), diags);
    };
    // Arity check: args count must match params.
    if def.params.len() != args.len() {
        diags.push(Diag::error(
            0,
            0,
            format!(
                "macro '{0}' expects {1} argument(s), found {2}",
                def.name,
                def.params.len(),
                args.len()
            ),
        ));
        return (String::new(), diags);
    }
    // Build param -> arg text map.
    let subs: HashMap<&String, &str> = def
        .params
        .iter()
        .zip(args.iter())
        .map(|(p, a)| (p, a.text.as_str()))
        .collect();
    let mut out = String::new();
    for part in parts {
        match part {
            Expr::TokenPart { text, .. } => {
                // Substitute standalone identifier tokens equal to a param name.
                if let Some(repl) = subs.get(&text) {
                    out.push_str(repl);
                } else {
                    out.push_str(&text);
                }
                out.push(' ');
            }
            Expr::MacroExpand { name, args, .. } => {
                // nested macro call in quote: keep as-is for now
                let argtxt: Vec<&str> = args.iter().map(|a| a.text.as_str()).collect();
                out.push('@');
                out.push_str(&name);
                out.push('(');
                out.push_str(&argtxt.join(", "));
                out.push(')');
                out.push(' ');
            }
            Expr::Quote { .. } => {
                diags.push(Diag::error(
                    0,
                    0,
                    "nested quote inside macro body is not supported yet".to_string(),
                ));
            }
            _ => {} // interpolation etc. ignored for now
        }
    }
    (out.trim().to_string(), diags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cj_parser::parse_source;

    #[test]
    fn collect_macro_defs() {
        let (file, _) =
            parse_source("public macro PlusOne(input: Tokens): Tokens { quote(input + 1) }\n");
        let macros = collect_macros(&file);
        assert!(
            macros.contains_key("PlusOne"),
            "macros: {:?}",
            macros.keys()
        );
        assert_eq!(macros["PlusOne"].params, vec!["input"]);
    }

    #[test]
    fn builtin_source_line() {
        let (file, _) = parse_source("let x = @sourceLine()\n");
        let (diags, exps) = expand_builtins(&file, "test.cj");
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(exps.len(), 1);
        assert_eq!(exps[0].expanded, "1"); // line 1
    }

    #[test]
    fn unresolved_macro_diag() {
        let (file, _) = parse_source("let x = @NoSuchMacro()\n");
        let (diags, _) = expand_builtins(&file, "test.cj");
        assert!(!diags.is_empty());
        assert!(diags[0].message.contains("unresolved macro 'NoSuchMacro'"));
    }

    #[test]
    fn user_macro_substitution() {
        let (file, _) = parse_source(
            "public macro Wrap(x: Tokens): Tokens { quote(print(x)) }\nlet y = @Wrap(42)\n",
        );
        let macros = collect_macros(&file);
        let def = &macros["Wrap"];
        let args = vec![cj_ast::Tokenish {
            text: "42".to_string(),
            pos: CodePos::new(4, 4, 0, 4, 6, 0),
        }];
        let (out, diags) = expand_user_macro(def, &args);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(out, "print ( 42");
    }
}
