// cj-lsp: workspace/symbol (Ctrl+T project-wide symbol search).
//
// Mirrors the official engine's WorkspaceSymbol semantics, verified against
// the cangjie_tools workspaceSymbol message tests:
//   - traverse every project file's `file.decls` (reusing the sibling scan
//     cache) and emit top-level + nested symbols
//   - `name` is the SIGNATURE (funcs: `name(params)`, enum cases:
//     `Enum.Case(...)`), but query filtering matches the raw IDENTIFIER
//   - `kind` follows the official ASTKind->SymbolKind map
//     (Class=5, Property=7, Enum=10, Interface=11, Function=12,
//      Variable=13, Object=19 (type alias), Struct=23)
//   - `containerName` is the scope chain: `pkg`, `pkg.Type`, `pkg.extend`
//   - query is a case-insensitive fuzzy subsequence (IsMatchingCompletion)

use cj_ast::{CodePos, Decl, File, TypeParam};
use serde_json::{json, Value};
use std::path::PathBuf;

// LSP SymbolKind (official ASTKind->SymbolKind map).
const KIND_CLASS: u32 = 5;
const KIND_PROPERTY: u32 = 7;
const KIND_ENUM: u32 = 10;
const KIND_INTERFACE: u32 = 11;
const KIND_FUNCTION: u32 = 12;
const KIND_VARIABLE: u32 = 13;
const KIND_OBJECT: u32 = 19;
const KIND_STRUCT: u32 = 23;

/// Collect workspace symbols from the given project files.
///
/// `files` is a list of (absolute path, parsed file, source text) — exactly
/// what `collect_sibling_docs_with_path` returns. Each top-level and nested
/// declaration becomes a WorkspaceSymbol filtered by `query`.
pub fn collect_workspace_symbols(files: &[(PathBuf, &File, &str)], query: &str) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for (path, file, source) in files {
        let uri = format!("file://{}", path.display());
        let pkg = file
            .package
            .clone()
            .unwrap_or_else(|| "default".to_string());
        collect_decls(&file.decls, &pkg, &uri, query, source, &mut out);
    }
    // Deterministic order: location (uri, line, char) then kind/name/container
    // (mirrors the official SymbolInformation operator<).
    out.sort_by(|a, b| {
        let au = a["location"]["uri"].as_str().unwrap_or("");
        let bu = b["location"]["uri"].as_str().unwrap_or("");
        au.cmp(bu).then_with(|| {
            let al = a["location"]["range"]["start"]["line"]
                .as_u64()
                .unwrap_or(0);
            let bl = b["location"]["range"]["start"]["line"]
                .as_u64()
                .unwrap_or(0);
            al.cmp(&bl).then_with(|| {
                let ac = a["location"]["range"]["start"]["character"]
                    .as_u64()
                    .unwrap_or(0);
                let bc = b["location"]["range"]["start"]["character"]
                    .as_u64()
                    .unwrap_or(0);
                ac.cmp(&bc)
                    .then_with(|| {
                        a["kind"]
                            .as_u64()
                            .unwrap_or(0)
                            .cmp(&b["kind"].as_u64().unwrap_or(0))
                    })
                    .then_with(|| {
                        a["name"]
                            .as_str()
                            .unwrap_or("")
                            .cmp(b["name"].as_str().unwrap_or(""))
                    })
                    .then_with(|| {
                        a["containerName"]
                            .as_str()
                            .unwrap_or("")
                            .cmp(b["containerName"].as_str().unwrap_or(""))
                    })
            })
        })
    });
    json!(out)
}

/// Walk a list of declarations, emitting symbols. `container` is the current
/// scope chain (e.g. "default", "default.testEnum", "testPkg.extend").
fn collect_decls(
    decls: &[Decl],
    container: &str,
    uri: &str,
    query: &str,
    source: &str,
    out: &mut Vec<Value>,
) {
    for d in decls {
        match d {
            Decl::Func {
                name,
                name_pos,
                params,
                type_params,
                ..
            } => {
                let sig = func_signature(name, params, type_params);
                push(
                    out,
                    query,
                    name,
                    &sig,
                    KIND_FUNCTION,
                    *name_pos,
                    uri,
                    container,
                );
            }
            Decl::Macro { name, pos, .. } => {
                // Official emits the macro identifier only (no parens).
                let np = name_pos_after_keyword(source, *pos, "macro", name);
                push(out, query, name, name, KIND_FUNCTION, np, uri, container);
            }
            Decl::Main { pos, .. } => {
                // bare `main()` — signature is identifier + "()".
                push(
                    out,
                    query,
                    "main",
                    "main()",
                    KIND_FUNCTION,
                    *pos,
                    uri,
                    container,
                );
            }
            Decl::Class {
                name,
                name_pos,
                members,
                type_params,
                ..
            } => {
                let dn = type_name(name, type_params);
                push(out, query, name, &dn, KIND_CLASS, *name_pos, uri, container);
                let sub = format!("{container}.{name}");
                collect_decls(members, &sub, uri, query, source, out);
            }
            Decl::Struct {
                name,
                name_pos,
                members,
                type_params,
                ..
            } => {
                let dn = type_name(name, type_params);
                push(
                    out,
                    query,
                    name,
                    &dn,
                    KIND_STRUCT,
                    *name_pos,
                    uri,
                    container,
                );
                let sub = format!("{container}.{name}");
                collect_decls(members, &sub, uri, query, source, out);
            }
            Decl::Interface {
                name,
                name_pos,
                members,
                type_params,
                ..
            } => {
                let dn = type_name(name, type_params);
                push(
                    out,
                    query,
                    name,
                    &dn,
                    KIND_INTERFACE,
                    *name_pos,
                    uri,
                    container,
                );
                let sub = format!("{container}.{name}");
                collect_decls(members, &sub, uri, query, source, out);
            }
            Decl::Enum {
                name,
                name_pos,
                cases,
                ..
            } => {
                push(out, query, name, name, KIND_ENUM, *name_pos, uri, container);
                let enum_container = format!("{container}.{name}");
                for case in cases {
                    // Payload-less case -> Variable(13); payload -> Function(12)
                    // with the payload types in the signature.
                    let (case_name, kind) = if case.payloads.is_empty() {
                        (format!("{name}.{}", case.name), KIND_VARIABLE)
                    } else {
                        let types: Vec<String> = case
                            .payloads
                            .iter()
                            .map(crate::completion::display_type)
                            .collect();
                        (
                            format!("{name}.{}({})", case.name, types.join(", ")),
                            KIND_FUNCTION,
                        )
                    };
                    push(
                        out,
                        query,
                        &case.name,
                        &case_name,
                        kind,
                        case.pos,
                        uri,
                        &enum_container,
                    );
                }
            }
            Decl::Extend { members, .. } => {
                // ExtendDecl has no identifier; its members live under
                // `<scope>.extend` (matches official `default.extend`).
                let sub = format!("{container}.extend");
                collect_decls(members, &sub, uri, query, source, out);
            }
            Decl::TypeAlias {
                name,
                pos,
                type_params,
                ..
            } => {
                let np = name_pos_after_keyword(source, *pos, "type", name);
                let dn = type_name(name, type_params);
                push(out, query, name, &dn, KIND_OBJECT, np, uri, container);
            }
            Decl::Var { name, name_pos, .. } => {
                push(
                    out,
                    query,
                    name,
                    name,
                    KIND_VARIABLE,
                    *name_pos,
                    uri,
                    container,
                );
            }
            Decl::Prop { name, pos, .. } => {
                let np = name_pos_after_keyword(source, *pos, "prop", name);
                push(out, query, name, name, KIND_PROPERTY, np, uri, container);
            }
            // PrimaryCtor (init), Builtin, Package, FuncParam, VarWithPattern,
            // GenericParam, MacroExpand, Invalid are not workspace symbols.
            _ => {}
        }
    }
}

