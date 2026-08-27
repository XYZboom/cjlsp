// cj-lsp: textDocument/completion support.
//
// Collects candidate names visible at the request position (top-level decls
// of the file + same-package siblings when available) and returns LSP
// CompletionItems. Prefix-matches the token under the cursor.
//
// LSP CompletionItemKind (subset used here):
//   3  = Field, 4  = Variable, 5  = Class, 6  = Method, 7  = Function,
//   8  = Interface, 9  = Module, 11 = Struct, 12 = Constant, 13 = Enum,
//   15 = TypeParameter, 19 = EnumMember
//
// The official suite (completion_001) expects e.g.:
//   {"label":"Any","kind":8,"detail":"public interface Any","documentation":"",
//    "filterText":"Any","insertText":"Any","insertTextFormat":1,"sortText":"",
//    "deprecated":false}

use cj_ast::{Decl, File};
use serde_json::{json, Value};
use std::collections::HashSet;

/// A completion candidate: name + LSP kind + optional detail (decl signature).
struct Candidate {
    name: String,
    kind: u32,
    detail: String,
}

/// Core std symbols implicitly visible in every package (std.core).
/// Format mirrors official expectations: ("Any", kind 8 Interface,
/// "public interface Any").
const STD_CORE_SYMBOLS: &[(&str, u32, &str)] = &[
    ("Any", 8, "public interface Any"),
    ("AnyClass", 5, "public class AnyClass"),
    ("String", 5, "public struct String"),
    ("Int8", 11, "public struct Int8"),
    ("Int16", 11, "public struct Int16"),
    ("Int32", 11, "public struct Int32"),
    ("Int64", 11, "public struct Int64"),
    ("UInt8", 11, "public struct UInt8"),
    ("UInt16", 11, "public struct UInt16"),
    ("UInt32", 11, "public struct UInt32"),
    ("UInt64", 11, "public struct UInt64"),
    ("Float16", 11, "public struct Float16"),
    ("Float32", 11, "public struct Float32"),
    ("Float64", 11, "public struct Float64"),
    ("Bool", 11, "public struct Bool"),
    ("Unit", 11, "public struct Unit"),
    ("Nothing", 8, "public interface Nothing"),
    ("Nope", 11, "public struct Nope"),
    ("Tuple0", 11, "public struct Tuple0"),
    ("Tuple1", 11, "public struct Tuple1"),
    ("Tuple2", 11, "public struct Tuple2"),
    ("Tuple3", 11, "public struct Tuple3"),
    ("Tuple4", 11, "public struct Tuple4"),
    ("Tuple5", 11, "public struct Tuple5"),
    ("Rune", 11, "public struct Rune"),
];

/// Compute the prefix under the cursor: the longest trailing identifier
/// characters of the line up to `character` (0-based).
pub fn prefix_at_line(line_text: &str, character: u32) -> String {
    let mut prefix = String::new();
    let mut col = character as usize;
    if col > line_text.len() {
        col = line_text.len();
    }
    let before = &line_text[..col];
    for ch in before.chars().rev() {
        if ch.is_alphanumeric() || ch == '_' {
            prefix.insert(0, ch);
        } else {
            break;
        }
    }
    prefix
}

