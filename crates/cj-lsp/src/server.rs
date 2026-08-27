// cj-lsp: LSP server state machine + diagnostics pipeline.

use cj_diag::{DiagFix, FixKind};
use cj_lexer::TokenKind;
use cj_sema::FuncSig;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// LSP server: tracks open documents and computes diagnostics via the
/// cj-frontend pipeline (lexer -> parser -> sema collector/resolver/dep).
pub struct LspServer {
    /// uri -> (path on disk, latest text from didOpen/didChange)
    open_docs: HashMap<String, (PathBuf, String)>,
    shutdown_received: bool,
    /// rootUri from `initialize` — used to derive the expected package name
    /// of each open file (module name + directory chain under src/).
    root_uri: String,
}

impl LspServer {
    pub fn new(_test_mode: bool) -> Self {
        LspServer {
            open_docs: HashMap::new(),
            shutdown_received: false,
            root_uri: String::new(),
        }
    }

    pub fn handle_shutdown(&mut self) -> Value {
        self.shutdown_received = true;
        Value::Null
    }

    pub fn handle_exit(&mut self) {
        // exit with code 0 when shutdown was received (LSP spec)
        std::process::exit(if self.shutdown_received { 0 } else { 1 });
    }

    /// Handle a request (method with id); return the response to send.
    pub fn dispatch(&mut self, method: &str, params: Value) -> Value {
        let id_placeholder = Value::Null; // filled by caller
        let _ = id_placeholder;
        match method {
            "textDocument/completion" => self.handle_completion(&params),
            "textDocument/hover" => self.handle_hover(&params),
            "initialize" => {
                // Remember rootUri so package-name checks can derive the
                // expected package name for each opened file.
                if let Some(ru) = params.get("rootUri").and_then(|v| v.as_str()) {
                    self.root_uri = ru.to_string();
                }
                // Respond with server capabilities + info. The test harness
                // ignores `category/code/jsonrpc`, but capabilities shape
                // matters for feature tests later.
                json!({
                    "capabilities": {
                        "textDocumentSync": {"openClose": true, "change": 1, "save": {"includeText": true}},
                        "completionProvider": {"triggerCharacters": [".", ":", "!", "(", "[", "{", "@", "$"]},
                        "definitionProvider": true,
                        "referencesProvider": true,
                        "documentSymbolProvider": true,
                        "hoverProvider": true,
                        "renameProvider": {"prepareProvider": true},
                        "documentHighlightProvider": true,
                        "semanticTokensProvider": {
                            "legend": {
                                "tokenTypes": ["namespace","type","class","enum","interface","struct","typeParameter","parameter","variable","property","enumMember","event","function","method","macro","keyword","modifier","comment","string","number","regexp","operator","member","label"],
                                "tokenModifiers": ["declaration","definition","readonly","static","deprecated","abstract","async","modification","documentation","defaultLibrary"]
                            },
                            "range": true,
                            "full": true
                        }
                    },
                    "serverInfo": {"name": "LSPServer", "version": "0.1.0"}
                })
            }
            _ => json!({"result": Value::Null}),
        }
    }

    /// Handle a notification (no id). Returns server-initiated messages to send.
    pub fn notify(&mut self, method: &str, params: Value) -> Vec<Value> {
        match method {
            "textDocument/didOpen" => self.did_open(params),
            "textDocument/didChange" => self.did_change(params),
            "textDocument/didClose" => self.did_close(params),
            "initialized" => Vec::new(),
            _ => Vec::new(),
        }
    }

    /// Handle textDocument/completion: collect visible names at the cursor.
    fn handle_completion(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let line = params["position"]["line"].as_u64().unwrap_or(0) as u32;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as u32;

        let Some((_, source)) = self.open_docs.get(&uri) else {
            return json!([]);
        };
        // Parse the current buffer to collect visible declarations.
        let mut parser = cj_parser::Parser::new(source, cj_lexer::Lexer::new(source).tokenize());
        let file = parser.run();

        crate::completion::complete_at(&file, source, line, character, None)
    }

