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
    /// Call-site position of the @Macro(...) invocation (1-based).
    pub call_line: u32,
    pub call_col: u32,
    /// End of the macro call span (1-based; exclusive column = one past the
    /// closing `)`). Diagnostics at or before this position on the call line
    /// are treated as inside the expanded-text region — code that came from
    /// the expansion.
    pub call_end_line: u32,
    pub call_end_col: u32,
    /// Expanded program text (or a preview string for builtins).
    pub expanded: String,
}

impl Expansion {
    /// Whether a 1-based `(line, col)` position falls inside this expansion's
    /// source span (the macro call text the expanded code replaces).
    pub fn contains(&self, line: u32, col: u32) -> bool {
        let start = (self.call_line, self.call_col);
        let end = (self.call_end_line, self.call_end_col);
        (line, col) >= start && (line, col) <= end
    }
}

/// Shared expansion context (avoids 8-arg function signatures).
struct ExpandCtx<'a> {
    file_name: &'a str,
    /// Macro definitions collected from this file (used by the quote-template
    /// fallback when the compiled .so path is unavailable or fails).
    defs: &'a HashMap<String, MacroDef>,
    cache: &'a mut crate::macro_cache::MacroCache,
    pkg_dir: Option<&'a std::path::Path>,
    expansions: &'a mut Vec<Expansion>,
    diags: &'a mut Vec<Diag>,
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
            call_end_line: pos.end_line,
            call_end_col: pos.end_col,
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

/// High-level entry: expand every @Macro in a file and attach expansion
/// previews to diagnostics.
///
/// Returns (expansions, diagnostics). Diagnostics for a macro call that fails
/// carry a note with the expansion preview — the "报错带完整宏展开预览"
/// requirement. Uses the macro cache for compiled .so when available (fast
/// path), falling back to builtin/quote-template expansion.
pub fn expand_file_with_cache(
    file: &File,
    file_name: &str,
    cache: &mut crate::macro_cache::MacroCache,
    pkg_dir: Option<&std::path::Path>,
) -> (Vec<Expansion>, Vec<Diag>) {
    let mut expansions = Vec::new();
    let mut diags = Vec::new();
    let defs = collect_macros(file);
    let mut ctx = ExpandCtx {
        file_name,
        defs: &defs,
        cache,
        pkg_dir,
        expansions: &mut expansions,
        diags: &mut diags,
    };
    for d in &file.decls {
        expand_decl_with_cache(d, &mut ctx);
    }
    (expansions, diags)
}

fn expand_decl_with_cache(d: &Decl, ctx: &mut ExpandCtx) {
    match d {
        Decl::MacroExpand { name, args, pos } => {
            expand_one_cached(name, args, pos, ctx);
        }
        Decl::Var {
            init: Some(init), ..
        } => {
            expand_expr_cached(init, ctx);
        }
        Decl::Func {
            body: Body::Block(stmts),
            ..
        }
        | Decl::Macro {
            body: Body::Block(stmts),
            ..
        } => {
            for s in stmts {
                expand_expr_cached(s, ctx);
            }
        }
        _ => {}
    }
}

fn expand_expr_cached(e: &Expr, ctx: &mut ExpandCtx) {
    match e {
        Expr::MacroExpand { name, args, pos } => {
            expand_one_cached(name, args, pos, ctx);
        }
        Expr::Call { callee, args, .. } => {
            expand_expr_cached(callee, ctx);
            for a in args {
                expand_expr_cached(&a.value, ctx);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            expand_expr_cached(lhs, ctx);
            expand_expr_cached(rhs, ctx);
        }
        Expr::Unary { inner, .. } => {
            expand_expr_cached(inner, ctx);
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                expand_expr_cached(s, ctx);
            }
        }
        _ => {}
    }
}

