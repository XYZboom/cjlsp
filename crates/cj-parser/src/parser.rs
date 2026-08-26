// cj-parser: Cangjie recursive-descent / Pratt parser.
//
// Behavioral reference: cangjie_compiler/src/Parse/ (ParseExpr.cpp Pratt loop,
// ParseDecl.cpp, ParseType.cpp, ...) — rewritten from scratch in Rust.
//
// Produces cj_ast nodes. All parse errors are collected into `diags` with
// positions; the parser always recovers and continues (matching cjc behavior).

use cj_ast::{CodePos, File, ImportSpec};
pub use cj_diag::Diag;
use cj_lexer::{Token, TokenKind};

use crate::decl::parse_decl;

/// Parser over a token stream (all tokens including NL/comments/END).
pub struct Parser<'a> {
    src: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    pub diags: Vec<Diag>,
}

impl<'a> Parser<'a> {
    pub fn new(src: &'a str, tokens: Vec<Token>) -> Self {
        Parser {
            src,
            tokens,
            pos: 0,
            diags: Vec::new(),
        }
    }

    // ---- token cursor ----

    /// Peek the next non-trivia token (skip NL/comments).
    pub fn peek(&self) -> TokenKind {
        self.peek_token().kind
    }

    pub fn peek_token(&self) -> &Token {
        let mut i = self.pos;
        while i < self.tokens.len() {
            let t = &self.tokens[i];
            if t.kind != TokenKind::COMMENT && t.kind != TokenKind::NL {
                return t;
            }
            i += 1;
        }
        // END sentinel — always present as last token.
        self.tokens.last().expect("token stream must end with END")
    }

    /// Peek the k-th non-trivia token ahead (0 = next).
    pub fn peek_ahead(&self, k: usize) -> TokenKind {
        let mut i = self.pos;
        let mut seen = 0usize;
        while i < self.tokens.len() {
            let t = &self.tokens[i];
            if t.kind != TokenKind::COMMENT && t.kind != TokenKind::NL {
                if seen == k {
                    return t.kind;
                }
                seen += 1;
            }
            i += 1;
        }
        TokenKind::END
    }

    pub fn peek_ahead_token(&self, k: usize) -> &Token {
        let mut i = self.pos;
        let mut seen = 0usize;
        while i < self.tokens.len() {
            let t = &self.tokens[i];
            if t.kind != TokenKind::COMMENT && t.kind != TokenKind::NL {
                if seen == k {
                    return t;
                }
                seen += 1;
            }
            i += 1;
        }
        self.tokens.last().expect("END sentinel")
    }

    /// Consume and return the next non-trivia token.
    pub fn advance(&mut self) -> Token {
        let tok = self.peek_token().clone();
        // move pos past this token (and any trivia before it)
        while self.pos < self.tokens.len() {
            let t = &self.tokens[self.pos];
            self.pos += 1;
            if t.kind == tok.kind && t.begin.offset == tok.begin.offset {
                break;
            }
        }
        tok
    }