    /// Handle textDocument/hover: return declaration info at the cursor.
    fn handle_hover(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let line = params["position"]["line"].as_u64().unwrap_or(0) as u32;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as u32;

        let Some((_, source)) = self.open_docs.get(&uri) else {
            return Value::Null;
        };
        let mut parser = cj_parser::Parser::new(source, cj_lexer::Lexer::new(source).tokenize());
        let file = parser.run();

        let file_name = uri.rsplit('/').next().unwrap_or("").to_string();
        let pkg = file.package.as_deref();
        crate::hover::hover_at(&file, pkg, &file_name, line, character)
    }

    fn uri_to_path(uri: &str) -> Option<PathBuf> {
        // LSP URI like "file:///abs/path" or relative "diagnosticsTest/src/..."
        let p = uri.strip_prefix("file://").unwrap_or(uri);
        Some(PathBuf::from(p))
    }

    fn did_open(&mut self, params: Value) -> Vec<Value> {
        let doc = &params["textDocument"];
        let uri = doc["uri"].as_str().unwrap_or("").to_string();
        if uri.is_empty() {
            return Vec::new();
        }
        let path = Self::uri_to_path(&uri).unwrap_or_default();
        let text = doc["text"].as_str().unwrap_or("").to_string();
        self.open_docs.insert(uri.clone(), (path, text));
        self.publish_diagnostics(&uri)
    }

    fn did_change(&mut self, params: Value) -> Vec<Value> {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if uri.is_empty() {
            return Vec::new();
        }
        // Apply content changes. The suite sends incremental changes
        // (range + text for the edited span); other clients may send the full
        // text (no range). Both are handled.
        if let Some((_, text)) = self.open_docs.get_mut(&uri) {
            if let Some(changes) = params["contentChanges"].as_array() {
                for chg in changes {
                    if let Some(t) = chg["text"].as_str() {
                        if let Some(range) = chg.get("range") {
                            apply_incremental_change(text, range, t);
                        } else {
                            *text = t.to_string();
                        }
                    }
                }
            }
        }
        self.publish_diagnostics(&uri)
    }

    fn did_close(&mut self, params: Value) -> Vec<Value> {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        self.open_docs.remove(&uri);
        Vec::new()
    }

    /// Compute diagnostics for a document and emit publishDiagnostics.
    /// Uses the in-memory text from didOpen/didChange (the didOpen uri is a
    /// virtual path that may not exist on disk — content travels in `text`).
    fn publish_diagnostics(&mut self, uri: &str) -> Vec<Value> {
        let Some((path, text)) = self.open_docs.get(uri) else {
            return Vec::new();
        };
        // The test harness runs us with cwd = <workspace>/sourcecode/cangjieTest
        // and sends a *virtual* uri for the opened file; sibling same-package
        // sources (needed for cross-file call type checking) live on disk under
        // that cwd. Resolve the real project root once per publish.
        let cwd = std::env::current_dir().unwrap_or_default();
        let project_root = resolve_project_root(&cwd, path);
        let expected = self.expected_package_name(uri);
        let diagnostics = if text.is_empty() {
            // The didOpen uri is a *virtual* path that usually does not exist
            // on disk (content travels in `text`). Only fall back to disk when
            // it actually has content; an empty file (no package decl) must
            // still produce package-name diagnostics (diag_001).
            match fs::read_to_string(path) {
                Ok(s) if !s.is_empty() => {
                    analyze_source(&s, project_root.as_deref(), expected.as_deref(), uri)
                }
                _ => analyze_source("", project_root.as_deref(), expected.as_deref(), uri),
            }
        } else {
            analyze_source(text, project_root.as_deref(), expected.as_deref(), uri)
        };
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "diagnostics": diagnostics
            }
        });
        vec![msg]
    }

    /// Derive the expected package name of a file from its uri + rootUri.
    ///
    /// Rule (official cjlsp behavior): expected = <module name> + "." + the
    /// directory chain of the file relative to <root>/src/ (skipping src/).
    /// Files directly under src/ use just the module name.
    fn expected_package_name(&self, uri: &str) -> Option<String> {
        // Normalize both sides: drop a "file://" scheme prefix and any leading
        // slashes so they compare cleanly (the harness sends "file:////abs/..."
        // — with an extra slash before the root).
        let root = self
            .root_uri
            .strip_prefix("file://")
            .unwrap_or(&self.root_uri)
            .trim_start_matches('/');
        let u = uri
            .strip_prefix("file://")
            .unwrap_or(uri)
            .trim_start_matches('/');
        let rel = u
            .strip_prefix(root.trim_end_matches('/'))?
            .trim_start_matches('/');
        if rel.is_empty() {
            return None;
        }
        let comps: Vec<&str> = rel.split(['/', '\\']).collect();
        let src_idx = comps.iter().position(|c| *c == "src")?;
        // directories after src/, before the file name
        let dirs = &comps[src_idx + 1..];
        let dirs = &dirs[..dirs.len().saturating_sub(1)];
        let module = root
            .rsplit(['/', '\\'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("cnj");
        if dirs.is_empty() {
            return Some(module.to_string());
        }
        let mut pkg = module.to_string();
        for d in dirs {
            pkg.push('.');
            pkg.push_str(d);
        }
        Some(pkg)
    }
}

/// Resolve the real project root directory for a uri-derived path.
///
/// The harness rewrites the opened uri to a path that does NOT exist on disk
/// (`.../cjlsp/diagnosticsTest/src/func_error/diag_001.cj`), while the real
/// sources live under the server's cwd (`.../cjlsp/sourcecode/cangjieTest/`
/// + `diagnosticsTest/...`). We find the project root by trying every suffix
///   of the uri path resolved against cwd and returning the first that exists.
fn resolve_project_root(cwd: &Path, uri_path: &Path) -> Option<PathBuf> {
    let comps: Vec<&std::ffi::OsStr> = uri_path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s),
            _ => None,
        })
        .collect();
    for i in 0..comps.len() {
        let mut tail = PathBuf::new();
        for c in &comps[i..] {
            tail.push(c);
        }
        if cwd.join(&tail).exists() {
            return Some(cwd.join(comps[i]));
        }
    }
    None
}

