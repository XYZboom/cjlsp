// cj-lsp: LSP server state machine + diagnostics pipeline.

use crate::hover::StdlibIndex;
use cj_diag::{DiagFix, FixKind};
use cj_lexer::TokenKind;
use cj_sema::FuncSig;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// LSP server: tracks open documents and computes diagnostics via the
/// cj-frontend pipeline (lexer -> parser -> sema collector/resolver/dep).
pub struct LspServer {
    /// uri -> (path on disk, latest text from didOpen/didChange)
    open_docs: HashMap<String, (PathBuf, String)>,
    /// uri -> current document version as tracked from didOpen/didChange.
    /// didOpen initialises to 0; each didChange sets it to the client's
    /// stated version + 1 (the version the text will have AFTER the change).
    /// WorkspaceEdit responses (rename, applyEdit...) echo this so clients
    /// can reconcile their own version counters.
    doc_versions: HashMap<String, u32>,
    shutdown_received: bool,
    /// rootUri from `initialize` — used to derive the expected package name
    /// of each open file (module name + directory chain under src/).
    root_uri: String,
    /// project root dir -> parsed same-package sibling scan cache (file path
    /// -> parsed file). Turns the per-request O(project) cross-file scan into
    /// an mtime diff: only changed/new sibling files are re-parsed, so a
    /// medium repo stays fast on every completion/diagnostics request.
    scan_cache: HashMap<PathBuf, HashMap<PathBuf, CachedSibling>>,
    /// The downloaded standard-library symbol index (loaded from
    /// ~/.cangjie-lsp/std/<version>/index.json at startup), used as the last
    /// fallback for definition jumps into the real stdlib source. Path is
    /// resolved dynamically — never hardcoded.
    stdlib: Option<StdlibIndex>,
}