    /// If the next non-trivia token is `kind`, consume it and return true.
    pub fn eat(&mut self, kind: TokenKind) -> bool {
        if self.peek() == kind {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Expect `kind`, else emit diagnostic and return the current token (recovery).
    pub fn expect(&mut self, kind: TokenKind) -> Token {
        let tok = self.peek_token().clone();
        if tok.kind == kind {
            self.advance();
            tok
        } else {
            self.error_at(
                tok.begin.line,
                tok.begin.column,
                &format!(
                    "expected '{}', found '{}'",
                    kind,
                    self.token_display(tok.kind)
                ),
            );
            tok
        }
    }

    /// True if the next non-trivia token is `kind`.
    pub fn at(&self, kind: TokenKind) -> bool {
        self.peek() == kind
    }

    pub fn at_any(&self, kinds: &[TokenKind]) -> bool {
        kinds.contains(&self.peek())
    }

    /// Record an error at a position.
    pub fn error_at(&mut self, line: u32, col: u32, msg: &str) {
        self.diags.push(cj_diag::Diag::error(line, col, msg));
    }

    /// Record an error anchored to a token.
    pub fn error(&mut self, tok: &Token, msg: &str) {
        self.error_at(tok.begin.line, tok.begin.column, msg);
    }

    /// Record a warning.
    pub fn warn_at(&mut self, line: u32, col: u32, msg: &str) {
        self.diags.push(cj_diag::Diag::warning(line, col, msg));
    }

    /// Record an error from an official diagnostic template (DiagId), filling
    /// its %s placeholders. Attaches the template's `here`/`note` text and the
    /// token's span so the SCAN output matches cjc byte-for-byte.
    pub fn error_id(&mut self, tok: &Token, id: cj_diag::DiagId, args: &[&str]) {
        let t = cj_diag::templates::template(id);
        let msg = fill_placeholders(t.message, args);
        let mut d = cj_diag::Diag::error(tok.begin.line, tok.begin.column, msg)
            .with_span(tok.end.line, tok.end.column);
        if let Some(here) = t.here {
            d = d.with_here(fill_placeholders(here, args));
        }
        for n in t.notes {
            d = d.with_note(fill_placeholders(n, args));
        }
        self.diags.push(d);
    }

    /// Same as [`Parser::error_id`] but for warnings.
    pub fn warn_id(&mut self, tok: &Token, id: cj_diag::DiagId, args: &[&str]) {
        let t = cj_diag::templates::template(id);
        let msg = fill_placeholders(t.message, args);
        let mut d = cj_diag::Diag::warning(tok.begin.line, tok.begin.column, msg)
            .with_span(tok.end.line, tok.end.column);
        if let Some(here) = t.here {
            d = d.with_here(fill_placeholders(here, args));
        }
        for n in t.notes {
            d = d.with_note(fill_placeholders(n, args));
        }
        self.diags.push(d);
    }

    pub fn token_display(&self, k: TokenKind) -> String {
        match k.literal() {
            "" => k.value_str().to_string(),
            lit => lit.to_string(),
        }
    }

    pub fn cur_pos(&self) -> CodePos {
        let t = self.peek_token();
        CodePos::new(
            t.begin.line,
            t.begin.column,
            t.begin.offset,
            t.end.line,
            t.end.column,
            t.end.offset,
        )
    }

    /// Position of the last consumed token.
    pub fn prev_pos(&self) -> CodePos {
        if self.pos > 0 {
            // find previous non-trivia token
            let mut i = self.pos;
            while i > 0 {
                i -= 1;
                let t = &self.tokens[i];
                if t.kind != TokenKind::COMMENT && t.kind != TokenKind::NL {
                    return CodePos::new(
                        t.begin.line,
                        t.begin.column,
                        t.begin.offset,
                        t.end.line,
                        t.end.column,
                        t.end.offset,
                    );
                }
            }
        }
        self.cur_pos()
    }

    pub fn source_text(&self, from_offset: usize, to_offset: usize) -> &str {
        let bytes = self.src.as_bytes();
        let from = from_offset.min(bytes.len());
        let to = to_offset.min(bytes.len());
        if from <= to {
            &self.src[from..to]
        } else {
            ""
        }
    }
}

pub fn fill_placeholders(template: &str, args: &[&str]) -> String {
    if args.is_empty() {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len() + 16);
    let mut arg_iter = args.iter();
    let mut rest = template;
    while let Some(pos) = rest.find("%s") {
        out.push_str(&rest[..pos]);
        match arg_iter.next() {
            Some(a) => out.push_str(a),
            None => out.push_str("%s"),
        }
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

/// Render a token's display form for diagnostics, matching cjc:
/// - keywords/identifiers: the raw text (e.g. `func`, `main`)
/// - multi-char punctuation/operators: `'...'` (e.g. `'@!'`, `'('`)
/// - literals: `'<integer literal>'` style
pub fn token_display_text(k: TokenKind) -> String {
    match k {
        TokenKind::IDENTIFIER => "identifier".to_string(),
        TokenKind::INTEGER_LITERAL => "integer literal".to_string(),
        TokenKind::FLOAT_LITERAL => "float literal".to_string(),
        TokenKind::STRING_LITERAL => "string literal".to_string(),
        TokenKind::RUNE_LITERAL => "rune literal".to_string(),
        TokenKind::BOOL_LITERAL => "boolean literal".to_string(),
        TokenKind::NL => "'<NL>'".to_string(),
        TokenKind::END => "end of file".to_string(),
        TokenKind::COMMENT => "comment".to_string(),
        // symbols/operators: rendered as quoted literal text, e.g. '@!', '('
        _ if k.symbol_like() => {
            let lit = k.literal();
            if lit.is_empty() {
                format!("'{}'", k.value_str())
            } else {
                format!("'{lit}'")
            }
        }
        // keywords fall through to their text form (e.g. `func`)
        _ => k.value_str().to_string(),
    }
}

// ---- entry point ----

impl<'a> Parser<'a> {
    /// Parse a whole file: optional `package`, imports, then top-level decls.
    pub fn parse_file(&mut self) -> File {
        let mut package = None;
        let mut imports = Vec::new();
        let mut decls = Vec::new();

        // package decl
        if self.at(TokenKind::PACKAGE) {
            let pkg_tok = self.advance();
            let name = self.parse_package_name();
            package = Some(name);
            let _ = pkg_tok;
        }

        // imports (repeatable, terminated by a decl keyword)
        while self.at(TokenKind::IMPORT) {
            if let Some(imp) = self.parse_import() {
                imports.push(imp);
            }
        }

        // top-level decls
        while self.peek() != TokenKind::END {
            if self.at(TokenKind::SEMI) {
                self.advance();
                continue;
            }
            match parse_decl(self, false) {
                Some(d) => decls.push(d),
                None => {
                    // recovery: official parse_expected_decl
                    // "expected declaration, found %s"
                    let t = self.peek_token().clone();
                    let found = token_display_text(t.kind);
                    self.error_id(&t, cj_diag::DiagId::PARSE_EXPECTED_DECL, &[&found]);
                    self.sync_to_decl_boundary();
                }
            }
        }

        let pos = CodePos::default();
        File {
            package,
            imports,
            decls,
            pos,
        }
    }

    /// Error recovery: advance until the next declaration boundary (NL, `;`,
    /// `}`, EOF, or a declaration-start keyword). Matches cjc behavior — after
    /// a failed decl we resync so the rest of the file parses normally.
    pub fn sync_to_decl_boundary(&mut self) {
        while self.peek() != TokenKind::END {
            match self.peek() {
                TokenKind::NL | TokenKind::SEMI | TokenKind::RCURL => {
                    self.advance();
                    break;
                }
                k if k.is_decl_start() => break,
                _ => {
                    self.advance();
                }
            }
        }
    }

    fn parse_package_name(&mut self) -> String {
        let mut parts = Vec::new();
        while self.peek() == TokenKind::IDENTIFIER || self.peek() == TokenKind::PACKAGE_IDENTIFIER {
            parts.push(self.advance().text);
            if !self.eat(TokenKind::DOT) {
                break;
            }
        }
        parts.join(".")
    }

    fn parse_import(&mut self) -> Option<ImportSpec> {
        let start = self.cur_pos();
        self.expect(TokenKind::IMPORT);
        let mut path = Vec::new();
        let mut glob = false;
        let mut selected = Vec::new();

        // org / module / ... :  or .*
        loop {
            match self.peek() {
                TokenKind::IDENTIFIER | TokenKind::PACKAGE_IDENTIFIER => {
                    path.push(self.advance().text);
                }
                TokenKind::MUL => {
                    // `*` handled below; break if it's after a dot
                }
                _ => break,
            }
            if self.eat(TokenKind::DOT) {
                continue;
            }
            break;
        }
        // `import a.b.*`
        if self.at(TokenKind::MUL) {
            self.advance();
            glob = true;
        } else if self.eat(TokenKind::COLON) {
            // `import a.b: X, Y`
            while self.peek() == TokenKind::IDENTIFIER {
                selected.push(self.advance().text);
                if !self.eat(TokenKind::COMMA) {
                    break;
                }
            }
        }
        let _ = start;
        Some(ImportSpec {
            path,
            glob,
            selected,
            pos: CodePos::default(),
        })
    }
}

/// Tokenize + parse a source string, returning the File AST and diagnostics.
pub fn parse_source(src: &str) -> (File, Vec<Diag>) {
    let tokens = cj_lexer::Lexer::new(src).tokenize();
    let mut parser = Parser::new(src, tokens);
    let file = parser.run();
    let diags = std::mem::take(&mut parser.diags);
    (file, diags)
}

impl<'a> Parser<'a> {
    pub fn run(&mut self) -> File {
        let file = self.parse_file();
        // diags left in self for caller to take
        file
    }
}
