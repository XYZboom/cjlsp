// cj-lsp: textDocument/rename + textDocument/prepareRename support.
//
// Symbol renaming in the CURRENT file only. Reuses documentHighlight's
// occurrence collection (type_or_expr_name_at + collect_occurrences) to get
// every position of the symbol under the cursor (declaration + usages +
// type positions), then returns a WorkspaceEdit with one TextEdit per
// occurrence. prepareRename returns the current symbol's range so the client
// can pre-fill the new-name box.
//
// Note: only the current file's references are renamed. Cross-file rename is
// a later, optional extension (the official server scopes a rename to the
// open document when a project root is not resolved).

use cj_ast::File;
use serde_json::{json, Value};

use crate::document_highlight::{collect_occurrences, type_or_expr_name_at};

/// textDocument/rename for the symbol at (line, character) — 0-based.
/// Returns a WorkspaceEdit (documentChanges: [{textDocument, edits}], the
/// format the official cjlsp suite expects) replacing every occurrence in
/// the current file with `new_name`, or null when the cursor is not on a
/// renameable symbol or the new name is empty.
pub fn rename_at(
    file: &File,
    uri: &str,
    line: u32,
    character: u32,
    new_name: &str,
    version: u32,
) -> Value {
    let Some(target) = type_or_expr_name_at(file, line, character) else {
        return Value::Null;
    };
    if new_name.is_empty() {
        return Value::Null;
    }
    let locs = collect_occurrences(file, &target);
    let edits: Vec<Value> = locs
        .iter()
        .map(|(l0, c0, e0)| {
            json!({
                "range": {
                    "start": {"line": l0, "character": c0},
                    "end": {"line": l0, "character": e0},
                },
                "newText": new_name,
            })
        })
        .collect();
    json!({
        "documentChanges": [{
            "textDocument": {"uri": uri, "version": version},
            "edits": edits,
        }]
    })
}

/// textDocument/prepareRename for the symbol at (line, character).
/// Returns the current symbol's range (+ placeholder name) so the client can
/// pre-fill the rename prompt, or null when the cursor is not on a symbol
/// (the client then shows its default "not renameable" error).
pub fn prepare_rename_at(file: &File, line: u32, character: u32) -> Value {
    let Some(target) = type_or_expr_name_at(file, line, character) else {
        return Value::Null;
    };
    // The symbol's own occurrence is the one containing the cursor — find its
    // range so prepareRename returns the exact span being renamed.
    for (l0, c0, e0) in collect_occurrences(file, &target) {
        if l0 == line && character >= c0 && character < e0 {
            return json!({
                "range": {
                    "start": {"line": l0, "character": c0},
                    "end": {"line": l0, "character": e0},
                },
                "placeholder": target,
            });
        }
    }
    // Cursor on the symbol but the occurrence walk missed the exact span
    // (e.g. cursor on the last char, exclusive end): fall back to the first
    // occurrence's range as a best effort.
    match collect_occurrences(file, &target).first() {
        Some((l0, c0, e0)) => json!({
            "range": {
                "start": {"line": l0, "character": c0},
                "end": {"line": l0, "character": e0},
            },
            "placeholder": target,
        }),
        None => Value::Null,
    }
}