/// Collect function signatures from all same-package `.cj` sources under the
/// project root (used for cross-file call type checking). Returns a map of
/// name -> FuncSig; the opened file's own sigs are merged separately.
fn collect_same_package_sigs(root: &Path, package: Option<&str>) -> HashMap<String, FuncSig> {
    let mut sigs = HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "cj") {
                let Ok(src) = fs::read_to_string(&p) else {
                    continue;
                };
                let mut parser =
                    cj_parser::Parser::new(&src, cj_lexer::Lexer::new(&src).tokenize());
                let f = parser.run();
                // Only same-package siblings are visible (spec Ch.03). A file
                // without a package decl is treated as package-less.
                let same_pkg = match (package, f.package.as_deref()) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                };
                if !same_pkg {
                    continue;
                }
                let r = cj_sema::Collector::new().collect_file(&f);
                for (name, sig) in r.func_sigs {
                    sigs.entry(name).or_insert(sig);
                }
            }
        }
    }
    sigs
}

/// Convert an LSP 0-based (line, character) position to a byte offset in
/// `src`. `character` counts Unicode code points (matches the lexer's column
/// semantics). Out-of-range positions clamp to the nearest valid offset.
fn lsp_pos_to_byte(src: &str, line: u32, character: u32) -> usize {
    let mut cur_line = 0u32;
    let mut line_start = 0usize;
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            if cur_line == line {
                let col_bytes: usize = src[line_start..i]
                    .chars()
                    .take(character as usize)
                    .map(char::len_utf8)
                    .sum();
                return line_start + col_bytes;
            }
            cur_line += 1;
            line_start = i + 1;
        }
    }
    // Last line (no trailing newline).
    let col_bytes: usize = src[line_start..]
        .chars()
        .take(character as usize)
        .map(char::len_utf8)
        .sum();
    line_start + col_bytes
}

/// Apply one incremental content change (range + new text) to `text`.
fn apply_incremental_change(text: &mut String, range: &Value, new_text: &str) {
    let start = &range["start"];
    let end = &range["end"];
    let s_line = start["line"].as_u64().unwrap_or(0) as u32;
    let s_char = start["character"].as_u64().unwrap_or(0) as u32;
    let e_line = end["line"].as_u64().unwrap_or(0) as u32;
    let e_char = end["character"].as_u64().unwrap_or(0) as u32;
    let s = lsp_pos_to_byte(text, s_line, s_char);
    let e = lsp_pos_to_byte(text, e_line, e_char);
    if s <= e && e <= text.len() {
        text.replace_range(s..e, new_text);
    }
}