impl LspServer {
    pub fn new(_test_mode: bool) -> Self {
        LspServer {
            open_docs: HashMap::new(),
            doc_versions: HashMap::new(),
            shutdown_received: false,
            root_uri: String::new(),
            scan_cache: HashMap::new(),
            stdlib: StdlibIndex::load(),
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
            "textDocument/definition" => self.handle_definition(&params),
            "textDocument/references" => self.handle_references(&params),
            "textDocument/codeLens" => self.handle_code_lens(&params),
            "textDocument/documentHighlight" => self.handle_document_highlight(&params),
            "textDocument/rename" => self.handle_rename(&params),
            "textDocument/prepareRename" => self.handle_prepare_rename(&params),
            "textDocument/signatureHelp" => self.handle_signature_help(&params),
            "textDocument/documentSymbol" => self.handle_document_symbol(&params),
            "workspace/symbol" => self.handle_workspace_symbol(&params),
            "textDocument/semanticTokens/full" => self.handle_semantic_tokens(&params),
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
                        "codeLensProvider": true,
                        "definitionProvider": true,
                        "referencesProvider": true,
                        "documentSymbolProvider": true,
                        "workspaceSymbolProvider": true,
                        "hoverProvider": true,
                        "signatureHelpProvider": {"triggerCharacters": ["(", ","]},
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

        let Some((path, source)) = self.open_docs.get(&uri) else {
            return json!([]);
        };
        // Copy the path + buffer text out so the scan-cache mutable borrow
        // below doesn't conflict with the open_docs borrow.
        let (path, source) = (path.clone(), source.clone());
        // Parse the current buffer to collect visible declarations.
        let mut parser = cj_parser::Parser::new(&source, cj_lexer::Lexer::new(&source).tokenize());
        let file = parser.run();

        self.completion_inner(&file, &source, &path, &uri, line, character)
    }

    fn completion_inner(
        &mut self,
        file: &cj_ast::File,
        source: &str,
        path: &std::path::Path,
        uri: &str,
        line: u32,
        character: u32,
    ) -> Value {
        let _ = uri;
        // Resolve the project root so completion can include same-package
        // sibling file decls (official behavior: cross-file completion). The
        // scan cache re-parses only changed/new siblings (mtime diff), not
        // the whole package, on every request.
        let cwd = std::env::current_dir().unwrap_or_default();
        let project_root = resolve_project_root(&cwd, path);
        // The current file's imports name the sibling packages whose decls are
        // visible cross-file (spec Ch.03). Wildcard (`pkg.*`) and selected
        // (`pkg.X`) imports both make that package visible; same-package
        // siblings are always visible.
        let mut imported: Vec<String> = Vec::new();
        for imp in &file.imports {
            let pkg = imp.path.join(".");
            if !pkg.is_empty() && !imported.contains(&pkg) {
                imported.push(pkg);
            }
        }
        let siblings = project_root.as_deref().map(|r| {
            let cache = self.scan_cache.entry(r.to_path_buf()).or_default();
            collect_same_package_candidates(
                cache,
                r,
                file.package.as_deref(),
                &imported,
                &path.to_string_lossy(),
            )
        });
        // Parsed sibling docs (file + source) for cross-file member-access
        // type/alias/var resolution. Borrow the cache immutably AFTER the
        // mutable refresh above has completed.
        let sibling_docs: Vec<(&cj_ast::File, &str)> = match &project_root {
            Some(r) => self
                .scan_cache
                .get(r)
                .map(|cache| {
                    collect_visible_sibling_docs(cache, r, file.package.as_deref(), &imported)
                })
                .unwrap_or_default(),
            None => Vec::new(),
        };

        crate::completion::complete_at(
            file,
            source,
            line,
            character,
            siblings.as_deref(),
            &sibling_docs,
            project_root.as_deref(),
            uri,
        )
    }

    /// Handle textDocument/hover: return declaration info at the cursor.
    fn handle_hover(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let line = params["position"]["line"].as_u64().unwrap_or(0) as u32;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as u32;

        let Some((path, source)) = self.open_docs.get(&uri).cloned() else {
            return Value::Null;
        };
        let mut parser = cj_parser::Parser::new(&source, cj_lexer::Lexer::new(&source).tokenize());
        let file = parser.run();

        let file_name = uri.rsplit('/').next().unwrap_or("").to_string();
        let pkg = file.package.as_deref();

        // Cross-file hover: refresh the sibling scan cache (same-package +
        // imported packages) so a symbol declared in another file of the
        // project gets a rendered declaration (mirrors handle_definition
        // wiring — completion/hover/definition all share the scan cache).
        let cwd = std::env::current_dir().unwrap_or_default();
        let project_root = resolve_project_root(&cwd, &path);
        let mut imported: Vec<String> = Vec::new();
        for imp in &file.imports {
            let pkg = imp.path.join(".");
            if !pkg.is_empty() && !imported.contains(&pkg) {
                imported.push(pkg);
            }
        }
        let siblings: Vec<(std::path::PathBuf, &cj_ast::File, &str)> = match &project_root {
            Some(r) => {
                let cache = self.scan_cache.entry(r.to_path_buf()).or_default();
                // Refresh first (drop the borrow) so collect_* sees fresh data.
                let _ = collect_same_package_candidates(
                    cache,
                    r,
                    file.package.as_deref(),
                    &imported,
                    &path.to_string_lossy(),
                );
                collect_sibling_docs_with_path(cache, r, file.package.as_deref(), &imported)
            }
            None => Vec::new(),
        };

        crate::hover::hover_at(&file, &source, pkg, &file_name, line, character, &siblings)
    }

    /// Handle textDocument/definition: return the declaration location at the
    /// cursor (the name's span in the declaring file).
    fn handle_definition(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let line = params["position"]["line"].as_u64().unwrap_or(0) as u32;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as u32;

        let Some((path, source)) = self.open_docs.get(&uri).cloned() else {
            return Value::Null;
        };
        let mut parser = cj_parser::Parser::new(&source, cj_lexer::Lexer::new(&source).tokenize());
        let file = parser.run();

        // Cross-file definition: refresh the sibling scan cache (same-package
        // + imported packages) so a symbol defined in another file of the
        // project can be jumped to. Mirrors completion/hover wiring.
        let cwd = std::env::current_dir().unwrap_or_default();
        let project_root = resolve_project_root(&cwd, &path);
        let mut imported: Vec<String> = Vec::new();
        for imp in &file.imports {
            let pkg = imp.path.join(".");
            if !pkg.is_empty() && !imported.contains(&pkg) {
                imported.push(pkg);
            }
        }
        let siblings: Vec<(std::path::PathBuf, &cj_ast::File, &str)> = match &project_root {
            Some(r) => {
                let cache = self.scan_cache.entry(r.to_path_buf()).or_default();
                // Refresh first (drop the borrow) so collect_* sees fresh data.
                let _ = collect_same_package_candidates(
                    cache,
                    r,
                    file.package.as_deref(),
                    &imported,
                    &path.to_string_lossy(),
                );
                collect_sibling_docs_with_path(cache, r, file.package.as_deref(), &imported)
            }
            None => Vec::new(),
        };

        crate::hover::definition_at(
            &file,
            &source,
            &uri,
            line,
            character,
            &siblings,
            self.stdlib.as_ref(),
        )
    }

    /// Handle textDocument/references: all locations referencing the name at
    /// the cursor (declaration + usages).
    fn handle_references(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let line = params["position"]["line"].as_u64().unwrap_or(0) as u32;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as u32;
        let include_decl = params["context"]["includeDeclaration"]
            .as_bool()
            .unwrap_or(true);

        let Some((_, source)) = self.open_docs.get(&uri) else {
            return json!([]);
        };
        let mut parser = cj_parser::Parser::new(source, cj_lexer::Lexer::new(source).tokenize());
        let file = parser.run();

        crate::references::references_at(&file, &uri, line, character, include_decl)
    }

    /// Handle textDocument/codeLens: a "N references" lens above every
    /// top-level declaration. Reuses the references collection (code_lens.rs
    /// walks the file once, grouping Name expressions by name) so the count
    /// equals what textDocument/references would return for that decl.
    fn handle_code_lens(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let Some((_, source)) = self.open_docs.get(&uri) else {
            return json!([]);
        };
        let mut parser = cj_parser::Parser::new(source, cj_lexer::Lexer::new(source).tokenize());
        let file = parser.run();
        crate::code_lens::code_lenses(&file, &uri)
    }

    /// Handle textDocument/documentSymbol: the file outline — a hierarchical
    /// symbol tree (name/kind/range/selectionRange/children) for every
    /// top-level decl, with class/struct/interface/extend members nested and
    /// enum cases as children of their enum.
    fn handle_document_symbol(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let Some((_, source)) = self.open_docs.get(&uri) else {
            return Value::Null;
        };
        let mut parser = cj_parser::Parser::new(source, cj_lexer::Lexer::new(source).tokenize());
        let file = parser.run();
        crate::document_symbol::document_symbols(&file, source)
    }

    /// Handle textDocument/semanticTokens/full: syntax-highlight the whole
    /// buffer. Returns LSP-encoded semantic tokens (relative deltas).
    fn handle_semantic_tokens(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let Some((_, source)) = self.open_docs.get(&uri) else {
            return json!({ "data": [] });
        };
        let mut parser = cj_parser::Parser::new(source, cj_lexer::Lexer::new(source).tokenize());
        let file = parser.run();
        let data = crate::semantic::semantic_tokens_full(source, &file);
        json!({ "data": data })
    }

    /// Handle textDocument/signatureHelp: the parameter list of the function
    /// / ctor / method whose call parens enclose the cursor.
    fn handle_signature_help(&mut self, params: &Value) -> Value {
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
        crate::signature::signature_help_at(&file, source, line, character)
    }

    /// Handle textDocument/documentHighlight: highlight all occurrences of
    /// the symbol under the cursor in the current file.
    fn handle_document_highlight(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let line = params["position"]["line"].as_u64().unwrap_or(0) as u32;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as u32;
        let Some((_, source)) = self.open_docs.get(&uri) else {
            return json!([]);
        };
        let mut parser = cj_parser::Parser::new(source, cj_lexer::Lexer::new(source).tokenize());
        let file = parser.run();
        crate::document_highlight::document_highlight_at(&file, line, character)
    }

    /// Handle textDocument/rename: replace every occurrence of the symbol
    /// under the cursor in the current file with `newName`, returning a
    /// WorkspaceEdit (changes: {uri: [TextEdit...]}).
    fn handle_rename(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let line = params["position"]["line"].as_u64().unwrap_or(0) as u32;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as u32;
        let new_name = params["newName"].as_str().unwrap_or("").to_string();
        let Some((_, source)) = self.open_docs.get(&uri) else {
            return Value::Null;
        };
        let version = self.doc_versions.get(&uri).copied().unwrap_or(0);
        let mut parser = cj_parser::Parser::new(source, cj_lexer::Lexer::new(source).tokenize());
        let file = parser.run();
        crate::rename::rename_at(&file, &uri, line, character, &new_name, version)
    }

    /// Handle textDocument/prepareRename: return the current symbol's range
    /// (+ placeholder) so the client can pre-fill the new-name box, or null
    /// when the cursor is not on a renameable symbol.
    fn handle_prepare_rename(&mut self, params: &Value) -> Value {
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
        crate::rename::prepare_rename_at(&file, line, character)
    }

    /// Handle workspace/symbol: Ctrl+T project-wide symbol search. Reuses the
    /// sibling scan cache (all packages under the project root) so every
    /// file.decls is collected, filters by `query` (case-insensitive fuzzy
    /// subsequence on the raw identifier), and returns WorkspaceSymbol
    /// (name/kind/location/containerName). Also includes the current files.
    fn handle_workspace_symbol(&mut self, params: &Value) -> Value {
        let query = params["query"].as_str().unwrap_or("").to_string();
        let cwd = std::env::current_dir().unwrap_or_default();
        // Project root: rootUri from initialize when present, else cwd. Then
        // normalize to the nearest cjpm root so scan_dirs covers src-dir.
        let mut root: PathBuf = if self.root_uri.is_empty() {
            cwd
        } else {
            let p = self
                .root_uri
                .strip_prefix("file://")
                .unwrap_or(&self.root_uri);
            PathBuf::from(p)
        };
        if let Some(r) = crate::cjpm::find_project_root(&root) {
            root = r;
        }
        // Refresh the sibling scan cache for this root with package=None so
        // EVERY .cj file under the project is parsed (workspace-wide search).
        let cache = self.scan_cache.entry(root.clone()).or_default();
        let _ = refresh_sibling_cache(cache, &root, None, &[], None);
        // Collect all cached files (no package/import visibility filter).
        let mut files: Vec<(std::path::PathBuf, cj_ast::File, String)> = cache
            .iter()
            .map(|(p, c)| (p.clone(), c.file.clone(), c.source.clone()))
            .collect();
        // Merge the currently open docs (live buffers) so unsaved edits are
        // reflected; an open doc not yet on disk is added as well.
        for (path, text) in self.open_docs.values() {
            let mut parser = cj_parser::Parser::new(text, cj_lexer::Lexer::new(text).tokenize());
            let file = parser.run();
            if let Some(existing) = files.iter_mut().find(|(p, _, _)| p == path) {
                existing.1 = file;
                existing.2 = text.clone();
            } else {
                files.push((path.clone(), file, text.clone()));
            }
        }
        let refs: Vec<(std::path::PathBuf, &cj_ast::File, &str)> = files
            .iter()
            .map(|(p, f, s)| (p.clone(), f, s.as_str()))
            .collect();
        crate::workspace_symbol::collect_workspace_symbols(&refs, &query)
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
        // Initial document version is 0; didChange bumps it to the client's
        // stated version + 1 (matches the official cjlsp rename suite).
        self.doc_versions.insert(uri.clone(), 0);
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
        // Version AFTER the applied change = client's stated version + 1
        // (official cjlsp rename suite echoes this in WorkspaceEdit).
        if let Some(v) = params["textDocument"]["version"].as_u64() {
            self.doc_versions.insert(uri.clone(), v as u32 + 1);
        }
        self.publish_diagnostics(&uri)
    }

    fn did_close(&mut self, params: Value) -> Vec<Value> {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or("")
            .to_string();
        self.open_docs.remove(&uri);
        self.doc_versions.remove(&uri);
        Vec::new()
    }

    /// Compute diagnostics for a document and emit publishDiagnostics.
    /// Uses the in-memory text from didOpen/didChange (the didOpen uri is a
    /// virtual path that may not exist on disk — content travels in `text`).
    fn publish_diagnostics(&mut self, uri: &str) -> Vec<Value> {
        let Some((path, text)) = self.open_docs.get(uri) else {
            return Vec::new();
        };
        // Copy the path + text out so the scan-cache mutable borrow below
        // doesn't conflict with the open_docs borrow.
        let (path, text) = (path.clone(), text.clone());
        // The test harness runs us with cwd = <workspace>/sourcecode/cangjieTest
        // and sends a *virtual* uri for the opened file; sibling same-package
        // sources (needed for cross-file call type checking) live on disk under
        // that cwd. Resolve the real project root once per publish.
        let cwd = std::env::current_dir().unwrap_or_default();
        let project_root = resolve_project_root(&cwd, &path);
        let expected = self.expected_package_name(uri);
        let diagnostics = if text.is_empty() {
            // The didOpen uri is a *virtual* path that usually does not exist
            // on disk (content travels in `text`). Only fall back to disk when
            // it actually has content; an empty file (no package decl) must
            // still produce package-name diagnostics (diag_001).
            match fs::read_to_string(&path) {
                Ok(s) if !s.is_empty() => analyze_source(
                    &s,
                    project_root.as_deref(),
                    expected.as_deref(),
                    uri,
                    &mut self.scan_cache,
                ),
                _ => analyze_source(
                    "",
                    project_root.as_deref(),
                    expected.as_deref(),
                    uri,
                    &mut self.scan_cache,
                ),
            }
        } else {
            analyze_source(
                &text,
                project_root.as_deref(),
                expected.as_deref(),
                uri,
                &mut self.scan_cache,
            )
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
    // 1. cjpm project: nearest ancestor with cjpm.toml beats cwd inference.
    if let Some(root) = crate::cjpm::find_project_root(uri_path) {
        return Some(root);
    }
    // 2. Legacy: virtual harness uri resolved against the server cwd.
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
            let root = cwd.join(comps[i]);
            if root.is_dir() {
                return Some(root);
            }
        }
    }
    None
}

/// One parsed same-package sibling file in the cross-file scan cache.
///
/// Holds the file's mtime (to detect disk changes), its package name (for
/// the spec Ch.03 same-package filter), the parsed AST + source (so member
/// completion can resolve cross-file types/aliases/vars), and both derived
/// payloads: the completion `Candidate`s and the collected function
/// signatures for cross-file call type checking.
struct CachedSibling {
    mtime: SystemTime,
    package: Option<String>,
    file: cj_ast::File,
    source: String,
    candidates: Vec<crate::completion::Candidate>,
    func_sigs: HashMap<String, FuncSig>,
}

/// Package-filtered sibling data produced by a cache refresh: completion
/// candidates + cross-file call signatures.
struct SiblingData {
    candidates: Vec<crate::completion::Candidate>,
    func_sigs: HashMap<String, FuncSig>,
}

/// Refresh a per-root sibling scan cache against disk and return the merged,
/// package-filtered data.
///
/// Cost model: the directory walk (`read_dir`) is cheap; each `.cj` file's
/// mtime is diffed against the cache and ONLY changed/new files are re-parsed
/// (read + lex + parse + collect). Deleted files are dropped. The merge keeps
/// files whose package matches `package` OR one of `imported` (spec Ch.03 —
/// same-package siblings plus wildcard/selected imports are visible);
/// `self_path`, when given, excludes the opened file itself from the
/// candidate list (its decls come from the live buffer).
fn refresh_sibling_cache(
    cache: &mut HashMap<PathBuf, CachedSibling>,
    root: &Path,
    package: Option<&str>,
    imported: &[String],
    self_path: Option<&str>,
) -> SiblingData {
    // 1. Cheap directory walk: (path, mtime) for every .cj file under root.
    let mut disk: HashMap<PathBuf, SystemTime> = HashMap::new();
    for start in crate::cjpm::scan_dirs(root) {
        let mut stack = vec![start];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "cj") {
                    if let Ok(m) = entry.metadata().and_then(|m| m.modified()) {
                        disk.insert(p, m);
                    }
                }
            }
        }
    }

    // 2. Re-parse only changed/new files; drop entries for deleted files.
    for (p, mtime) in &disk {
        let stale = cache.get(p).is_none_or(|c| c.mtime != *mtime);
        if stale {
            match parse_sibling_file(p, *mtime) {
                Some(f) => {
                    cache.insert(p.clone(), f);
                }
                None => {
                    cache.remove(p);
                }
            }
        }
    }
    cache.retain(|p, _| disk.contains_key(p));

    // 3. Merge same-package + imported payloads (excluding the opened file
    // for candidates). Decls from local cjpm dependencies are also visible
    // (their packages appear in visible_packages).
    let visible = crate::cjpm::visible_packages(root);
    let mut candidates = Vec::new();
    let mut func_sigs = HashMap::new();
    for (p, c) in cache.iter() {
        let pkg = c.package.as_deref().unwrap_or_default();
        let same_pkg = match package {
            Some(a) => a == pkg || visible.contains(pkg) || imported.iter().any(|i| i == pkg),
            None => true,
        };
        if !same_pkg {
            continue;
        }
        if self_path.is_none_or(|sp| !p.to_string_lossy().ends_with(sp)) {
            candidates.extend(c.candidates.iter().cloned());
        }
        func_sigs.extend(c.func_sigs.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    SiblingData {
        candidates,
        func_sigs,
    }
}

/// Parse one sibling `.cj` file into its cache entry (package + candidates +
/// function signatures). Returns `None` when the file cannot be read (mirrors
/// the old per-file `continue` on read errors).
fn parse_sibling_file(p: &Path, mtime: SystemTime) -> Option<CachedSibling> {
    let src = fs::read_to_string(p).ok()?;
    let mut parser = cj_parser::Parser::new(&src, cj_lexer::Lexer::new(&src).tokenize());
    let f = parser.run();
    let package = f.package.clone();
    let candidates = crate::completion::sibling_candidates(&f, &src);
    let r = cj_sema::Collector::new().collect_file(&f);
    Some(CachedSibling {
        mtime,
        package,
        file: f,
        source: src,
        candidates,
        func_sigs: r.func_sigs,
    })
}

/// Collect function signatures from all same-package `.cj` sources under the
/// project root (used for cross-file call type checking). Returns a map of
/// name -> FuncSig; the opened file's own sigs are merged separately. Reuses
/// the per-file scan cache — only changed/new files are re-parsed.
fn collect_same_package_sigs(
    cache: &mut HashMap<PathBuf, CachedSibling>,
    root: &Path,
    package: Option<&str>,
) -> HashMap<String, FuncSig> {
    refresh_sibling_cache(cache, root, package, &[], None).func_sigs
}

/// Collect top-level decl candidates from same-package sibling .cj files
/// under the project root (cross-file completion). Excludes `path` itself.
/// Returns the candidates (labels/kinds/details) for completion to merge.
/// Reuses the per-file scan cache — only changed/new files are re-parsed.
fn collect_same_package_candidates(
    cache: &mut HashMap<PathBuf, CachedSibling>,
    root: &Path,
    package: Option<&str>,
    imported: &[String],
    self_path: &str,
) -> Vec<crate::completion::Candidate> {
    refresh_sibling_cache(cache, root, package, imported, Some(self_path)).candidates
}

/// Collect parsed sibling `File`s (+ sources) whose package is visible to the
/// opened file (same package, wildcard/selected import, or cjpm dependency).
/// The completion engine uses these to resolve member-access types, aliases
/// and vars defined cross-file. Reuses the scan cache (already refreshed).
fn collect_visible_sibling_docs<'a>(
    cache: &'a HashMap<PathBuf, CachedSibling>,
    root: &Path,
    package: Option<&str>,
    imported: &[String],
) -> Vec<(&'a cj_ast::File, &'a str)> {
    let visible = crate::cjpm::visible_packages(root);
    cache
        .iter()
        .filter(|(_, c)| {
            let pkg = c.package.as_deref().unwrap_or_default();
            match package {
                Some(a) => a == pkg || imported.iter().any(|i| i == pkg) || visible.contains(pkg),
                None => true,
            }
        })
        .map(|(_, c)| (&c.file, c.source.as_str()))
        .collect()
}

