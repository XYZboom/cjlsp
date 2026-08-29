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

use cj_ast::File;
use serde_json::{json, Value};

use crate::hover::Index;

/// LSP signatureHelp at (line, character) — 0-based.
pub fn signature_help_at(file: &File, source: &str, line: u32, character: u32) -> Value {
    let idx = Index::new(file, source, None, "");
    // 1. Find the enclosing call parens.
    let Some((open_line, open_col, callee)) = enclosing_call(source, line, character) else {
        return Value::Null;
    };
    // 2. Count top-level commas between the open paren and the cursor
    //    (active parameter index). Label/param resolution below.
    let active_param = count_commas(source, open_line, open_col, line, character);

    // 3. Resolve the callee to a signature.
    let sig = resolve_signature(&idx, &callee, line, character);
    let Some((label, params)) = sig else {
        return Value::Null;
    };

    // 4. Build SignatureHelp.
    let param_objs: Vec<Value> = params.iter().map(|p| json!({ "label": p })).collect();
    json!({
        "signatures": [{
            "label": label,
            "parameters": param_objs,
        }],
        "activeSignature": 0,
        "activeParameter": active_param.min(params.len().saturating_sub(1) as u32),
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
                    // found the enclosing call open paren at (l, ci)
                    return callee_before(src, &lines, l as u32, ci as u32);
                }
                _ => {}
            }
            ci -= 1;
        }
        if l == 0 {
            return None;
        }
        l -= 1;
        c = text.chars().count() as i64;
    }
}

/// Extract the callee word immediately before the open paren. Handles
/// `name(`, `recv.name(` and `name<T>(`. Returns 1-based (line, col) of the
/// open paren plus the callee display text.
fn callee_before(
    src: &str,
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
    // `name<T>(` — strip the type args for the display but keep the base
    let base = trimmed.trim_end_matches('>');
    let word = base
        .rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .next()?;
    if word.is_empty() {
        return None;
    }
    let _ = src;
    Some((open_line, open_col, word.to_string()))
}

/// Count top-level commas between the open paren and the cursor.
fn count_commas(src: &str, open_line: u32, open_col: u32, cur_line: u32, cur_char: u32) -> u32 {
    let lines: Vec<&str> = src.split('\n').collect();
    let mut depth = 0i32;
    let mut commas = 0u32;
    for (li, text) in lines.iter().enumerate() {
        let l = li as u32;
        let start = if l == open_line { open_col as usize } else { 0 };
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

/// Resolve the callee to (label, param_labels). Reuses the hover Index for
/// type/member/local/std resolution.
fn resolve_signature(
    idx: &Index,
    callee: &str,
    line: u32,
    character: u32,
) -> Option<(String, Vec<String>)> {
    // member access `recv.name`
    if let Some(dot) = callee.rfind('.') {
        let recv = &callee[..dot];
        let name = &callee[dot + 1..];
        let recv_ty = idx.receiver_type(recv, line, character)?;
        let hi = idx.member_lookup(&recv_ty, name)?;
        return Some((hi.signature.clone(), hi.param_tys.clone()));
    }
    // type ctor `Foo(...)` → implicit/default init
    if idx.types.contains_key(callee) {
        if let Some(hi) = idx.implicit_inits.get(callee) {
            return Some((hi.signature.clone(), hi.param_tys.clone()));
        }
        // class without declared init: show `Foo()` — no params
        return Some((format!("{callee}()"), Vec::new()));
    }
    // local value / top-level func
    if let Some(hi) = idx.lookup_local(callee, line, character).or_else(|| {
        idx.by_name
            .get(callee)
            .and_then(|hits| hits.iter().find(|&&i| !idx.all[i].is_type))
            .map(|&i| &idx.all[i])
    }) {
        return Some((hi.signature.clone(), hi.param_tys.clone()));
    }
    // stdlib symbol (STD_SYMS has sig only for types; funcs fall through)
    if let Some(hi) = idx.lookup_std(callee) {
        return Some((hi.signature.clone(), Vec::new()));
    }
    None
}