/// File-name for macro diagnostics: the project root's file name or "unknown".
fn path_or_unknown(project_root: Option<&Path>) -> &str {
    project_root
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
}

/// Run the full frontend pipeline on source text and return LSP-format
/// diagnostics (1-based diag positions -> 0-based LSP ranges).
///
/// `project_root` — the resolved project root (for cross-file function
/// signature collection); `expected` — the package name derived from the
/// file's URI (pass `None` when it cannot be inferred); `uri` — the file's
/// LSP URI, used as the `changes` target of unused-symbol code actions.
fn analyze_source(
    src: &str,
    project_root: Option<&Path>,
    expected: Option<&str>,
    uri: &str,
) -> Vec<Value> {
    // Tokens for quickfix deletion-range computation. Strings/comments are
    // already handled by the lexer, so brace/paren matching below is safe.
    let toks = cj_lexer::Lexer::new(src).tokenize();
    let mut parser = cj_parser::Parser::new(src, cj_lexer::Lexer::new(src).tokenize());
    let file = parser.run();

    // Sema: collect + resolve + dep graph + package/import checks.
    let collector = cj_sema::Collector::new();
    let sema_result = collector.collect_file(&file);
    let mut pkg = cj_sema::PackageTable::default();
    pkg.merge(&sema_result);
    let mut resolver = cj_sema::resolver::Resolver::new(&pkg);
    resolver.resolve_file(&file);
    let resolve_diags = resolver.take_diags();
    let dep_graph = cj_sema::dep_graph::DepGraph::build(&[&file]);
    let dep_diags = dep_graph.detect_cycles();
    let unused_diags = cj_sema::unused::detect_unused(&file);
    // Literal type checking for typed var decls (M3c, spec Ch.02): String=1,
    // Int8='x', Int8=999999 etc. — the 011 suite.
    let lit_diags = cj_sema::typecheck::check_decls(&file);
    // Macro expansion (spec Ch.14): builtin + cached .so via SDK; diagnostics
    // carry the expansion preview (report position + generated code).
    let mut macro_cache = cj_sema::macro_cache::MacroCache::new();
    let (expansions, macro_diags) = cj_sema::expander::expand_file_with_cache(
        &file,
        path_or_unknown(project_root),
        &mut macro_cache,
        project_root,
    );

    // Cross-file call type checking: merge the opened file's own signatures
    // with same-package sibling signatures found on disk.
    let mut func_sigs = sema_result.func_sigs.clone();
    if let Some(root) = project_root {
        for (name, sig) in collect_same_package_sigs(root, file.package.as_deref()) {
            func_sigs.entry(name).or_insert(sig);
        }
    }
    let call_diags = cj_sema::typecheck::check_calls(&file, &func_sigs);
    let package_diags = cj_sema::package::check_package(&file, expected);
    let overload_diags = cj_sema::overload::detect_overload_conflicts(&file);
    // Targeted semantic checks (var-init ordering, undeclared types,
    // super/finalizer rules, override params, optional-param-in-abstract,
    // bare type-name expressions). `src` is only used for the finalizer
    // modifier scan (the parser drops modifier tokens).
    let sema_checks = cj_sema::checks::check_semantics(&file, &pkg, Some(src));

    // Collect all diagnostics, then attach macro-expansion preview notes to
    // any whose position falls inside an expansion's source span (code that
    // came from the expansion — official cjc shows "the code after the macro
    // is expanded as follows" for these).
    let mut all_diags: Vec<cj_diag::Diag> = Vec::new();
    all_diags.extend(parser.diags.iter().cloned());
    all_diags.extend(sema_result.diags.iter().cloned());
    all_diags.extend(resolve_diags.iter().cloned());
    all_diags.extend(dep_diags.iter().cloned());
    all_diags.extend(unused_diags.iter().cloned());
    all_diags.extend(call_diags.iter().cloned());
    all_diags.extend(lit_diags.iter().cloned());
    all_diags.extend(package_diags.iter().cloned());
    all_diags.extend(overload_diags.iter().cloned());
    all_diags.extend(sema_checks.iter().cloned());
    all_diags.extend(macro_diags.iter().cloned());
    attach_expansion_notes(&mut all_diags, &expansions);

    // Convert (line, col) 1-based -> LSP 0-based positions.
    let mut out = Vec::new();
    let mut push = |d: &cj_diag::Diag| {
        let severity = match d.severity {
            cj_diag::Severity::Error => 1,
            cj_diag::Severity::Warning => 2,
            cj_diag::Severity::Note => 3,
            cj_diag::Severity::Hint => 4,
            cj_diag::Severity::Fatal => 1,
        };
        let end_col = if d.end_col > d.col {
            d.end_col
        } else {
            d.col + 1
        };
        // LSP `relatedInformation` entries for the macro-expansion preview
        // note. Only emitted when the diag actually carries that note (the
        // official diagnostic key set stays unchanged for all other cases).
        let related = expansion_related_info(d, uri);
        // Unused-declaration diagnostics carry `tags: [Unnecessary]` plus a
        // `quickfix.removeUnusedSymbol` code action deleting the declaration.
        // The official key set for these is `code/codeActions/data/message/
        // range/severity/source/tags` — note there is NO `category` key, and
        // `code` is 0. Other diagnostics keep the plain shape below.
        if let Some(fix) = &d.fix {
            let (ca, data) = match compute_remove_range(src, &toks, fix) {
                Some(((sl, sc), (el, ec))) => {
                    let ca = json!([{
                        "edit": {
                            "changes": {
                                uri: [{
                                    "newText": "",
                                    "range": {
                                        "start": {"line": sl, "character": sc},
                                        "end": {"line": el, "character": ec}
                                    }
                                }]
                            }
                        },
                        "kind": "quickfix.removeUnusedSymbol",
                        "title": fix.title,
                    }]);
                    let cloned = ca.clone();
                    (cloned, json!({"codeActions": ca}))
                }
                None => (Value::Null, Value::Null),
            };
            let mut obj = json!({
                "code": 0,
                "codeActions": ca,
                "data": data,
                "message": lsp_diag_message(d),
                "range": {
                    "start": {"line": d.line.saturating_sub(1), "character": d.col.saturating_sub(1)},
                    "end": {"line": d.end_line.saturating_sub(1), "character": end_col.saturating_sub(1)}
                },
                "severity": severity,
                "source": "Cangjie",
                "tags": if d.tags.is_empty() { Value::Null } else { json!(d.tags) },
            });
            if let Some(ri) = related {
                obj["relatedInformation"] = ri;
            }
            out.push(obj);
        } else {
            let mut obj = json!({
                "category": null,
                "code": null,
                "codeActions": null,
                "range": {
                    "start": {"line": d.line.saturating_sub(1), "character": d.col.saturating_sub(1)},
                    "end": {"line": d.end_line.saturating_sub(1), "character": end_col.saturating_sub(1)}
                },
                "severity": severity,
                "message": lsp_diag_message(d),
                "source": "Cangjie",
                "data": {"codeActions": null}
            });
            if let Some(ri) = related {
                obj["relatedInformation"] = ri;
            }
            out.push(obj);
        }
    };
    for d in &all_diags {
        push(d);
    }
    out
}

