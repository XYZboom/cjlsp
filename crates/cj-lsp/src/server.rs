// cj-lsp: LSP server state machine + diagnostics pipeline.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

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
        let Some((_, text)) = self.open_docs.get(uri) else {
            return Vec::new();
        };
        let expected = self.expected_package_name(uri);
        let diagnostics = analyze_source(text, expected.as_deref());
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

/// Run the full frontend pipeline on source text and return LSP-format
/// diagnostics (1-based diag positions -> 0-based LSP ranges).
///
/// `expected` — the package name derived from the file's URI (pass `None`
/// when it cannot be inferred); drives package-level checks.
fn analyze_source(src: &str, expected: Option<&str>) -> Vec<Value> {
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
    let package_diags = cj_sema::package::check_package(&file, expected);

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
        out.push(json!({
            "category": null,
            "code": null,
            "codeActions": null,
            "range": {
                "start": {"line": d.line.saturating_sub(1), "character": d.col.saturating_sub(1)},
                "end": {"line": d.end_line.saturating_sub(1), "character": end_col.saturating_sub(1)}
            },
            "severity": severity,
            "message": d.message,
            "source": "Cangjie",
            "data": {"codeActions": null}
        }));
    };
    for d in parser
        .diags
        .iter()
        .chain(sema_result.diags.iter())
        .chain(resolve_diags.iter())
        .chain(dep_diags.iter())
        .chain(unused_diags.iter())
        .chain(package_diags.iter())
    {
        push(d);
    }
    out
}
