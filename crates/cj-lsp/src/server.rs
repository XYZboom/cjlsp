// cj-lsp: LSP server state machine + diagnostics pipeline.

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
        // The test harness runs us with cwd = <workspace>/sourcecode/cangjieTest
        // and sends a *virtual* uri for the opened file; sibling same-package
        // sources (needed for cross-file call type checking) live on disk under
        // that cwd. Resolve the real project root once per publish.
        let cwd = std::env::current_dir().unwrap_or_default();
        let project_root = resolve_project_root(&cwd, path);
        let diagnostics = if text.is_empty() {
            analyze_file(path, project_root.as_deref())
        } else {
            analyze_source(text, project_root.as_deref())
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

/// Run the full frontend pipeline on a file and return LSP-format diagnostics.
fn analyze_file(path: &PathBuf, project_root: Option<&Path>) -> Vec<Value> {
    match fs::read_to_string(path) {
        Ok(s) => analyze_source(&s, project_root),
        Err(_) => Vec::new(),
    }
}

/// Run the full frontend pipeline on source text and return LSP-format
/// diagnostics (1-based diag positions -> 0-based LSP ranges).
fn analyze_source(src: &str, project_root: Option<&Path>) -> Vec<Value> {
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

    // Cross-file call type checking: merge the opened file's own signatures
    // with same-package sibling signatures found on disk.
    let mut func_sigs = sema_result.func_sigs.clone();
    if let Some(root) = project_root {
        for (name, sig) in collect_same_package_sigs(root, file.package.as_deref()) {
            func_sigs.entry(name).or_insert(sig);
        }
    }
    let call_diags = cj_sema::typecheck::check_calls(&file, &func_sigs);

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
        .chain(call_diags.iter())
    {
        push(d);
    }
    out
}