/// Emit one WorkspaceSymbol if `identifier` matches `query` (case-insensitive
/// fuzzy subsequence on the raw identifier, per the official engine).
#[allow(clippy::too_many_arguments)]
fn push(
    out: &mut Vec<Value>,
    query: &str,
    identifier: &str,
    display_name: &str,
    kind: u32,
    pos: CodePos,
    uri: &str,
    container: &str,
) {
    if !fuzzy_match(identifier, query) {
        return;
    }
    out.push(json!({
        "name": display_name,
        "kind": kind,
        "containerName": container,
        "location": {
            "uri": uri,
            "range": {
                "start": {"line": pos.line - 1, "character": pos.col - 1},
                "end": {"line": pos.end_line - 1, "character": pos.end_col - 1}
            }
        }
    }));
}

/// Official function signature: `name<T>(a: Int64, b: Int64)`.
fn func_signature(name: &str, params: &[cj_ast::Param], type_params: &[TypeParam]) -> String {
    let mut s = name.to_string();
    if !type_params.is_empty() {
        let tps: Vec<&str> = type_params.iter().map(|t| t.name.as_str()).collect();
        s.push('<');
        s.push_str(&tps.join(", "));
        s.push('>');
    }
    s.push('(');
    let ps: Vec<String> = params
        .iter()
        .map(|p| {
            let ty = crate::completion::display_type(&p.ty);
            if p.is_named {
                format!("{}!: {ty}", p.name)
            } else {
                format!("{}: {ty}", p.name)
            }
        })
        .collect();
    s.push_str(&ps.join(", "));
    s.push(')');
    s
}

/// Type name with generic params: `Foo<T, K>`.
fn type_name(name: &str, type_params: &[TypeParam]) -> String {
    if type_params.is_empty() {
        name.to_string()
    } else {
        let tps: Vec<&str> = type_params.iter().map(|t| t.name.as_str()).collect();
        format!("{name}<{}>", tps.join(", "))
    }
}

/// Case-insensitive fuzzy subsequence match (official IsMatchingCompletion).
fn fuzzy_match(name: &str, query: &str) -> bool {
    let n: Vec<char> = name.chars().collect();
    let q: Vec<char> = query.chars().collect();
    if q.is_empty() {
        return true;
    }
    let mut ni = 0;
    for qc in &q {
        let mut found = false;
        while ni < n.len() {
            if n[ni].eq_ignore_ascii_case(qc) {
                found = true;
                ni += 1;
                break;
            }
            ni += 1;
        }
        if !found {
            return false;
        }
    }
    true
}

/// For decls where the AST stores the KEYWORD position (`type`/`prop`/`macro`)
/// but not a name_pos, compute the identifier's span from the source: skip the
/// keyword, then whitespace, then the name token follows.
fn name_pos_after_keyword(source: &str, kw: CodePos, keyword: &str, name: &str) -> CodePos {
    let mut off = kw.offset + keyword.len();
    let bytes = source.as_bytes();
    while off < bytes.len() && (bytes[off] as char).is_whitespace() {
        off += 1;
    }
    let (line, col) = offset_to_line_col(source, off);
    let end_off = (off + name.len()).min(source.len());
    let (end_line, end_col) = offset_to_line_col(source, end_off);
    CodePos {
        line,
        col,
        offset: off,
        end_line,
        end_col,
        end_offset: end_off,
    }
}

/// Byte offset -> (1-based line, 1-based column).
fn offset_to_line_col(source: &str, off: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut col = 1u32;
    for (i, b) in source.bytes().enumerate() {
        if i >= off {
            break;
        }
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
