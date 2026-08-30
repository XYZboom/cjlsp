// cj-lsp: textDocument/codeLens support.
//
// Shows "N references" above every top-level declaration (function / class /
// struct / interface / enum / type alias / top-level var). The count reuses
// the references collection (references.rs): it equals the number of
// locations textDocument/references would return for that declaration
// (declaration itself + every Name-expression usage in the file), so the
// lens count and the references panel always agree.
//
// Performance: the file is walked ONCE — every Name expression is grouped by
// name into a hash map, and every top-level decl name position is recorded in
// the same pass. Each lens then looks up its count and its full location list
// from those maps, so the whole request is O(file) regardless of how many
// top-level decls exist (no per-decl full-file rescan). Only top-level decls
// get lenses; nested members are not recursed into.

use cj_ast::{Body, Decl, Expr, File};
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::references::decl_name_pos;

/// LSP position triple (0-based line, start col, end col) for a 1-based span.
type Ls = (u32, u32, u32);

fn lsp_span(p: &cj_ast::CodePos) -> Ls {
    (
        p.line.saturating_sub(1),
        p.col.saturating_sub(1),
        p.end_col.saturating_sub(1),
    )
}

/// Generate CodeLens entries for `file` (one per top-level declaration).
/// `uri` is used in the showReferences command arguments so clicking a lens
/// opens the references panel for that declaration.
pub fn code_lenses(file: &File, uri: &str) -> Value {
    // Single pass: group every Name-expression position by name (a name used
    // twice -> two entries) AND record every top-level decl name position, in
    // the same walk. Keyed by the raw identifier, exactly like references.rs
    // matches (name equality, no scope resolution).
    let mut decl_positions: HashMap<String, Vec<Ls>> = HashMap::new();
    let mut usage_positions: HashMap<String, Vec<Ls>> = HashMap::new();
    for d in &file.decls {
        if let Some((name, npos)) = decl_name_pos(d) {
            decl_positions
                .entry(name)
                .or_default()
                .push(lsp_span(&npos));
        }
        collect_decl_all_names(d, &mut usage_positions);
    }

    // Merge each name's decl + usage locations once, in source order, so every
    // lens for that name (incl. overloads) shares the same location list.
    let mut locations: HashMap<String, Vec<Ls>> = HashMap::new();
    for (name, dpos) in &decl_positions {
        let mut locs = dpos.clone();
        if let Some(u) = usage_positions.get(name) {
            locs.extend(u.iter().copied());
        }
        locs.sort_unstable();
        locations.insert(name.clone(), locs);
    }

    let mut lenses: Vec<Value> = Vec::new();
    for d in &file.decls {
        // Only top-level decls with a name get a lens (Main/Builtin/etc. are
        // skipped — decl_name_pos returns None for them).
        let Some((name, npos)) = decl_name_pos(d) else {
            continue;
        };
        let decl_n = decl_positions.get(&name).map_or(0, Vec::len) as u32;
        let usage_n = usage_positions.get(&name).map_or(0, Vec::len) as u32;
        let total = decl_n + usage_n;

        let (l0, c0, e0) = lsp_span(&npos);
        // Range = the declaration name's span (0-based). Lenses attach to the
        // name token itself so VS Code renders "N references" above the line.
        let range = json!({
            "start": {"line": l0, "character": c0},
            "end": {"line": l0, "character": e0},
        });

        let locs: Vec<Value> = locations
            .get(&name)
            .map(|v| {
                v.iter()
                    .map(|&(l, c, e)| {
                        json!({
                            "uri": uri,
                            "range": {
                                "start": {"line": l, "character": c},
                                "end": {"line": l, "character": e},
                            }
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        lenses.push(json!({
            "range": range,
            "command": {
                "title": format!("{total} references"),
                "command": "editor.action.showReferences",
                "arguments": [
                    uri,
                    {"line": l0, "character": c0},
                    locs,
                ],
            }
        }));
    }
    json!(lenses)
}

/// Walk a declaration's body, grouping every Name expression by name
/// (mirrors references::expr_name_refs traversal, but collects ALL names in
/// one pass instead of filtering on a single target).
fn collect_decl_all_names(d: &Decl, usages: &mut HashMap<String, Vec<Ls>>) {
    match d {
        Decl::Var {
            init: Some(init), ..
        } => {
            collect_all_names(init, usages);
        }
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
                collect_all_names(s, usages);
            }
        }
        _ => {}
    }
}

/// Group Name expressions in an expression tree by name.
fn collect_all_names(e: &Expr, usages: &mut HashMap<String, Vec<Ls>>) {
    match e {
        Expr::Name { name, pos, .. } => {
            usages.entry(name.clone()).or_default().push(lsp_span(pos));
        }
        Expr::Call { callee, args, .. } => {
            collect_all_names(callee, usages);
            for a in args {
                collect_all_names(&a.value, usages);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_all_names(lhs, usages);
            collect_all_names(rhs, usages);
        }
        Expr::Unary { inner, .. } => collect_all_names(inner, usages),
        Expr::Paren { inner, .. } => collect_all_names(inner, usages),
        Expr::Subscript { object, index, .. } => {
            collect_all_names(object, usages);
            collect_all_names(index, usages);
        }
        Expr::Member { object, .. } => collect_all_names(object, usages),
        Expr::Block { stmts, .. } => {
            for s in stmts {
                collect_all_names(s, usages);
            }
        }
        Expr::If {
            cond, then, els, ..
        } => {
            collect_all_names(cond, usages);
            collect_all_names(then, usages);
            if let Some(e2) = els {
                collect_all_names(e2, usages);
            }
        }
        Expr::LetPatternDestructor { initializer, .. } => {
            collect_all_names(initializer, usages);
        }
        Expr::Return { value: Some(v), .. } => collect_all_names(v, usages),
        Expr::Assign { lhs, rhs, .. } => {
            collect_all_names(lhs, usages);
            collect_all_names(rhs, usages);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> cj_ast::File {
        let source = src.to_string();
        cj_parser::Parser::new(&source, cj_lexer::Lexer::new(&source).tokenize()).run()
    }

    /// Extract (anchor, count) pairs from a codeLens result, in decl order.
    fn lens_counts(result: Value) -> Vec<(String, u32)> {
        result
            .as_array()
            .expect("array")
            .iter()
            .map(|l| {
                let title = l["command"]["title"].as_str().unwrap_or("");
                let count = title
                    .split_whitespace()
                    .next()
                    .and_then(|n| n.parse::<u32>().ok())
                    .unwrap_or(0);
                let start = &l["range"]["start"];
                (
                    format!(
                        "line{}:col{}",
                        start["line"].as_u64().unwrap(),
                        start["character"].as_u64().unwrap()
                    ),
                    count,
                )
            })
            .collect()
    }

    #[test]
    fn counts_function_usages_and_decl() {
        // foo declared once, called twice -> 3 references (decl + 2 calls).
        // bar declared once, never used -> 1 reference (its decl).
        let file = parse(
            "func foo(): Unit {}\n\
             func bar(): Unit {}\n\
             func main(): Unit {\n\
                 foo()\n\
                 foo()\n\
             }\n",
        );
        let result = code_lenses(&file, "file:///proj/main.cj");
        let counts = lens_counts(result.clone());
        assert_eq!(counts.len(), 3, "one lens per top-level func: {:?}", counts);

        // Lenses are anchored on each decl's name line (0-based): foo=0, bar=1, main=2.
        let foo = &counts[0];
        let bar = &counts[1];
        let main = &counts[2];
        assert_eq!(foo.0, "line0:col5", "foo name at 0-based line0 col5");
        assert_eq!(foo.1, 3, "foo: decl + 2 calls = 3 references");
        assert_eq!(bar.0, "line1:col5", "bar name at line1");
        assert_eq!(bar.1, 1, "bar: decl only = 1 reference");
        assert_eq!(main.0, "line2:col5", "main name at line2");
        assert_eq!(main.1, 1, "main: decl only = 1 reference");

        // The command is editor.action.showReferences (clickable lens).
        assert_eq!(
            result[0]["command"]["command"].as_str().unwrap(),
            "editor.action.showReferences"
        );
        assert_eq!(
            result[0]["command"]["arguments"][0].as_str().unwrap(),
            "file:///proj/main.cj"
        );
    }

    #[test]
    fn counts_class_usage_in_bodies() {
        // Point() is a ctor call whose callee is a Name expr -> 1 decl + 1 use
        // = 2 references. movePoint lives inside the class (not top-level) so
        // it gets NO lens of its own; its name is not a bare top-level ref.
        let file = parse(
            "class Point {\n\
             \x20   func movePoint(): Unit {}\n\
             }\n\
             func useIt(): Unit {\n\
                 Point()\n\
             }\n",
        );
        let result = code_lenses(&file, "file:///proj/a.cj");
        let counts = lens_counts(result);
        // Top-level decls with names: Point (line0) + useIt (line3).
        assert_eq!(
            counts.len(),
            2,
            "only top-level decls get lenses: {:?}",
            counts
        );
        assert_eq!(counts[0].0, "line0:col6");
        assert_eq!(counts[0].1, 2, "Point: decl + `Point()` ctor call = 2");
        assert_eq!(counts[1].0, "line3:col5");
        assert_eq!(counts[1].1, 1, "useIt decl only");
    }

    #[test]
    fn main_only_file_gets_one_lens() {
        let file = parse("func main(): Unit { print(\"hi\") }\n");
        let result = code_lenses(&file, "file:///proj/m.cj");
        let counts = lens_counts(result);
        // main IS a named top-level func decl -> gets a lens. print is a
        // different name and stdlib-only; main: 1 reference (decl only).
        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0].1, 1);
    }

    #[test]
    fn overloaded_names_share_reference_total() {
        // Two `f` decls + one call site -> each lens shows decl_count(2)+usage(1)=3.
        let file = parse(
            "func f(a: Int64): Unit {}\n\
             func f(a: String): Unit {}\n\
             func main(): Unit { f(1) }\n",
        );
        let result = code_lenses(&file, "file:///proj/o.cj");
        let counts = lens_counts(result);
        assert_eq!(counts.len(), 3);
        assert_eq!(counts[0].0, "line0:col5");
        assert_eq!(counts[0].1, 3, "f: 2 decls + 1 usage = 3");
        assert_eq!(counts[1].0, "line1:col5");
        assert_eq!(counts[1].1, 3, "f overload shares the same total");
    }

    #[test]
    fn empty_file_yields_empty_lenses() {
        let file = parse("");
        let result = code_lenses(&file, "file:///proj/e.cj");
        assert_eq!(result.as_array().unwrap().len(), 0);
    }

    #[test]
    fn lens_locations_match_references_order() {
        // Decl + usages must appear in source order in the showReferences
        // arguments, mirroring references_at output. NOTE: use explicit \n +
        // real spaces — Rust string continuation strips leading whitespace.
        let file = parse(
            "func foo(): Unit {}\n\
             func main(): Unit {\n    foo()\n    bar()\n    foo()\n}\n\
             func bar(): Unit {}\n",
        );
        let result = code_lenses(&file, "file:///proj/r.cj");
        let locs = result[0]["command"]["arguments"][2].as_array().unwrap();
        let spans: Vec<(u64, u64)> = locs
            .iter()
            .map(|l| {
                let s = &l["range"]["start"];
                (
                    s["line"].as_u64().unwrap(),
                    s["character"].as_u64().unwrap(),
                )
            })
            .collect();
        // foo decl at 0:5, then foo() at 2:4, then foo() at 4:4 -> sorted.
        assert_eq!(
            spans,
            vec![(0, 5), (2, 4), (4, 4)],
            "locations in source order: {:?}",
            spans
        );
    }
}