/// LSP diagnostic message text: the official server renders a parsed diag's
/// `SubDiagnostic` notes inline in the message (e.g. the top-level note
/// `only declarations or macro expressions can be used in the top-level`
/// appended to `expected declaration, found 'test06'` — 008/018). Other notes
/// (redefinition `'X' is previously declared here`) are relatedInformation, not
/// message suffixes, so only the known inline note is appended.
fn lsp_diag_message(d: &cj_diag::Diag) -> String {
    const TOPLEVEL_NOTE: &str =
        "only declarations or macro expressions can be used in the top-level";
    for n in &d.notes {
        if n == TOPLEVEL_NOTE {
            return format!("{}, {}", d.message, n);
        }
    }
    d.message.clone()
}

/// Official macro-expansion note header (mirrors cjc DiagnosticEngine.h
/// `MACROCALL_CODE` = "the code after the macro is expanded as follows").
const MACRO_EXPANSION_NOTE: &str = "the code after the macro is expanded as follows";

/// Attach the macro-expansion preview note to every diagnostic whose position
/// falls inside an expansion's source span (code that came from the
/// expansion). The note matches the official cjc format:
///
///   note: the code after the macro is expanded as follows
///       /* 6.1 */print(x)
///
/// The `/* <line>.1 */` marker is the expansion's call line (the official
/// compiler labels generated code with the call-site position).
fn attach_expansion_notes(
    diags: &mut [cj_diag::Diag],
    expansions: &[cj_sema::expander::Expansion],
) {
    for d in diags.iter_mut() {
        if d.notes.iter().any(|n| n == MACRO_EXPANSION_NOTE) {
            continue;
        }
        let Some(exp) = expansions.iter().find(|e| e.contains(d.line, d.col)) else {
            continue;
        };
        d.notes.push(MACRO_EXPANSION_NOTE.to_string());
        d.notes
            .push(format!("/* {}.1 */{}", exp.call_line, exp.expanded));
    }
}

