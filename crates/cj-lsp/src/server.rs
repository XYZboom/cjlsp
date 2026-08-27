// cj-lsp: LSP server state machine + diagnostics pipeline.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// LSP server: tracks open documents and computes diagnostics via the
/// cj-frontend pipeline (lexer -> parser -> sema collector/resolver/dep).
pub struct LspServer {
    /// uri -> (path on disk, latest text from didOpen/didChange)
    open_docs: HashMap<String, (PathBuf, String)>,
    shutdown_received: bool,
}

impl LspServer {
    pub fn new(_test_mode: bool) -> Self {
        LspServer {
            open_docs: HashMap::new(),
            shutdown_received: false,
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
    pub fn dispatch(&mut self, method: &str, _params: Value) -> Value {
        let id_placeholder = Value::Null; // filled by caller
        let _ = id_placeholder;
        match method {
            "initialize" => {
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
        // Apply content changes (full text in contentChanges[0].text for
        // full-sync mode; incremental is refined later).
        if let Some((_, text)) = self.open_docs.get_mut(&uri) {
            if let Some(chg) = params["contentChanges"].get(0) {
                if let Some(t) = chg["text"].as_str() {
                    *text = t.to_string();
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
    /// Prefers the in-memory text from didOpen/didChange; falls back to disk.
    fn publish_diagnostics(&mut self, uri: &str) -> Vec<Value> {
        let Some((path, text)) = self.open_docs.get(uri) else {
            return Vec::new();
        };
        let diagnostics = if text.is_empty() {
            analyze_file(path)
        } else {
            analyze_source(text)
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
}

/// Run the full frontend pipeline on a file and return LSP-format diagnostics.
fn analyze_file(path: &PathBuf) -> Vec<Value> {
    match fs::read_to_string(path) {
        Ok(s) => analyze_source(&s),
        Err(_) => Vec::new(),
    }
}

/// Run the full frontend pipeline on source text and return LSP-format
/// diagnostics (1-based diag positions -> 0-based LSP ranges).
fn analyze_source(src: &str) -> Vec<Value> {
    let mut parser = cj_parser::Parser::new(src, cj_lexer::Lexer::new(src).tokenize());
    let file = parser.run();

    // Sema: collect + resolve + dep graph.
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
    {
        push(d);
    }
    out
}