/// Collect parsed sibling `File`s (+ sources + paths) whose package is
/// visible to the opened file. Used by cross-file definition (jump to a
/// declaration in another file of the same project).
fn collect_sibling_docs_with_path<'a>(
    cache: &'a HashMap<PathBuf, CachedSibling>,
    root: &Path,
    package: Option<&str>,
    imported: &[String],
) -> Vec<(PathBuf, &'a cj_ast::File, &'a str)> {
    let visible = crate::cjpm::visible_packages(root);
    cache
        .iter()
        .filter(|(_, c)| {
            let pkg = c.package.as_deref().unwrap_or_default();
            match package {
                Some(a) => a == pkg || imported.iter().any(|i| i == pkg) || visible.contains(pkg),
                None => true,
            }
        })
        .map(|(p, c)| (p.clone(), &c.file, c.source.as_str()))
        .collect()
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

/// Count characters in the given 0-based line.
fn line_char_len(src: &str, line: u32) -> usize {
    let mut cur_line = 0u32;
    let mut line_start = 0usize;
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            if cur_line == line {
                return src[line_start..i].chars().count();
            }
            cur_line += 1;
            line_start = i + 1;
        }
    }
    src[line_start..].chars().count()
}

/// Apply one incremental content change (range + new text) to `text`.
fn apply_incremental_change(text: &mut String, range: &Value, new_text: &str) {
    let start = &range["start"];
    let end = &range["end"];
    let s_line = start["line"].as_u64().unwrap_or(0) as u32;
    let s_char = start["character"].as_u64().unwrap_or(0) as u32;
    let e_line = end["line"].as_u64().unwrap_or(0) as u32;
    let e_char = end["character"].as_u64().unwrap_or(0) as u32;
    let s_len = line_char_len(text, s_line) as u32;
    let e_len = line_char_len(text, e_line) as u32;
    // Only skip an edit whose start is STRICTLY past the line's content
    // (a truly invalid range that claims to consume chars that don't exist).
    // An edit that starts AT the line's end is a legitimate insertion into a
    // trailing empty line (e.g. typing "C1" on the final line of a file) and
    // must be applied — the official engine registers it (050/107).
    if s_line == e_line && s_char > s_len && e_char > e_len {
        return;
    }
    let s = lsp_pos_to_byte(text, s_line, s_char);
    // Clamp the end to the line's length: a range end past the line means
    // "to end of line" (replacing a typed span that ends past the content).
    let e = if e_char as usize > e_len as usize {
        lsp_pos_to_byte(text, e_line, e_len)
    } else {
        lsp_pos_to_byte(text, e_line, e_char)
    };
    let s = s.min(e);
    if e <= text.len() {
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
    scan_cache: &mut HashMap<PathBuf, HashMap<PathBuf, CachedSibling>>,
) -> Vec<Value> {
    // Tokens for quickfix deletion-range computation. Strings/comments are
    // already handled by the lexer, so brace/paren matching below is safe.
    // Lexer errors (number-suffix etc.) surface as diagnostics, matching the
    // official pipeline (lexer runs before the parser).
    let mut lexer = cj_lexer::Lexer::new(src);
    let toks = lexer.tokenize();
    let lex_diags: Vec<cj_diag::Diag> = std::mem::take(&mut lexer.errors)
        .into_iter()
        .map(|e| cj_diag::Diag::error(e.pos.line, e.pos.column, e.message))
        .collect();
    let mut parser = cj_parser::Parser::new(src, toks.clone());
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
    // with same-package sibling signatures found on disk (scan-cached: only
    // changed/new siblings re-parsed).
    let mut func_sigs = sema_result.func_sigs.clone();
    if let Some(root) = project_root {
        let cache = scan_cache.entry(root.to_path_buf()).or_default();
        for (name, sig) in collect_same_package_sigs(cache, root, file.package.as_deref()) {
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
    all_diags.extend(lex_diags.iter().cloned());
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

    /// A temp project dir used by the scan-cache tests (unique per process).
    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("t33_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_project_root_returns_dir_only() {
        // The trap: cwd sitting INSIDE the file's directory makes the first
        // existing suffix be the FILE itself. The is_dir() guard must reject
        // it (a file is not a usable project root), not return it.
        let dir = temp_repo("root");
        fs::write(dir.join("cur.cj"), "package p\n").unwrap();
        let uri = dir.join("cur.cj");

        assert_eq!(resolve_project_root(&dir, &uri), None);

        // cwd = parent directory -> resolves to the directory containing the
        // file, never the file.
        let parent = dir.parent().expect("temp dir has a parent");
        assert_eq!(resolve_project_root(parent, &uri), Some(dir.clone()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_cache_refreshes_only_changed_files() {
        let dir = temp_repo("scan");
        let a = dir.join("a.cj");
        let b = dir.join("b.cj");
        fs::write(&a, "package p\nfunc alpha(): Int64 { return 1 }\n").unwrap();
        fs::write(&b, "package p\nclass Beta {}\n").unwrap();

        let mut cache: HashMap<PathBuf, CachedSibling> = HashMap::new();
        let root = dir.clone();

        // First refresh parses every sibling: both files cached, alpha sig
        // visible from a.cj (b.cj has no funcs).
        let d1 = refresh_sibling_cache(&mut cache, &root, Some("p"), &[], None);
        assert!(d1.func_sigs.contains_key("alpha"));
        assert!(cache.contains_key(&a) && cache.contains_key(&b));
        let d1_count = d1.candidates.len();

        // Unchanged mtime -> cache hit, identical output (no re-parse).
        let d2 = refresh_sibling_cache(&mut cache, &root, Some("p"), &[], None);
        assert!(d2.func_sigs.contains_key("alpha"));
        assert_eq!(d2.candidates.len(), d1_count);

        // Change b.cj (mtime advances) -> only b re-parsed; its new func
        // signature appears and the cached candidates update.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&b, "package p\nfunc gamma(): Int64 { return 2 }\n").unwrap();
        let d3 = refresh_sibling_cache(&mut cache, &root, Some("p"), &[], None);
        assert!(d3.func_sigs.contains_key("gamma"), "changed file re-parsed");
        assert!(!d3.func_sigs.contains_key("beta"));

        // Delete a.cj -> its cache entry drops and alpha disappears.
        fs::remove_file(&a).unwrap();
        let d4 = refresh_sibling_cache(&mut cache, &root, Some("p"), &[], None);
        assert!(!cache.contains_key(&a), "deleted file dropped from cache");
        assert!(!d4.func_sigs.contains_key("alpha"));
        assert!(d4.candidates.len() < d1_count);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_cache_excludes_self_file_for_candidates() {
        let dir = temp_repo("self");
        let a = dir.join("a.cj");
        let b = dir.join("b.cj");
        fs::write(&a, "package p\nfunc alpha(): Int64 { return 1 }\n").unwrap();
        fs::write(&b, "package p\nclass Beta {}\n").unwrap();

        let mut cache: HashMap<PathBuf, CachedSibling> = HashMap::new();
        let root = dir.clone();

        let base = refresh_sibling_cache(&mut cache, &root, Some("p"), &[], None)
            .candidates
            .len();
        // The opened file a.cj is excluded from completion candidates (its
        // decls come from the live buffer) — but its func sigs are still
        // collected for the cross-file call checks.
        let a_str = a.to_string_lossy().to_string();
        let excl = refresh_sibling_cache(&mut cache, &root, Some("p"), &[], Some(&a_str));
        assert!(excl.candidates.len() < base && !excl.candidates.is_empty());
        assert!(
            excl.func_sigs.contains_key("alpha"),
            "self sigs still merged"
        );

        let _ = fs::remove_dir_all(&dir);
    }
    #[test]
    fn cjpm_root_and_src_scan_scope() {
        let base = std::env::temp_dir().join(format!("cjpm_e2e_{}", std::process::id()));
        fs::create_dir_all(base.join("src/mypkg/sub")).unwrap();
        fs::create_dir_all(base.join("target")).unwrap();
        fs::write(
            base.join("cjpm.toml"),
            "[package]\nname = \"demo\"\nsrc-dir = \"src\"\n\n[dependencies]\n",
        )
        .unwrap();
        fs::write(
            base.join("src/mypkg/a.cj"),
            "package demo.mypkg\n\nclass A {}\n",
        )
        .unwrap();
        fs::write(
            base.join("src/mypkg/sub/b.cj"),
            "package demo.mypkg\n\nfunc from_sub(): Int64 { return 2 }\n",
        )
        .unwrap();
        // Stray .cj OUTSIDE src/: a whole-tree scan would wrongly include it.
        fs::write(
            base.join("target/stray.cj"),
            "package demo.mypkg\n\nclass Stray {}\n",
        )
        .unwrap();

        let opened = base.join("src/mypkg/a.cj");
        let cwd = std::env::current_dir().unwrap_or_default();

        // 1. cjpm root: nearest cjpm.toml ancestor wins over cwd inference.
        let root = resolve_project_root(&cwd, &opened).expect("project root");
        assert_eq!(root, base);

        // 2. Scan scope = src-dir only: the src/sub sibling is a candidate
        // (bare + call-snippet variants), the target/ stray decl is NOT.
        let mut cache: HashMap<PathBuf, CachedSibling> = HashMap::new();
        let cands = collect_same_package_candidates(
            &mut cache,
            &root,
            Some("demo.mypkg"),
            &[],
            &opened.to_string_lossy(),
        );
        let labels: Vec<&str> = cands.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"from_sub"), "src/sub sibling visible");
        assert!(!labels.contains(&"Stray"), "stray target/ decl excluded");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn cjpm_local_dependency_visible() {
        let base = std::env::temp_dir().join(format!("cjpm_dep_e2e_{}", std::process::id()));
        fs::create_dir_all(base.join("src")).unwrap();
        fs::create_dir_all(base.join("lib/src")).unwrap();
        fs::write(
            base.join("cjpm.toml"),
            "[dependencies]\nlib = {path=\"./lib\"}\n\n[package]\nname = \"app\"\nsrc-dir = \"src\"\n",
        )
        .unwrap();
        fs::write(base.join("src/main.cj"), "package app\n\nfunc main() {}\n").unwrap();
        // The dependency is its own cjpm package ("lib" from its own toml).
        fs::write(
            base.join("lib/cjpm.toml"),
            "[package]\nname = \"lib\"\nsrc-dir = \"src\"\n",
        )
        .unwrap();
        fs::write(
            base.join("lib/src/helper.cj"),
            "package lib\n\nfunc lib_helper(): Int64 { return 7 }\n",
        )
        .unwrap();

        let opened = base.join("src/main.cj");
        let cwd = std::env::current_dir().unwrap_or_default();
        let root = resolve_project_root(&cwd, &opened).expect("project root");
        let mut cache: HashMap<PathBuf, CachedSibling> = HashMap::new();
        let cands = collect_same_package_candidates(
            &mut cache,
            &root,
            Some("app"),
            &[],
            &opened.to_string_lossy(),
        );
        let labels: Vec<&str> = cands.iter().map(|c| c.label.as_str()).collect();
        // lib_helper from the local dependency package is visible cross-file,
        // despite its package ("lib") differing from the opened file's.
        assert!(
            labels.contains(&"lib_helper"),
            "local dependency decl visible cross-file"
        );
        // The opened file itself is excluded.
        assert!(!labels.contains(&"main"));

        let _ = fs::remove_dir_all(&base);
    }
}