/// LSP `relatedInformation` entries for the macro-expansion preview note.
/// Returns `None` (so the `relatedInformation` key is omitted entirely) unless
/// the diagnostic carries that note — the official diagnostic key set stays
/// unchanged for every other case.
fn expansion_related_info(d: &cj_diag::Diag, uri: &str) -> Option<Value> {
    if !d.notes.iter().any(|n| n == MACRO_EXPANSION_NOTE) {
        return None;
    }
    let end_col = if d.end_col > d.col {
        d.end_col
    } else {
        d.col + 1
    };
    let range = json!({
        "start": {"line": d.line.saturating_sub(1), "character": d.col.saturating_sub(1)},
        "end": {"line": d.end_line.saturating_sub(1), "character": end_col.saturating_sub(1)}
    });
    let infos: Vec<Value> = d
        .notes
        .iter()
        .map(|n| json!({ "location": { "uri": uri, "range": range.clone() }, "message": n }))
        .collect();
    Some(Value::Array(infos))
}

/// Whether `kind` is a declaration modifier that can precede the decl keyword
/// (`open class C`, `public static func f`) — included in the deletion range.
fn is_decl_modifier(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::PUBLIC
            | TokenKind::PRIVATE
            | TokenKind::PROTECTED
            | TokenKind::INTERNAL
            | TokenKind::STATIC
            | TokenKind::ABSTRACT
            | TokenKind::OPEN
            | TokenKind::SEALED
            | TokenKind::MUT
            | TokenKind::OPERATOR
    )
}

/// Index of the first token of a declaration: walk backward from the decl
/// keyword (at `kw_idx`) over modifier tokens, so the removal range covers
/// `public static func f` from `public`, not from `func`.
fn decl_start_index(toks: &[cj_lexer::Token], kw_idx: usize) -> usize {
    let mut i = kw_idx;
    while i > 0 && is_decl_modifier(toks[i - 1].kind) {
        i -= 1;
    }
    i
}

