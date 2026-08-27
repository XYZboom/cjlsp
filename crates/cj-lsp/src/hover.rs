// cj-lsp: textDocument/hover support.
//
// Given a cursor position, find the declaration whose name span contains it
// and return LSP Hover markdown:
//   "Declared in: <file>  \nPackage info: <pkg>  \n\n```cangjie\n<decl>\n```"
// plus the range of the identifier under the cursor.
//
// Official suite (hover_001) expects e.g. for `I3` at a1.cj:17:11:
//   contents.kind = "markdown"
//   contents.value = "Declared in: a1.cj  \nPackage info: default.Any  \n\n```cangjie\ninternal interface I3 extends Any\n```\n"
//   range = {start:{line:17,character:10}, end:{line:17,character:12}}

use cj_ast::{CodePos, Decl, File};
use serde_json::{json, Value};

/// Find the declaration whose name span contains (line, character) — LSP
/// 0-based. Returns (detail line, range start, range end) if found.
pub fn hover_at(
    file: &File,
    package: Option<&str>,
    file_name: &str,
    line: u32,
    character: u32,
) -> Value {
    for d in &file.decls {
        if let Some((detail, start, end)) = decl_hover(d, line, character) {
            return json!({
                "contents": {
                    "kind": "markdown",
                    "value": format!(
                        "Declared in: {file_name}  \nPackage info: {}  \n\n```cangjie\n{detail}\n```\n",
                        package.unwrap_or("")
                    ),
                },
                "range": {
                    "start": {"line": start.line, "character": start.col},
                    "end": {"line": end.line, "character": end.col},
                }
            });
        }
    }
    json!(Value::Null)
}

/// Find the declaration at the cursor and return it as an LSP Location
/// (uri + name span). Reuses decl_hover's span matching.
pub fn definition_at(file: &File, uri: &str, line: u32, character: u32) -> Value {
    for d in &file.decls {
        if let Some((_, start, end)) = decl_hover(d, line, character) {
            return json!({
                "uri": uri,
                "range": {
                    "start": {"line": start.line, "character": start.col},
                    "end": {"line": end.line, "character": end.col},
                }
            });
        }
    }
    json!(Value::Null)
}

/// If the declaration's name span contains the cursor, return the hover detail
/// and the name's byte-range (LSP 0-based).
fn decl_hover(d: &Decl, line: u32, character: u32) -> Option<(String, CodePos, CodePos)> {
    // name_pos spans the identifier (1-based line/col from the lexer).
    let (name, name_pos, detail): (&str, &CodePos, String) = match d {
        Decl::Func {
            name,
            name_pos,
            params,
            ret,
            ..
        } => {
            let param_types: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
            let ret_s = ret
                .as_ref()
                .map(super::completion::display_type)
                .unwrap_or_else(|| "Unit".to_string());
            (
                name,
                name_pos,
                format!(
                    "func {name}({}){}",
                    param_types.join(", "),
                    if ret_s == "Unit" {
                        String::new()
                    } else {
                        format!(": {ret_s}")
                    }
                ),
            )
        }
        Decl::Class {
            name,
            name_pos,
            is_public,
            is_abstract,
            is_open,
            is_sealed,
            parents,
            ..
        } => {
            let mut sig = String::new();
            if *is_public {
                sig.push_str("public ");
            }
            if *is_abstract {
                sig.push_str("abstract ");
            }
            if *is_open {
                sig.push_str("open ");
            }
            if *is_sealed {
                sig.push_str("sealed ");
            }
            sig.push_str("class ");
            sig.push_str(name);
            if !parents.is_empty() {
                sig.push_str(" extends ");
                sig.push_str(&format_parents(parents));
            }
            (name, name_pos, sig)
        }
        Decl::Interface {
            name,
            name_pos,
            is_public,
            parents,
            ..
        } => {
            let mut sig = String::new();
            if *is_public {
                sig.push_str("public ");
            } else {
                sig.push_str("internal ");
            }
            sig.push_str("interface ");
            sig.push_str(name);
            if !parents.is_empty() {
                sig.push_str(" extends ");
                sig.push_str(&format_parents(parents));
            }
            (name, name_pos, sig)
        }
        Decl::Struct {
            name,
            name_pos,
            is_public,
            is_open,
            ..
        } => {
            let mut sig = String::new();
            if *is_public {
                sig.push_str("public ");
            }
            if *is_open {
                sig.push_str("open ");
            }
            sig.push_str("struct ");
            sig.push_str(name);
            (name, name_pos, sig)
        }
        Decl::Enum {
            name,
            name_pos,
            is_public,
            ..
        } => {
            let mut sig = String::new();
            if *is_public {
                sig.push_str("public ");
            }
            sig.push_str("enum ");
            sig.push_str(name);
            (name, name_pos, sig)
        }
        Decl::Var {
            name,
            name_pos,
            is_public,
            ty,
            ..
        } => {
            let mut sig = String::new();
            if *is_public {
                sig.push_str("public ");
            }
            sig.push_str("let ");
            sig.push_str(name);
            if let Some(t) = ty {
                sig.push_str(": ");
                sig.push_str(&super::completion::display_type(t));
            }
            (name, name_pos, sig)
        }
        Decl::Macro {
            name,
            is_public,
            pos,
            ..
        } => {
            let mut sig = String::new();
            if *is_public {
                sig.push_str("public ");
            }
            sig.push_str("macro ");
            sig.push_str(name);
            (name, pos, sig)
        }
        _ => return None,
    };

    // name_pos is 1-based (lexer columns); convert to 0-based for comparison.
    let name_line_0 = name_pos.line.saturating_sub(1);
    let name_start_col_0 = name_pos.col.saturating_sub(1);
    // LSP end is exclusive: 1-based end_col -> 0-based exclusive end.
    let name_end_col_0 = name_pos.end_col.saturating_sub(1);

    if line == name_line_0 && character >= name_start_col_0 && character < name_end_col_0 {
        let _ = name;
        // Return 0-based LSP positions (subtract 1 from 1-based lexer cols).
        Some((
            detail,
            CodePos {
                line: name_pos.line.saturating_sub(1),
                col: name_start_col_0,
                offset: 0,
                end_line: name_pos.end_line.saturating_sub(1),
                end_col: name_pos.end_col.saturating_sub(1),
                end_offset: 0,
            },
            CodePos {
                line: name_pos.line.saturating_sub(1),
                col: name_pos.end_col.saturating_sub(1),
                offset: 0,
                end_line: name_pos.line.saturating_sub(1),
                end_col: name_pos.end_col.saturating_sub(1),
                end_offset: 0,
            },
        ))
    } else {
        None
    }
}

/// Format parent type names: ["Any"] -> "Any"; ["A", "B"] -> "A, B".
fn format_parents(parents: &[cj_ast::Type]) -> String {
    parents
        .iter()
        .map(super::completion::display_type)
        .collect::<Vec<_>>()
        .join(", ")
}