/// Expand one macro call: builtin first, then cached .so (compile if needed),
/// then quote-template fallback. Diagnostics carry the expansion preview.
fn expand_one_cached(name: &str, args: &[cj_ast::Tokenish], pos: &CodePos, ctx: &mut ExpandCtx) {
    // 1. Builtin macros (always fast, no cache).
    let (builtin_out, builtin_diag) = expand_builtin_macro(name, args, pos, ctx.file_name);
    if builtin_diag.is_none() {
        ctx.expansions.push(Expansion {
            call_line: pos.line,
            call_col: pos.col,
            call_end_line: pos.end_line,
            call_end_col: pos.end_col,
            expanded: builtin_out,
        });
        return;
    }

    // 2. Macro-package compile cache: if a package dir is available, compile
    //    the macro package (cached by source hash) and dlopen the resulting
    //    .so to actually run `macroCall_c_*` (real expansion, spec Ch.14).
    //    On any failure (no SDK, dlopen/dlsym miss, call error) we fall back
    //    gracefully — template expansion if a def is available, else the
    //    previous placeholder text — so the LSP never destabilizes.
    if let Some(dir) = ctx.pkg_dir {
        if let Ok((so, pkg_name)) = ctx.cache.compile_macro_package(dir) {
            let key = crate::macro_cache::MacroCache::expansion_key(name, &args_text(args));
            let expanded =
                ctx.cache.expand_cached(&key, || {
                    match crate::dylib::expand_macro_call(&so, name, &pkg_name, args) {
                        Ok(tokens) => crate::dylib::tokens_to_text(&tokens),
                        Err(e) => fallback_expansion(name, args, ctx.defs, &so, &e),
                    }
                });
            ctx.expansions.push(Expansion {
                call_line: pos.line,
                call_col: pos.col,
                call_end_line: pos.end_line,
                call_end_col: pos.end_col,
                expanded,
            });
            return;
        }
    }

    // 3. In-file quote-template fallback: a macro defined in this file can be
    //    expanded without the SDK (spec Ch.14 quote substitution). Record the
    //    expansion so diagnostics within the call span can show the preview.
    if let Some(def) = ctx.defs.get(name) {
        let (out, tdiags) = expand_user_macro(def, args);
        ctx.expansions.push(Expansion {
            call_line: pos.line,
            call_col: pos.col,
            call_end_line: pos.end_line,
            call_end_col: pos.end_col,
            expanded: out,
        });
        ctx.diags.extend(tdiags);
        return;
    }

    // 4. Unknown macro: report unresolved (official wording).
    ctx.diags.push(Diag::error(
        pos.line,
        pos.col,
        format!("unresolved macro '{name}'"),
    ));
}

/// Graceful fallback when the compiled .so cannot be invoked: template
/// expansion via the macro's quote body (the pre-T11 behavior), else a
/// placeholder noting the .so + failure. Never returns an unresolved diag —
/// the caller has already decided this macro has a definition.
fn fallback_expansion(
    name: &str,
    args: &[cj_ast::Tokenish],
    defs: &HashMap<String, MacroDef>,
    so: &std::path::Path,
    err: &str,
) -> String {
    if let Some(def) = defs.get(name) {
        let (out, _) = expand_user_macro(def, args);
        if !out.is_empty() {
            return out;
        }
    }
    format!(
        "<macro {name} compiled to {} (dlopen/expand failed: {err})>",
        so.display()
    )
}

fn args_text(args: &[cj_ast::Tokenish]) -> String {
    args.iter()
        .map(|a| a.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
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

    #[test]
    fn local_def_template_fallback_records_span() {
        // A macro defined in the file expands without the SDK (quote-template
        // fallback): the expansion trace must carry the full call span so the
        // LSP can attach the "code after the macro is expanded" note to any
        // diagnostic inside `@Wrap(42)`.
        let (file, _) = parse_source(
            "public macro Wrap(x: Tokens): Tokens { quote(print(x)) }\nlet y = @Wrap(42)\n",
        );
        let mut cache = crate::macro_cache::MacroCache::new();
        let (exps, diags) = expand_file_with_cache(&file, "t.cj", &mut cache, None);
        assert!(diags.is_empty(), "diags: {diags:?}");
        assert_eq!(exps.len(), 1, "exps: {exps:?}");
        let e = &exps[0];
        // `let y = @Wrap(42)` on line 2: `@` at col 9, `)` at col 17.
        assert_eq!((e.call_line, e.call_col), (2, 9));
        assert_eq!((e.call_end_line, e.call_end_col), (2, 18));
        assert!(e.contains(2, 9), "@ itself is inside the span");
        assert!(e.contains(2, 17), "closing paren is inside the span");
        assert!(!e.contains(2, 8), "before the call is outside");
        assert!(!e.contains(2, 19), "after the call is outside");
        assert_eq!(e.expanded, "print ( 42");
    }

    #[test]
    fn expansion_span_covers_diagnostic_position() {
        // A diag at/inside the call (e.g. on the closing paren, or on the
        // token right after it — end_col is the column one past `)`) is
        // attributed to the expansion; further right is not.
        let (file, _) = parse_source(
            "public macro Wrap(x: Tokens): Tokens { quote(print(x)) }\nlet y = @Wrap(42)\n",
        );
        let mut cache = crate::macro_cache::MacroCache::new();
        let (exps, _) = expand_file_with_cache(&file, "t.cj", &mut cache, None);
        assert!(exps[0].contains(2, 9)); // `@`
        assert!(exps[0].contains(2, 17)); // `)`
        assert!(exps[0].contains(2, 18)); // inclusive end (right after `)`)
        assert!(!exps[0].contains(2, 19)); // one past the call
        assert!(!exps[0].contains(2, 8)); // before the call
    }
}