/// Collect completion candidates visible at `line`/`character` (1-based line,
/// 0-based character as LSP sends). Returns a JSON array of CompletionItems.
pub fn complete_at(
    file: &File,
    source: &str,
    line: u32,
    character: u32,
    sibling_decls: Option<&Vec<(String, u32, String)>>,
) -> Value {
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    #[inline]
    fn push(
        cands: &mut Vec<Candidate>,
        seen: &mut HashSet<String>,
        name: String,
        kind: u32,
        detail: String,
    ) {
        if seen.insert(name.clone()) {
            cands.push(Candidate { name, kind, detail });
        }
    }

    // 1. Top-level declarations of this file.
    for d in &file.decls {
        match d {
            Decl::Func {
                name, is_public, ..
            } => {
                let vis = if *is_public { "public " } else { "" };
                push(
                    &mut candidates,
                    &mut seen,
                    name.clone(),
                    7, // Function
                    format!("{vis}func {name}()"),
                );
            }
            Decl::Class {
                name, is_public, ..
            } => {
                let vis = if *is_public { "public " } else { "" };
                push(
                    &mut candidates,
                    &mut seen,
                    name.clone(),
                    5, // Class
                    format!("{vis}class {name}"),
                );
            }
            Decl::Interface {
                name, is_public, ..
            } => {
                let vis = if *is_public { "public " } else { "" };
                push(
                    &mut candidates,
                    &mut seen,
                    name.clone(),
                    8, // Interface
                    format!("{vis}interface {name}"),
                );
            }
            Decl::Struct {
                name, is_public, ..
            } => {
                let vis = if *is_public { "public " } else { "" };
                push(
                    &mut candidates,
                    &mut seen,
                    name.clone(),
                    11, // Struct
                    format!("{vis}struct {name}"),
                );
            }
            Decl::Enum {
                name, is_public, ..
            } => {
                let vis = if *is_public { "public " } else { "" };
                push(
                    &mut candidates,
                    &mut seen,
                    name.clone(),
                    13, // Enum
                    format!("{vis}enum {name}"),
                );
            }
            Decl::Var {
                name,
                is_public,
                ty,
                ..
            } => {
                let vis = if *is_public { "public " } else { "" };
                let ty_s = ty
                    .as_ref()
                    .map(|t| format!(": {}", display_type(t)))
                    .unwrap_or_default();
                push(
                    &mut candidates,
                    &mut seen,
                    name.clone(),
                    4, // Variable
                    format!("{vis}var {name}{ty_s}"),
                );
            }
            Decl::Macro {
                name, is_public, ..
            } => {
                let vis = if *is_public { "public " } else { "" };
                push(
                    &mut candidates,
                    &mut seen,
                    name.clone(),
                    13, // reuse: macro shown as keyword-ish (kind not critical)
                    format!("{vis}macro {name}"),
                );
            }
            _ => {}
        }
    }

    // 2. Same-package sibling declarations (from other files in the package).
    if let Some(sibs) = sibling_decls {
        for (name, kind, detail) in sibs {
            push(
                &mut candidates,
                &mut seen,
                name.clone(),
                *kind,
                detail.clone(),
            );
        }
    }

    // 3. Core std symbols (implicitly imported by every package — `Any` etc.).
    //    The official suite expects these: e.g. completion_001 returns
    //    {"label":"Any","kind":8,"detail":"public interface Any"}.
    for (name, kind, detail) in STD_CORE_SYMBOLS {
        push(
            &mut candidates,
            &mut seen,
            name.to_string(),
            *kind,
            (*detail).to_string(),
        );
    }

    // 3. Prefix filter. If the line has no identifier at the cursor (empty
    //    prefix), return everything (LSP clients filter further).
    let line_text = source_line_text(source, line);
    let prefix = prefix_at_line(&line_text, character);
    let mut items: Vec<Value> = candidates
        .iter()
        .filter(|c| prefix.is_empty() || c.name.starts_with(&prefix))
        .map(|c| {
            json!({
                "label": c.name.clone(),
                "kind": c.kind,
                "detail": c.detail.clone(),
                "documentation": "",
                "filterText": c.name.clone(),
                "insertText": c.name.clone(),
                "insertTextFormat": 1,
                "sortText": "",
                "deprecated": false,
            })
        })
        .collect();

    // 4. Official behaviour: when there is an exact match (filterText ==
    //    prefix), return ONLY exact matches. Otherwise return all prefix
    //    matches. This matches the diagnostics suite expectations.
    if !prefix.is_empty() {
        let exact: Vec<Value> = items
            .iter()
            .filter(|it| it.get("filterText").and_then(|v| v.as_str()) == Some(&prefix))
            .cloned()
            .collect();
        if !exact.is_empty() {
            items = exact;
        }
    }

    json!(items)
}

/// Text of a source line. `line` is the LSP 0-based line number.
fn source_line_text(source: &str, line: u32) -> String {
    source.lines().nth(line as usize).unwrap_or("").to_string()
}

/// Minimal type display for var details (avoids leaking internal names).
fn display_type(t: &cj_ast::Type) -> String {
    match t {
        cj_ast::Type::Ref { name, .. } | cj_ast::Type::Qualified { name, .. } => name.clone(),
        cj_ast::Type::Primitive { kind, .. } => format!("{kind:?}"),
        _ => "?".to_string(),
    }
}
