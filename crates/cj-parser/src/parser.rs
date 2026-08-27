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

    /// Peek the first non-trivia token that is not an access modifier
    /// (skipping `public`/`private`/`protected`/`internal`). Used to decide
    /// whether a modifier is a prefix of an `import` declaration without
    /// consuming it (the modifier may belong to the next declaration).
    fn peek_past_import_modifiers(&self) -> TokenKind {
        let mut i = self.pos;
        while i < self.tokens.len() {
            let k = self.tokens[i].kind;
            if k == TokenKind::COMMENT || k == TokenKind::NL {
                i += 1;
                continue;
            }
            if matches!(
                k,
                TokenKind::PUBLIC | TokenKind::PRIVATE | TokenKind::PROTECTED | TokenKind::INTERNAL
            ) {
                i += 1;
                continue;
            }
            return k;
        }
        TokenKind::END
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

    /// Kind of the raw token at absolute index `i` (may be trivia). Returns
    /// `END` past the end of the stream. Used by lookahead scanners that must
    /// walk the raw token stream (e.g. generic-args / lambda detection).
    pub(crate) fn raw_kind_at(&self, i: usize) -> TokenKind {
        self.tokens.get(i).map(|t| t.kind).unwrap_or(TokenKind::END)
    }

    /// True when the next non-trivia token can close a `>` generic-args list:
    /// `>` itself, or a compound token that starts with `>` (`>>`, `>=`,
    /// `>>=`) which must be split by `eat_gt`.
    pub(crate) fn is_gt_close(&self) -> bool {
        matches!(
            self.peek(),
            TokenKind::GT | TokenKind::RSHIFT | TokenKind::GE | TokenKind::RSHIFT_ASSIGN
        )
    }

    /// Consume the next non-trivia token as a generic-args `>` closer.
    /// Compound tokens (`>>`, `>=`, `>>=`) are split so the remaining
    /// `>`/`=` stays in the stream for the enclosing level (nested generics
    /// `A<B<C>>`; the lexer merges adjacent closers into one token).
    pub(crate) fn eat_gt(&mut self) -> Token {
        // find the raw index of the next non-trivia token
        let mut i = self.pos;
        while i < self.tokens.len() {
            let k = self.tokens[i].kind;
            if k == TokenKind::COMMENT || k == TokenKind::NL {
                i += 1;
                continue;
            }
            break;
        }
        if i >= self.tokens.len() {
            return self.peek_token().clone();
        }
        let tok = self.tokens[i].clone();
        match tok.kind {
            TokenKind::GT => {
                self.pos = i + 1;
                tok
            }
            TokenKind::RSHIFT => {
                // `>>` → `>` + `>`
                self.tokens[i].kind = TokenKind::GT;
                self.tokens[i].text = String::new();
                self.tokens[i].end.column = tok.begin.column + 1;
                self.tokens[i].end.offset = tok.begin.offset + 1;
                let mut rest = tok.clone();
                rest.kind = TokenKind::GT;
                rest.text = String::new();
                rest.begin.column = tok.begin.column + 1;
                rest.begin.offset = tok.begin.offset + 1;
                self.tokens.insert(i + 1, rest);
                self.pos = i + 1;
                self.tokens[i].clone()
            }
            TokenKind::GE => {
                // `>=` → `>` + `=`
                self.tokens[i].kind = TokenKind::GT;
                self.tokens[i].text = String::new();
                self.tokens[i].end.column = tok.begin.column + 1;
                self.tokens[i].end.offset = tok.begin.offset + 1;
                let mut rest = tok.clone();
                rest.kind = TokenKind::ASSIGN;
                rest.text = String::new();
                rest.begin.column = tok.begin.column + 1;
                rest.begin.offset = tok.begin.offset + 1;
                self.tokens.insert(i + 1, rest);
                self.pos = i + 1;
                self.tokens[i].clone()
            }
            TokenKind::RSHIFT_ASSIGN => {
                // `>>=` → `>` + `>` + `=`
                self.tokens[i].kind = TokenKind::GT;
                self.tokens[i].text = String::new();
                self.tokens[i].end.column = tok.begin.column + 1;
                self.tokens[i].end.offset = tok.begin.offset + 1;
                let mut gt = tok.clone();
                gt.kind = TokenKind::GT;
                gt.text = String::new();
                gt.begin.column = tok.begin.column + 1;
                gt.begin.offset = tok.begin.offset + 1;
                gt.end.column = tok.begin.column + 2;
                gt.end.offset = tok.begin.offset + 2;
                let mut eq = tok.clone();
                eq.kind = TokenKind::ASSIGN;
                eq.text = String::new();
                eq.begin.column = tok.begin.column + 2;
                eq.begin.offset = tok.begin.offset + 2;
                self.tokens.insert(i + 1, gt);
                self.tokens.insert(i + 2, eq);
                self.pos = i + 1;
                self.tokens[i].clone()
            }
            other => {
                // no closing `>`: report and recover (mirror `expect(GT)`)
                let found = crate::token_display_text(&tok);
                self.error_id(
                    &tok,
                    cj_diag::DiagId::PARSE_EXPECTED_RIGHT_DELIMITER,
                    &["<", ">", "<", &found],
                );
                self.pos = i + 1;
                let _ = other;
                tok
            }
        }
    }

    /// Absolute index of the next unconsumed token in the raw stream.
    pub(crate) fn cursor(&self) -> usize {
        self.pos
    }

    /// Total number of raw tokens (including trivia and END sentinel).
    pub(crate) fn token_len(&self) -> usize {
        self.tokens.len()
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

    /// Like [`Parser::error_id`] but with an extra dynamic note appended
    /// (e.g. the top-level-decl note cjc adds in `DiagExpectedDeclaration`).
    pub fn error_id_with_note(
        &mut self,
        tok: &Token,
        id: cj_diag::DiagId,
        args: &[&str],
        note: &str,
    ) {
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
        d = d.with_note(note);
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

/// Render a MODIFIER name for diagnostics: bare keyword text without the
/// `keyword '...'` wrapper (official modifier diagnostics use bare names, e.g.
/// `unexpected modifier 'sealed' on ...`, `redundant modifier: 'public'`).
pub fn modifier_display(t: &Token) -> String {
    let lit = t.kind.literal();
    if lit.is_empty() {
        t.kind.value_str().to_string()
    } else {
        lit.to_string()
    }
}

/// Render a token's display form for diagnostics, matching official
/// `ConvertToken` (ParserDiag.cpp):
///   END => `'<EOF>'`; NL => `'<NL>'`
///   keyword => `keyword 'func'`
///   integer/float literal => `literal '42'`
///   bool/identifier/package identifier => `'x'`
///   everything else => `'('`  (token's literal text)
pub fn token_display_text(t: &Token) -> String {
    match t.kind {
        TokenKind::END => "'<EOF>'".to_string(),
        TokenKind::NL => "'<NL>'".to_string(),
        // Official TOKEN(ILLEGAL, "illegal", "", 0) — ConvertToken renders it
        // as `''` (empty quoted string), e.g. the package-name diag in 007.
        TokenKind::ILLEGAL => "''".to_string(),
        TokenKind::INTEGER_LITERAL | TokenKind::FLOAT_LITERAL => {
            format!("literal '{}'", t.text)
        }
        TokenKind::BOOL_LITERAL | TokenKind::IDENTIFIER | TokenKind::PACKAGE_IDENTIFIER => {
            format!("'{}'", t.text)
        }
        k if k.is_keyword() => format!("keyword '{}'", k.literal()),
        _ => {
            let lit = t.kind.literal();
            if lit.is_empty() {
                format!("'{}'", t.kind.value_str())
            } else {
                format!("'{lit}'")
            }
        }
    }
}

// ---- entry point ----

impl<'a> Parser<'a> {
    /// Parse a whole file: optional `package`, imports, then top-level decls.
    pub fn parse_file(&mut self) -> File {
        let mut package = None;
        let mut package_pos = None;
        let mut imports = Vec::new();
        let mut decls = Vec::new();

        // package decl
        if self.at(TokenKind::PACKAGE) {
            let pkg_tok = self.advance();
            let (name, name_pos) = self.parse_package_name();
            match name {
                Some(n) => {
                    package = Some(n);
                    package_pos = Some(name_pos);
                    // official ParsePackageHeaderEnd: the package header must be
                    // terminated by `;` or `<NL>` (comments/EOF are fine too).
                    // peek() skips NL, so scan the raw token stream here.
                    let mut i = self.pos;
                    let mut term_ok = false;
                    while i < self.tokens.len() {
                        match self.tokens[i].kind {
                            TokenKind::COMMENT => i += 1,
                            TokenKind::NL | TokenKind::SEMI | TokenKind::END => {
                                term_ok = true;
                                break;
                            }
                            _ => break,
                        }
                    }
                    if !term_ok && self.pos < self.tokens.len() {
                        let t = self.peek_token().clone();
                        let found = crate::token_display_text(&t);
                        self.error_id(
                            &t,
                            cj_diag::DiagId::PARSE_EXPECTED_CHARACTER,
                            &["';' or '<NL>'", &found],
                        );
                    }
                }
                None => {
                    // official DiagExpectedIdentifierPackageSpec: `expected a
                    // package name after keyword 'package', found ''`, then
                    // consume the rest of the line (IS_BROKEN) so the garbage
                    // does not cascade into top-level decl errors (007).
                    let t = self.peek_token().clone();
                    let found = crate::token_display_text(&t);
                    self.error_id(
                        &t,
                        cj_diag::DiagId::PARSE_EXPECTED_NAME,
                        &["a package name", "after keyword 'package'", &found],
                    );
                    self.consume_until_nl();
                }
            }
            let _ = pkg_tok;
        }

        // imports (repeatable, terminated by a decl keyword). The spec's
        // importModifier is optional and defaults to private, so `internal
        // import pkg.*` / `public import pkg.*` are legal — peek past access
        // modifiers (without consuming them, in case the next decl starts with
        // a modifier like `public class`) and only consume them when import
        // really follows.
        loop {
            if self.peek_past_import_modifiers() == TokenKind::IMPORT {
                while matches!(
                    self.peek(),
                    TokenKind::PUBLIC
                        | TokenKind::PRIVATE
                        | TokenKind::PROTECTED
                        | TokenKind::INTERNAL
                ) {
                    self.advance();
                }
                if let Some(imp) = self.parse_import() {
                    imports.push(imp);
                }
            } else {
                break;
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
                    // recovery: official DiagExpectedDeclaration(TOPLEVEL) —
                    // parse_expected_decl + top-level note + resync boundary
                    let t = self.peek_token().clone();
                    let found = token_display_text(&t);
                    self.error_id_with_note(
                        &t,
                        cj_diag::DiagId::PARSE_EXPECTED_DECL,
                        &[&found],
                        "only declarations or macro expressions can be used in the top-level",
                    );
                    self.sync_to_decl_boundary();
                }
            }
        }

        let pos = CodePos::default();
        File {
            package,
            package_pos,
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

    /// Consume raw tokens up to and including the next NL token (or END).
    /// Used for official-style recovery that swallows the rest of the line
    /// without diagnosing it (e.g. `let` with an invalid pattern, 012).
    pub fn consume_until_nl(&mut self) {
        while self.pos < self.tokens.len() {
            let k = self.tokens[self.pos].kind;
            self.pos += 1;
            if k == TokenKind::NL || k == TokenKind::END {
                break;
            }
        }
    }

    fn parse_package_name(&mut self) -> (Option<String>, CodePos) {
        let mut parts = Vec::new();
        let mut first = None;
        let mut last_end = None;
        while self.peek() == TokenKind::IDENTIFIER || self.peek() == TokenKind::PACKAGE_IDENTIFIER {
            let t = self.advance();
            if first.is_none() {
                first = Some(t.begin);
            }
            last_end = Some(t.end);
            parts.push(t.text);
            if !self.eat(TokenKind::DOT) {
                break;
            }
        }
        let name = parts.join(".");
        let pos = match (first, last_end) {
            (Some(b), Some(e)) => {
                CodePos::new(b.line, b.column, b.offset, e.line, e.column, e.offset)
            }
            // No name identifiers found (e.g. `package` followed by a literal):
            // anchor at the current token, which sits where the name was expected.
            _ => self.cur_pos(),
        };
        let name = if parts.is_empty() { None } else { Some(name) };
        (name, pos)
    }

    fn parse_import(&mut self) -> Option<ImportSpec> {
        let import_tok = self.expect(TokenKind::IMPORT);
        let mut path = Vec::new();
        let mut glob = false;
        let mut selected = Vec::new();
        // package-name span: first path segment begin .. last path segment end
        let mut name_begin = None;
        let mut name_end = None;
        let mut last_tok_end = import_tok.end;

        // org / module / ... :  or .*
        loop {
            let t = self.peek_token();
            if t.kind == TokenKind::IDENTIFIER || t.kind == TokenKind::PACKAGE_IDENTIFIER {
                if name_begin.is_none() {
                    name_begin = Some(t.begin);
                }
                let seg = self.advance();
                name_end = Some(seg.end);
                last_tok_end = seg.end;
                path.push(seg.text);
            } else if t.kind == TokenKind::MUL {
                // `*` handled below; break if it's after a dot
            } else {
                break;
            }
            if self.eat(TokenKind::DOT) {
                continue;
            }
            break;
        }
        // `import a.b.*`
        if self.at(TokenKind::MUL) {
            let star = self.advance();
            last_tok_end = star.end;
            glob = true;
        } else if self.eat(TokenKind::COLON) {
            // `import a.b: X, Y`
            while self.peek() == TokenKind::IDENTIFIER {
                let s = self.advance();
                last_tok_end = s.end;
                selected.push(s.text);
                if !self.eat(TokenKind::COMMA) {
                    break;
                }
            }
        }
        let pos = CodePos::new(
            import_tok.begin.line,
            import_tok.begin.column,
            import_tok.begin.offset,
            last_tok_end.line,
            last_tok_end.column,
            last_tok_end.offset,
        );
        let name_pos = match (name_begin, name_end) {
            (Some(b), Some(e)) => {
                CodePos::new(b.line, b.column, b.offset, e.line, e.column, e.offset)
            }
            _ => pos,
        };
        Some(ImportSpec {
            path,
            glob,
            selected,
            pos,
            name_pos,
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
