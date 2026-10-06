// cj-lsp: textDocument/signatureHelp — show the parameter list of the
// function/ctor/method being called at the cursor.
//
// LSP SignatureHelp:
//   { signatures: [ { label, parameters: [ {label, documentation?} ],
//                    activeParameter?, documentation? } ],
//     activeSignature: 0, activeParameter: N }
//
// Strategy: find the enclosing call `callee(...)` by scanning back from the
// cursor to the innermost unmatched `(`; extract the callee word (plain name
// or `recv.member`); resolve it through the same Index used by hover
// (types/by_name/members/locals/std) to build the signature; count the
// top-level commas after the open paren to set activeParameter.

use cj_ast::{File, ImportSpec};
use serde_json::{json, Value};

use crate::hover::{Hoverable, Index};

struct ResolvedSignature {
    label: String,
    parameters: Vec<String>,
}

/// LSP signatureHelp at (line, character) — 0-based.
pub fn signature_help_at(
    file: &File,
    source: &str,
    line: u32,
    character: u32,
    sibling_docs: &[(&File, &str)],
    context: Option<&Value>,
) -> Value {
    let primary = Index::new(file, source, file.package.as_deref(), "");
    let sibling_indices: Vec<Index<'_>> = sibling_docs
        .iter()
        .map(|(sibling, sibling_source)| {
            Index::new(sibling, sibling_source, sibling.package.as_deref(), "")
        })
        .collect();
    // 1. Find the enclosing call parens.
    let Some((open_line, open_col, callee)) = enclosing_call(source, line, character) else {
        return Value::Null;
    };
    // 2. Count top-level commas between the open paren and the cursor
    //    (active parameter index). Label/param resolution below.
    let active_param = count_commas(source, open_line, open_col, line, character);

    // 3. Resolve the callee to all visible overloads.
    let mut signatures = resolve_signatures(
        file,
        source,
        &primary,
        &sibling_indices,
        &callee,
        line,
        character,
    );
    if signatures.is_empty() {
        return Value::Null;
    }
    signatures.sort_by_key(|s| s.parameters.len());

    let is_retrigger = context
        .and_then(|c| c.get("isRetrigger"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let retrigger_sig = if is_retrigger {
        context
            .and_then(|c| c.get("activeSignatureHelp"))
            .and_then(|h| h.get("activeSignature"))
            .and_then(Value::as_u64)
            .map(|v| v as u32)
    } else {
        None
    };

    let active_signature = if is_retrigger {
        retrigger_sig.unwrap_or(0)
    } else {
        0
    };

    let signature_objects: Vec<Value> = signatures
        .into_iter()
        .map(|signature| {
            if signature.parameters.is_empty() {
                json!({ "label": signature.label })
            } else {
                let parameters: Vec<Value> = signature
                    .parameters
                    .iter()
                    .map(|parameter| json!({ "label": parameter }))
                    .collect();
                json!({ "label": signature.label, "parameters": parameters })
            }
        })
        .collect();

    json!({
        "signatures": signature_objects,
        "activeSignature": active_signature,
        "activeParameter": active_param,
    })
}

/// Find the `(` that encloses the cursor and the callee word before it.
/// Returns (open_line, open_col, callee_display). `open_line/col` are 1-based
/// source columns as used by the source scan; the returned callee is the
/// text of the callee (e.g. `add` or `obj.method`).
fn enclosing_call(src: &str, line: u32, character: u32) -> Option<(u32, u32, String)> {
    let lines: Vec<&str> = src.split('\n').collect();
    let cur = lines.get(line as usize)?;
    // cursor byte-col (approximate: chars before it — CJ is mostly ASCII)
    let cur_col = cur[..(character as usize).min(cur.len())].chars().count();

    // Walk backwards from the cursor tracking paren depth; the first `(` at
    // depth 0 (relative to where we started, i.e. the innermost unmatched
    // one) is our call paren.
    let mut depth = 0i32;
    let mut l = line as i64;
    let mut c = cur_col as i64;
    loop {
        let text = lines.get(l as usize)?;
        let chars: Vec<char> = text.chars().collect();
        let mut ci = c - 1;
        while ci >= 0 {
            let ch = chars.get(ci as usize).copied()?;
            match ch {
                ')' => depth += 1,
                '(' if depth > 0 => depth -= 1,
                '(' => {
                    // Grouping/tuple parentheses have no callee. Keep scanning
                    // outward until we find the surrounding call expression.
                    if let Some(call) = callee_before(src, &lines, l as u32, ci as u32) {
                        return Some(call);
                    }
                }
                _ => {}
            }
            ci -= 1;
        }
        if l == 0 {
            return None;
        }
        l -= 1;
        let prev = lines.get(l as usize)?;
        c = prev.chars().count() as i64;
    }
}

/// Extract the callee word immediately before the open paren. Handles
/// `name(`, `recv.name(` and `name<T>(`. Returns 1-based (line, col) of the
/// open paren plus the callee display text.
fn callee_before(
    _src: &str,
    lines: &[&str],
    open_line: u32,
    open_col: u32,
) -> Option<(u32, u32, String)> {
    let line = lines.get(open_line as usize)?;
    let before = &line[..open_col as usize];
    let trimmed = before.trim_end();
    if trimmed.is_empty() {
        return None;
    }
    // `name<T>(` — skip one balanced trailing generic argument list.
    let mut base = trimmed;
    if base.ends_with('>') {
        let mut depth = 0u32;
        for (byte, ch) in base.char_indices().rev() {
            match ch {
                '>' => depth += 1,
                '<' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        base = base[..byte].trim_end();
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    let callee = extract_callee(base)?;
    Some((open_line, open_col, callee))
}

fn extract_callee(base: &str) -> Option<String> {
    let base = base.trim_end();
    if base.is_empty() {
        return None;
    }
    let name_end = base.len();
    let name_start = base
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map(|i| i + 1)
        .unwrap_or(0);
    let name = &base[name_start..name_end];
    if name.is_empty() {
        return None;
    }
    let before_name = base[..name_start].trim_end();
    if let Some(before_dot) = before_name.strip_suffix('.') {
        let before_dot = before_dot.trim_end();
        let recv = if before_dot.ends_with(')') {
            let mut depth = 0u32;
            let mut open_idx = None;
            for (byte, ch) in before_dot.char_indices().rev() {
                match ch {
                    ')' => depth += 1,
                    '(' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            open_idx = Some(byte);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(open) = open_idx {
                let callee_head = before_dot[..open].trim_end();
                let head_start = callee_head
                    .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
                    .map(|i| i + 1)
                    .unwrap_or(0);
                &before_dot[head_start..]
            } else {
                before_dot
            }
        } else if let Some(stripped) = before_dot.strip_suffix('"') {
            let quote_start = stripped.rfind('"').unwrap_or(0);
            &before_dot[quote_start..]
        } else {
            let recv_start = before_dot
                .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
                .map(|i| i + 1)
                .unwrap_or(0);
            &before_dot[recv_start..]
        };
        if !recv.is_empty() {
            return Some(format!("{recv}.{name}"));
        }
    }

    let prefix = base[..name_start].trim_end();
    if prefix.ends_with("func") || prefix.ends_with("macro") {
        return None;
    }
    Some(name.to_string())
}

/// Count top-level commas between the open paren and the cursor.
fn count_commas(src: &str, open_line: u32, open_col: u32, cur_line: u32, cur_char: u32) -> u32 {
    let lines: Vec<&str> = src.split('\n').collect();
    let mut depth = 0i32;
    let mut commas = 0u32;
    for (li, text) in lines.iter().enumerate() {
        let l = li as u32;
        if l < open_line {
            continue;
        }
        let start = if l == open_line {
            open_col.saturating_add(1) as usize
        } else {
            0
        };
        let end = if l == cur_line {
            (cur_char as usize).min(text.len())
        } else {
            text.len()
        };
        if start >= end && l != open_line {
            continue;
        }
        for ch in text[start..end].chars() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => commas += 1,
                _ => {}
            }
        }
        if l >= cur_line {
            break;
        }
    }
    commas
}

/// Resolve the callee to every visible callable overload.
fn resolve_signatures(
    file: &File,
    source: &str,
    primary: &Index,
    siblings: &[Index<'_>],
    callee: &str,
    line: u32,
    character: u32,
) -> Vec<ResolvedSignature> {
    let resolved_callee = resolve_import_alias(source, &file.imports, callee);
    let callee = resolved_callee.as_deref().unwrap_or(callee);

    let candidates: Vec<&Hoverable> = if let Some(dot) = callee.rfind('.') {
        let recv = &callee[..dot];
        let name = &callee[dot + 1..];
        let recv_ty = primary.receiver_type(recv, line, character).or_else(|| {
            if recv.ends_with(')') {
                if let Some(open) = recv.find('(') {
                    let callee_head = recv[..open].trim();
                    let head = callee_head.rsplit('.').next().unwrap_or(callee_head);
                    let head = head.split('<').next().unwrap_or(head).trim();
                    if siblings.iter().any(|idx| idx.types.contains_key(head)) {
                        return Some(head.to_string());
                    }
                }
            }
            siblings
                .iter()
                .find_map(|idx| idx.receiver_type(recv, line, character))
        });
        let Some(recv_ty) = recv_ty else {
            return Vec::new();
        };
        let mut values = primary.signature_member_candidates(&recv_ty, name);
        for index in siblings {
            values.extend(index.signature_member_candidates(&recv_ty, name));
        }
        values
    } else if primary.types.contains_key(callee)
        || siblings
            .iter()
            .any(|index| index.types.contains_key(callee))
    {
        // Signature help only exposes explicitly declared constructors. The
        // hover index's implicit `init()` is useful for navigation, but the
        // official signature-help behavior is null for a default constructor.
        let mut values = primary.signature_member_candidates(callee, "init");
        for index in siblings {
            values.extend(index.signature_member_candidates(callee, "init"));
        }
        values
    } else {
        let mut values = primary.signature_local_candidates(callee, line, character);
        if let Some(indices) = primary.by_name.get(callee) {
            values.extend(
                indices
                    .iter()
                    .map(|index| &primary.all[*index])
                    .filter(|candidate| !candidate.is_type),
            );
        }
        for index in siblings {
            if let Some(indices) = index.by_name.get(callee) {
                values.extend(
                    indices
                        .iter()
                        .map(|symbol| &index.all[*symbol])
                        .filter(|candidate| !candidate.is_type),
                );
            }
        }
        if values.is_empty() {
            if let Some(candidate) = primary.lookup_std(callee) {
                values.push(candidate);
            }
        }
        values
    };

    let mut signatures = Vec::new();
    for candidate in candidates {
        if let Some(signature) = signature_from_hover(candidate) {
            if !signatures
                .iter()
                .any(|existing: &ResolvedSignature| existing.label == signature.label)
            {
                signatures.push(signature);
            }
        }
    }
    signatures
}

fn resolve_import_alias(source: &str, _imports: &[ImportSpec], alias: &str) -> Option<String> {
    source.lines().find_map(|line| {
        let trimmed = line.trim();
        let import = trimmed.strip_prefix("import ")?;
        let (target, imported_alias) = import.split_once(" as ")?;
        if imported_alias.trim() == alias {
            let symbol = target.rsplit('.').next()?;
            Some(symbol.trim().to_string())
        } else {
            None
        }
    })
}

fn signature_from_hover(candidate: &Hoverable) -> Option<ResolvedSignature> {
    if let Some(function) = candidate.signature.find("func ") {
        let declaration = &candidate.signature[function + "func ".len()..];
        let open = declaration.find('(')?;
        let close = matching_paren(declaration, open)?;
        let parameters = split_parameters(&declaration[open + 1..close]);
        let mut label = format!("{}({})", candidate.name, parameters.join(", "));
        if candidate.name != "init" {
            let return_type = declaration[close + 1..]
                .trim()
                .strip_prefix(':')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("Unit");
            label.push_str(" -> ");
            label.push_str(return_type);
        }
        return Some(ResolvedSignature { label, parameters });
    }

    if candidate.signature.contains("// In enum ")
        || candidate
            .signature
            .contains(&format!(".{}(", candidate.name))
    {
        if let Some(open) = candidate.signature.find('(') {
            if let Some(close) = candidate.signature.rfind(')') {
                let raw_params = split_parameters(&candidate.signature[open + 1..close]);
                let parameters: Vec<String> = raw_params
                    .into_iter()
                    .enumerate()
                    .map(|(i, p)| {
                        if p.contains(':') {
                            p
                        } else {
                            format!("p{}: {}", i + 1, p)
                        }
                    })
                    .collect();
                let label = format!("{}({})", candidate.name, parameters.join(", "));
                return Some(ResolvedSignature { label, parameters });
            }
        }
    }

    if let Some(arrow) = candidate.signature.find("=>") {
        let before_arrow = &candidate.signature[..arrow];
        if let Some(open) = before_arrow.rfind('{').or_else(|| before_arrow.rfind('(')) {
            let raw_params = split_parameters(&candidate.signature[open + 1..arrow]);
            let parameters: Vec<String> =
                raw_params.into_iter().filter(|p| !p.is_empty()).collect();
            let label = format!("lambda({})", parameters.join(", "));
            return Some(ResolvedSignature { label, parameters });
        }
    }

    if !candidate.param_tys.is_empty() && candidate.ty.as_deref().is_some_and(|t| t.contains("->"))
    {
        let label = format!("lambda({})", candidate.param_tys.join(", "));
        return Some(ResolvedSignature {
            label,
            parameters: candidate.param_tys.clone(),
        });
    }

    None
}

fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0u32;
    for (offset, ch) in text[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_parameters(text: &str) -> Vec<String> {
    let mut parameters = Vec::new();
    let mut start = 0;
    let mut depth = 0u32;
    for (offset, ch) in text.char_indices() {
        match ch {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' | '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let parameter = text[start..offset].trim();
                if !parameter.is_empty() {
                    parameters.push(parameter.to_string());
                }
                start = offset + ch.len_utf8();
            }
            _ => {}
        }
    }
    let parameter = text[start..].trim();
    if !parameter.is_empty() {
        parameters.push(parameter.to_string());
    }
    parameters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_parameter_counts_only_call_arguments() {
        for (source, character, expected) in [
            ("var d4 = ff13(1.2,", 22, 1),
            ("var xu1 = CC(1,2", 16, 1),
            ("let res3 = f32(", 15, 0),
        ] {
            let (line, column, _) = enclosing_call(source, 0, character).unwrap();
            assert_eq!(count_commas(source, line, column, 0, character), expected);
        }
    }

    #[test]
    fn nested_grouping_still_resolves_outer_call() {
        let source = "f32(()";
        let (line, column, callee) = enclosing_call(source, 0, 6).unwrap();
        assert_eq!((line, column, callee.as_str()), (0, 3, "f32"));
        assert_eq!(count_commas(source, line, column, 0, 6), 0);
    }

    #[test]
    fn generic_callee_name_excludes_type_arguments() {
        let (_, _, callee) = enclosing_call("identity<Int64>(", 0, 16).unwrap();
        assert_eq!(callee, "identity");
    }
}