/// Convert a byte offset to an LSP 0-based (line, character) position.
/// `character` counts Unicode scalar values (matches the lexer's columns).
fn offset_to_lsp_pos(src: &str, off: usize) -> (u32, u32) {
    let off = off.min(src.len());
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (i, b) in src.bytes().enumerate().take(off) {
        if b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    let char_count = src[line_start..off].chars().count() as u32;
    (line, char_count)
}

/// Deletion range of an unused *parameter*: from the name, extended over the
/// marker/type/default plus one adjacent comma with surrounding whitespace,
/// matching the official cjlsp output:
///   `a:Int8,` / `,b: Bool` / `a: Int64 = 1, ` / `b!: Float64 = 1.0`
/// The first param claims the trailing comma (+ following whitespace); a
/// non-first param without a default claims the leading comma (+ preceding
/// whitespace); params with a default value claim no comma. Type scanning
/// stops at `,`/`)` at depth 0, so generic/tuple types stay whole.
fn param_range(
    toks: &[cj_lexer::Token],
    src: &str,
    name_idx: usize,
) -> Option<((u32, u32), (u32, u32))> {
    let name = &toks[name_idx];
    let bytes = src.as_bytes();

    // Is this the first param of the list? (previous non-NL/COMMENT token is `(`.)
    let is_first = toks[..name_idx]
        .iter()
        .rev()
        .find(|t| !matches!(t.kind, TokenKind::NL | TokenKind::COMMENT))
        .is_none_or(|t| t.kind == TokenKind::LPAREN);

    // Walk the `!` marker, `: type`, and an optional `= default` expression.
    let mut i = name_idx + 1;
    if toks.get(i).is_some_and(|t| t.kind == TokenKind::NOT) {
        i += 1;
    }
    if toks.get(i).is_some_and(|t| t.kind == TokenKind::COLON) {
        i += 1;
    }
    let mut depth = 0i32;
    let mut has_default = false;
    let mut end_off = name.end.offset;
    while let Some(t) = toks.get(i) {
        let boundary = depth == 0 && matches!(t.kind, TokenKind::COMMA | TokenKind::RPAREN);
        if boundary {
            break;
        }
        if depth == 0 && t.kind == TokenKind::ASSIGN {
            has_default = true;
        }
        match t.kind {
            TokenKind::LPAREN | TokenKind::LSQUARE | TokenKind::LT => depth += 1,
            TokenKind::RPAREN | TokenKind::RSQUARE | TokenKind::GT => depth = (depth - 1).max(0),
            _ => {}
        }
        end_off = t.end.offset;
        i += 1;
    }

    // Start: extend LEFT over whitespace and a preceding comma for non-first
    // params without a default (`, b: Bool`).
    let mut start_off = name.begin.offset;
    if !is_first && !has_default {
        let mut s = start_off;
        while s > 0 && matches!(bytes[s - 1], b' ' | b'\t' | b'\r') {
            s -= 1;
        }
        if s > 0 && bytes[s - 1] == b',' {
            start_off = s - 1;
        }
    }

    // End: extend RIGHT over a following comma + whitespace for the first
    // param (`a: Int8, `).
    if is_first {
        let mut e = end_off;
        while e < bytes.len() && matches!(bytes[e], b' ' | b'\t' | b'\r') {
            e += 1;
        }
        if e < bytes.len() && bytes[e] == b',' {
            e += 1;
            while e < bytes.len() && matches!(bytes[e], b' ' | b'\t' | b'\r') {
                e += 1;
            }
            end_off = e;
        }
    }

    Some((
        offset_to_lsp_pos(src, start_off),
        offset_to_lsp_pos(src, end_off),
    ))
}

/// End of a single-line `let`/`var` statement: the last token before the next
/// newline/semicolon/closing-brace at depth 0 (multi-line inits stay whole via
/// paren/bracket/brace depth tracking).
fn var_end(toks: &[cj_lexer::Token], start_idx: usize) -> Option<&cj_lexer::Token> {
    let mut depth = 0i32;
    let mut last: Option<&cj_lexer::Token> = None;
    for t in toks.iter().skip(start_idx + 1) {
        match t.kind {
            TokenKind::NL | TokenKind::SEMI => {
                if depth == 0 {
                    return last;
                }
            }
            TokenKind::RCURL if depth == 0 => return last,
            TokenKind::LPAREN | TokenKind::LSQUARE | TokenKind::LCURL => {
                depth += 1;
                last = Some(t);
            }
            TokenKind::RPAREN | TokenKind::RSQUARE | TokenKind::RCURL => {
                depth = (depth - 1).max(0);
                last = Some(t);
            }
            _ => last = Some(t),
        }
    }
    last
}

/// The closing brace of a braced declaration: find the body `{` (the first
/// `{` at paren/bracket depth 0 after the decl start — param lists, parent
/// types and generic args are skipped), then match `}` by brace depth.
fn braced_end(toks: &[cj_lexer::Token], start_idx: usize) -> Option<&cj_lexer::Token> {
    let mut paren = 0i32;
    let mut i = start_idx + 1;
    let mut body: Option<usize> = None;
    while let Some(t) = toks.get(i) {
        match t.kind {
            TokenKind::LCURL if paren == 0 => {
                body = Some(i);
                break;
            }
            TokenKind::LPAREN | TokenKind::LSQUARE => paren += 1,
            TokenKind::RPAREN | TokenKind::RSQUARE => paren = (paren - 1).max(0),
            _ => {}
        }
        i += 1;
    }
    let body = body?;
    let mut brace = 1i32;
    for t in toks.iter().skip(body + 1) {
        match t.kind {
            TokenKind::LCURL => brace += 1,
            TokenKind::RCURL => {
                brace -= 1;
                if brace == 0 {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

/// Compute the LSP 0-based deletion range `(start, end)` of an unused-symbol
/// quickfix from the source text + tokens. The fix's `start_line`/`start_col`
/// anchor the declaration keyword/name; the end depends on the fix kind.
fn compute_remove_range(
    src: &str,
    toks: &[cj_lexer::Token],
    fix: &DiagFix,
) -> Option<((u32, u32), (u32, u32))> {
    let start_off = lsp_pos_to_byte(
        src,
        fix.start_line.saturating_sub(1),
        fix.start_col.saturating_sub(1),
    );
    let kw_idx = toks.iter().position(|t| t.begin.offset == start_off)?;
    match fix.kind {
        FixKind::Param => param_range(toks, src, kw_idx),
        _ => {
            let start_idx = decl_start_index(toks, kw_idx);
            let start = &toks[start_idx];
            let s = (
                start.begin.line.saturating_sub(1),
                start.begin.column.saturating_sub(1),
            );
            let end_tok = match fix.kind {
                FixKind::Var => var_end(toks, start_idx)?,
                _ => braced_end(toks, start_idx)?,
            };
            let e = (
                end_tok.end.line.saturating_sub(1),
                end_tok.end.column.saturating_sub(1),
            );
            Some((s, e))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cj_diag::Diag;
    use cj_sema::expander::Expansion;

    fn expansion(line: u32, col: u32, end_line: u32, end_col: u32, text: &str) -> Expansion {
        Expansion {
            call_line: line,
            call_col: col,
            call_end_line: end_line,
            call_end_col: end_col,
            expanded: text.to_string(),
        }
    }

    #[test]
    fn attaches_note_to_in_span_diag() {
        // A diag on the `)` of `@Wrap(42)` (line 2, cols 9..=17) is inside the
        // expansion span and must receive the official expansion-preview note.
        let exp = expansion(2, 9, 2, 18, "print ( 42");
        let d = Diag::error(2, 17, "expected declaration, found '!'");
        let mut diags = vec![d];
        attach_expansion_notes(&mut diags, &[exp]);
        assert_eq!(
            diags[0].notes,
            vec![
                "the code after the macro is expanded as follows".to_string(),
                "/* 2.1 */print ( 42".to_string(),
            ]
        );
    }

    #[test]
    fn skips_out_of_span_diag() {
        // A diag before the call, on another line, or after the call's span
        // must NOT get the macro note (it is not about expanded code).
        let exp = expansion(2, 9, 2, 18, "print ( 42");
        let before = Diag::error(2, 8, "x");
        let other_line = Diag::error(3, 1, "y");
        let after = Diag::error(2, 19, "z");
        let mut diags = vec![before, other_line, after];
        attach_expansion_notes(&mut diags, &[exp]);
        for d in &diags {
            assert!(!d.notes.iter().any(|n| n == MACRO_EXPANSION_NOTE));
        }
    }

    #[test]
    fn related_info_only_for_macro_note() {
        let mut plain = Diag::error(1, 1, "m");
        plain
            .notes
            .push("only declarations or macro expressions can be used in the top-level".into());
        assert_eq!(expansion_related_info(&plain, "file:///a.cj"), None);

        let mut noted = Diag::error(2, 17, "expected declaration, found '!'");
        noted
            .notes
            .push("the code after the macro is expanded as follows".into());
        noted.notes.push("/* 2.1 */print ( 42".into());
        let ri = expansion_related_info(&noted, "file:///a.cj").expect("note present");
        let arr = ri.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[1]["message"], "/* 2.1 */print ( 42");
        assert_eq!(arr[0]["location"]["uri"], "file:///a.cj");
    }
}
