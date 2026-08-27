// cj-lexer: Cangjie lexer implementation.
//
// Behavioral reference: cangjie_compiler/src/Lex/Lexer.cpp (read-only reference,
// rewritten from scratch in Rust — no source copying).
//
// Produces a token stream matching the official token set, including:
//   - integer literals (dec/hex/bin/oct + suffix), float literals (incl. hex float)
//   - rune/byte literals (`r'a'`, `b'a'`), string literals (single/double, multiline, raw)
//   - identifiers (ASCII + Unicode), backquoted (raw) identifiers
//   - operators (multi-char, e.g. `..=`, `**=`, `!in`, `??`)
//   - comments (line/block), newlines, EOF

use crate::token::{lookup_keyword, TokenKind};

/// Position of a token in the source (1-based line/column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: u32,
    pub offset: usize,
}

impl Position {
    pub fn new(line: u32, column: u32, offset: usize) -> Self {
        Position {
            line,
            column,
            offset,
        }
    }
}

/// A single lexical token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// Raw text of the token (for identifiers/literals; empty for pure punctuation).
    pub text: String,
    pub begin: Position,
    pub end: Position,
}

impl Token {
    fn new(kind: TokenKind, text: String, begin: Position, end: Position) -> Self {
        Token {
            kind,
            text,
            begin,
            end,
        }
    }
}

/// A lexical diagnostic (error or warning) with position and message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub message: String,
    pub pos: Position,
    pub is_warning: bool,
}

/// Lexer over an in-memory source string.
pub struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize, // byte offset
    line: u32,
    col: u32,
    pub errors: Vec<LexError>,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Lexer {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            line: 1,
            col: 1,
            errors: Vec::new(),
        }
    }

    /// Lex the whole input, returning all tokens (including NL tokens, ending with END).
    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut out = Vec::new();
        loop {
            let tok = self.next_token();
            let is_end = tok.kind == TokenKind::END;
            out.push(tok);
            if is_end {
                break;
            }
        }
        out
    }

    /// Tokenize `src` and take the lexer errors out as well (the parser/
    /// frontend surface these as diagnostics — official parity for number
    /// suffix errors etc.).
    pub fn tokenize_and_errors(src: &str) -> (Vec<Token>, Vec<LexError>) {
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize();
        let errors = std::mem::take(&mut lexer.errors);
        (tokens, errors)
    }

    // ---- low-level char access ----

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<u8> {
        self.bytes.get(self.pos + 1).copied()
    }

    fn peek3(&self) -> Option<u8> {
        self.bytes.get(self.pos + 2).copied()
    }

    /// Advance one byte, tracking line/column. Newline handling is done by the caller
    /// via `advance_newline` where semantics require it; here we treat '\n' as a
    /// plain byte and advance col (the caller adjusts line when scanning NL tokens).
    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(b)
    }

    fn cur_pos(&self) -> Position {
        Position::new(self.line, self.col, self.pos)
    }

    /// Peek the next Unicode scalar value (None at EOF or on invalid UTF-8).
    /// Returns the char and its byte length.
    fn peek_char(&self) -> Option<(char, usize)> {
        self.src[self.pos..]
            .chars()
            .next()
            .map(|c| (c, c.len_utf8()))
    }

    /// Decode the char starting at `self.pos` and advance past it (UTF-8 aware,
    /// updates column by 1 codepoint — matching official column semantics).
    fn bump_char(&mut self) -> Option<char> {
        let (c, len) = self.peek_char()?;
        self.pos += len;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    /// Position of the byte just before the current position (assumes a single
    /// ASCII byte, e.g. a backslash, was consumed). Used to anchor diagnostics
    /// at the backslash of a `\\u{...}` escape, matching the official `old` pos.
    fn pos_before_backslash(&self) -> Position {
        let col = self.col.saturating_sub(1).max(1);
        Position::new(self.line, col, self.pos.saturating_sub(1))
    }

    // ---- main token loop ----

    pub fn next_token(&mut self) -> Token {
        // Skip whitespace (space, tab, \r) but NOT newline (NL is a real token).
        while let Some(b' ') | Some(b'\t') | Some(b'\r') = self.peek() {
            self.bump();
        }

        let begin = self.cur_pos();
        let Some(c) = self.peek() else {
            return Token::new(TokenKind::END, String::new(), begin, begin);
        };

        match c {
            b'\n' => {
                self.bump();
                let end = self.cur_pos();
                Token::new(TokenKind::NL, "\n".into(), begin, end)
            }
            b'0'..=b'9' => self.scan_number(begin),
            // Context-sensitive: `r'...'` / `b'...'` / `r"..."` are rune/byte/string
            // literals, but a bare `r`/`b` (not followed by a quote) is an identifier.
            b'r' | b'b' if self.peek2() == Some(b'\'') || self.peek2() == Some(b'"') => {
                self.scan_rune_or_string(begin, c)
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' | 0x80.. => self.scan_identifier(begin),
            b'`' => self.scan_backquoted(begin),
            b'\'' | b'"' => self.scan_rune_or_string(begin, c),
            b'#' => self.scan_hash(begin),
            // Leading-dot float: `.` followed by a digit is a number
            // (official ScanNumberOrDotPrefixSymbol). Anything else (.. / ... /
            // member access) stays a symbol.
            b'.' if self.peek2().is_some_and(|c| c.is_ascii_digit()) => self.scan_number(begin),
            _ => self.scan_symbol(begin),
        }
    }

    // ---- identifiers ----

    fn scan_identifier(&mut self, begin: Position) -> Token {
        let start = self.pos;
        // First char: XID_Start or '_' (IsCJXIDStart). ASCII fast path.
        let first_ok = match self.peek() {
            Some(b'a'..=b'z') | Some(b'A'..=b'Z') | Some(b'_') => true,
            Some(b) if b >= 0x80 => self
                .peek_char()
                .is_some_and(|(c, _)| unicode_ident::is_xid_start(c) || c == '_'),
            _ => false,
        };
        if !first_ok {
            // Not an identifier start (e.g. U+FFFD from invalid UTF-8 replacement,
            // or a Unicode char outside XID_Start). Consume one char so the main
            // loop always advances — never return without bumping (avoids hang).
            self.bump_char();
            let text = self.src[start..self.pos].to_string();
            let end = self.cur_pos();
            return Token::new(TokenKind::ILLEGAL, text, begin, end);
        }
        self.bump_char();
        // Continuation: XID_Continue (includes XID_Start, digits, '_', ZWNJ/ZWJ).
        while let Some((c, _)) = self.peek_char() {
            if unicode_ident::is_xid_continue(c) {
                self.bump_char();
            } else {
                break;
            }
        }
        let text = &self.src[start..self.pos];
        let kind = lookup_keyword(text).unwrap_or(TokenKind::IDENTIFIER);
        let end = self.cur_pos();
        Token::new(kind, text.to_string(), begin, end)
    }

    fn scan_backquoted(&mut self, begin: Position) -> Token {
        // `identifier` — raw identifier, e.g. `class` used as a variable name.
        // Content must still be a valid identifier (XID rules), e.g. `a·` / `中文`.
        self.bump(); // consume `
        let start = self.pos;
        // First char: XID_Start or '_'.
        match self.peek_char() {
            Some((c, _)) if unicode_ident::is_xid_start(c) || c == '_' => {
                self.bump_char();
            }
            _ => {
                self.errors.push(LexError {
                    message: "expected identifier after backtick".into(),
                    pos: self.cur_pos(),
                    is_warning: false,
                });
                let text = &self.src[start..self.pos];
                let end = self.cur_pos();
                return Token::new(TokenKind::ILLEGAL, text.to_string(), begin, end);
            }
        }
        while let Some((c, _)) = self.peek_char() {
            if c == '`' {
                break;
            }
            if !unicode_ident::is_xid_continue(c) {
                self.errors.push(LexError {
                    message: format!("invalid character in backquoted identifier: {c:?}"),
                    pos: self.cur_pos(),
                    is_warning: false,
                });
                break;
            }
            self.bump_char();
        }
        let text = &self.src[start..self.pos];
        let closed = self.peek() == Some(b'`');
        if closed {
            self.bump(); // consume closing `
        } else {
            self.errors.push(LexError {
                message: "unterminated backquoted identifier".into(),
                pos: self.cur_pos(),
                is_warning: false,
            });
        }
        let end = self.cur_pos();
        Token::new(TokenKind::IDENTIFIER, text.to_string(), begin, end)
    }

    // ---- number literals ----

    fn scan_number(&mut self, begin: Position) -> Token {
        let start = self.pos;
        // Mirrors official `success`: once a diagnostic is emitted on this
        // number, later secondary diagnostics are suppressed (ProcessDigits /
        // ProcessNumberFloatSuffix gate on `success`).
        let mut failed = false;
        let mut is_float = false;
        let mut token_kind = TokenKind::INTEGER_LITERAL;
        let start_at_dot = self.peek() == Some(b'.');

        // 1. Integer part (skipped for leading-dot floats like `.5`).
        if !start_at_dot {
            // Radix prefix (0x/0o/0b)
            if self.peek() == Some(b'0') {
                let p2 = self.peek2();
                let base = match p2 {
                    Some(b'x') | Some(b'X') => Some(16),
                    Some(b'o') | Some(b'O') => Some(8),
                    Some(b'b') | Some(b'B') => Some(2),
                    _ => None,
                };
                if let Some(b) = base {
                    self.bump(); // 0
                    self.bump(); // x/o/b
                    let digits_start = self.pos;
                    while let Some(c) = self.peek() {
                        let ok = match b {
                            16 => c.is_ascii_hexdigit() || c == b'_',
                            8 => (b'0'..=b'7').contains(&c) || c == b'_',
                            2 => c == b'0' || c == b'1' || c == b'_',
                            _ => false,
                        };
                        if ok {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    if self.pos == digits_start {
                        self.errors.push(LexError {
                            message: "expected digit after radix prefix".into(),
                            pos: begin,
                            is_warning: false,
                        });
                        let text = &self.src[start..self.pos];
                        let end = self.cur_pos();
                        return Token::new(TokenKind::ILLEGAL, text.to_string(), begin, end);
                    }
                    // hex float: 0x1.fp1 — requires BOTH a '.' followed by a hex
                    // digit AND a 'p' exponent to be a float. `0x1.foo` must roll
                    // back to `0x1` + `.` + `foo` (official: isFloat stays false
                    // without exponent, `!isFloat && hasDot` triggers rollback).
                    if b == 16 && self.peek() == Some(b'.') {
                        let after_dot = self.peek2();
                        if after_dot.is_some_and(|c| c.is_ascii_hexdigit()) {
                            // Tentatively consume `.<hexdigits>`.
                            let dot_pos = self.pos;
                            self.bump(); // '.'
                            while self
                                .peek()
                                .is_some_and(|c| c.is_ascii_hexdigit() || c == b'_')
                            {
                                self.bump();
                            }
                            // exponent p/P required for hex float
                            if self.peek().is_some_and(|c| c == b'p' || c == b'P') {
                                self.bump();
                                if self.peek().is_some_and(|c| c == b'+' || c == b'-') {
                                    self.bump();
                                }
                                while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'_') {
                                    self.bump();
                                }
                                let text = &self.src[start..self.pos];
                                let end = self.cur_pos();
                                return Token::new(
                                    TokenKind::FLOAT_LITERAL,
                                    text.to_string(),
                                    begin,
                                    end,
                                );
                            }
                            // No exponent: official rolls back to DOT (token =
                            // integer part only; `.xxx` lexed separately).
                            let consumed = self.pos - dot_pos; // bytes consumed since '.'
                            self.pos = dot_pos;
                            self.col = self.col.saturating_sub(consumed as u32).max(1);
                            let text = &self.src[start..self.pos];
                            let end = self.cur_pos();
                            return Token::new(
                                TokenKind::INTEGER_LITERAL,
                                text.to_string(),
                                begin,
                                end,
                            );
                        }
                    }
                    // Radix integers may carry an integer suffix (0xffu8) or a
                    // stray unknown suffix (0xffzog) — official
                    // ProcessIntegerSuffix / ProcessNumberFloatSuffix handle
                    // both; absorb and diagnose.
                    self.scan_number_suffix(false, &mut token_kind, &mut failed);
                    let text = &self.src[start..self.pos];
                    let end = self.cur_pos();
                    return Token::new(token_kind, text.to_string(), begin, end);
                }
            }

            // Decimal integer part (official ProcessDigits for DEC_BASE):
            // digits/underscores; a hex digit in a decimal literal is an
            // "unexpected digit"; `i`/`u` start an integer-type suffix; `e`/`E`
            // and `p`/`P` break out for the exponent stage below.
            loop {
                match self.peek() {
                    Some(b'0'..=b'9') | Some(b'_') => {
                        self.bump();
                    }
                    Some(c)
                        if c.is_ascii_alphabetic()
                            && !matches!(c, b'e' | b'E' | b'f' | b'F' | b'p' | b'P') =>
                    {
                        if (b'a'..=b'f').contains(&c) || (b'A'..=b'F').contains(&c) {
                            // Hex digit in a decimal literal: official
                            // ProcessXdigit reports "unexpected digit '<c>'".
                            if !failed {
                                let pos = self.cur_pos();
                                self.errors.push(LexError {
                                    message: format!(
                                        "unexpected digit '{}' in decimal, decimal may \
                                         only contain digit within 0~9",
                                        c as char
                                    ),
                                    pos,
                                    is_warning: false,
                                });
                                failed = true;
                            }
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    _ => break,
                }
            }
        }

        // 2. Fractional part: `.` followed by a digit is a float.
        if self.peek() == Some(b'.') && self.peek2().is_some_and(|c| c.is_ascii_digit()) {
            is_float = true;
            token_kind = TokenKind::FLOAT_LITERAL;
            self.bump(); // '.'
            while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'_') {
                self.bump();
            }
        } else if self.peek() == Some(b'.') && !start_at_dot {
            // `1.foo` / `1.` — integer then member access/range: official
            // returns the integer token early (no fractional/suffix scan).
            let text = &self.src[start..self.pos];
            let end = self.cur_pos();
            return Token::new(token_kind, text.to_string(), begin, end);
        }

        // 3. Exponent part: `e`/`E` is valid in decimal; `p`/`P` in decimal is
        // "unexpected exponent part 'p__' in decimal" (official
        // DiagUnexpectedExponentPart) but still switches the token to a float
        // and consumes the exponent. `e` is only treated as an exponent when
        // digits follow (matches the historical lexer behavior for `1e`).
        if let Some(c) = self.peek() {
            let exp = c.to_ascii_lowercase();
            if exp == b'e' || exp == b'p' {
                let can_exp = match self.peek2() {
                    Some(b'+') | Some(b'-') => self.peek3().is_some_and(|c| c.is_ascii_digit()),
                    Some(c2) => c2.is_ascii_digit(),
                    None => false,
                };
                if can_exp || exp == b'p' {
                    if exp == b'p' && !failed {
                        let pos = self.cur_pos();
                        self.errors.push(LexError {
                            message: "unexpected exponent part 'p__' in decimal".into(),
                            pos,
                            is_warning: false,
                        });
                        failed = true;
                    }
                    is_float = true;
                    token_kind = TokenKind::FLOAT_LITERAL;
                    self.bump(); // e/p
                    if self.peek().is_some_and(|c| c == b'+' || c == b'-') {
                        self.bump();
                    }
                    while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'_') {
                        self.bump();
                    }
                }
            }
        }

        // 4. Suffix scan (official ProcessNumberFloatSuffix): always absorb a
        // trailing identifier-ish run into the token; diagnose only if no prior
        // error was reported on this number.
        self.scan_number_suffix(is_float, &mut token_kind, &mut failed);

        let text = &self.src[start..self.pos];
        let end = self.cur_pos();
        Token::new(token_kind, text.to_string(), begin, end)
    }

    /// Absorb a trailing identifier-ish run after a number and diagnose an
    /// invalid suffix (official ProcessNumberFloatSuffix / ProcessIntegerSuffix).
    /// The run is ALWAYS consumed into the token; diagnostics are gated by
    /// `failed` (official `success`).
    ///
    ///   `1f32`/`1.5f64`            -> valid float suffix
    ///   `1foo`/`1.5bar`            -> illegal float number suffix 'foo'
    ///   `1u8`/`1i64` (non-float)   -> valid integer suffix
    ///   `1iagnosticsTest`          -> illegal integer suffix 'iagnosticsTest'
    ///   `1var0052`                 -> unknown suffix 'var0052'
    fn scan_number_suffix(
        &mut self,
        is_float: bool,
        token_kind: &mut TokenKind,
        failed: &mut bool,
    ) {
        let sstart = self.cur_pos();
        // f: float type suffix (official ProcessFloatSuffix).
        if self.peek() == Some(b'f') {
            let mut run = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_alphanumeric() {
                    run.push(c as char);
                    self.bump();
                } else {
                    break;
                }
            }
            if matches!(run.as_str(), "f16" | "f32" | "f64") {
                *token_kind = TokenKind::FLOAT_LITERAL;
            } else {
                // Official ProcessFloatSuffix is not gated by `success`.
                self.errors.push(LexError {
                    message: format!(
                        "illegal float number suffix '{run}', float literal type \
                         suffix can only be 'f16', 'f32', 'f64'"
                    ),
                    pos: sstart,
                    is_warning: false,
                });
                *failed = true;
                *token_kind = TokenKind::ILLEGAL;
            }
            return;
        }
        // Generic trailing run (alnum / underscore; stop at a '.' that begins a
        // member access or a range).
        if !self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return;
        }
        let mut run_len = 0usize;
        while let Some(c) = self.peek() {
            if c == b'.' {
                if self.peek2() == Some(b'.')
                    || self
                        .peek2()
                        .is_some_and(|c2| c2.is_ascii_alphabetic() || c2 == b'_')
                {
                    break;
                }
                self.bump();
                run_len += 1;
            } else if c.is_ascii_alphanumeric() || c == b'_' {
                self.bump();
                run_len += 1;
            } else {
                break;
            }
        }
        if run_len == 0 {
            return;
        }
        let run = &self.src[sstart.offset..self.pos];
        // An integer-type suffix (`i8`/`u64`...) is only valid on a non-float
        // integer literal; otherwise it is an illegal/unknown suffix. Official
        // ProcessIntegerSuffix is NOT gated by `success` — the illegal-integer
        // diag fires even after a prior error on this number (e.g. the
        // unexpected-digit in `1diagnosticsTest`).
        if !is_float {
            if let Some(rest) = run.strip_prefix(['i', 'u']) {
                if matches!(rest, "8" | "16" | "32" | "64") {
                    return; // valid integer suffix
                }
                self.errors.push(LexError {
                    message: format!(
                        "illegal integer suffix '{run}', integer literal type \
                         suffix can only be 'u8', 'u16', 'u32', 'u64', 'i8', 'i16', 'i32', 'i64'"
                    ),
                    pos: sstart,
                    is_warning: false,
                });
                *failed = true;
                *token_kind = TokenKind::ILLEGAL;
                return;
            }
        }
        // Unknown suffix (official lex_unknown_suffix IS gated by `success`).
        if *failed {
            return;
        }
        self.errors.push(LexError {
            message: format!("unknown suffix '{run}' for number literal"),
            pos: sstart,
            is_warning: false,
        });
        *failed = true;
    }
    // ---- rune / byte / string literals ----

    /// Unified entry for rune/byte/string literals.
    /// `c` is the first byte seen at `begin`: `'` = rune, `"` = string,
    /// `r` = rune or string prefix, `b` = byte literal prefix.
    /// Multiline: three identical quotes (''' / \"\"\").
    fn scan_rune_or_string(&mut self, begin: Position, c: u8) -> Token {
        let mut is_byte = false;
        let mut is_rune = c == b'\'';
        // Consume optional prefix char (r / b / J).
        match c {
            b'r' | b'b' => {
                is_byte = c == b'b';
                self.bump(); // consume r/b
            }
            b'J' => {
                self.bump(); // consume J
            }
            _ => {}
        }
        // Now at a quote char (`'` or `"`).
        let quote = self.peek().unwrap_or(b'"');
        let is_double = quote == b'"';
        is_rune = is_rune || !is_double;
        // Multiline: three consecutive identical quotes.
        let multiline = self.peek() == Some(quote) && self.peek2() == Some(quote);
        if multiline {
            self.bump(); // 1st
            self.bump(); // 2nd
            self.bump(); // 3rd
        } else {
            self.bump(); // opening quote
        }

        let mut content = String::new();
        let mut closed = false;
        'scan: while let Some((ch, _)) = self.peek_char() {
            if !multiline && ch == '\n' {
                break; // unterminated (single-line)
            }
            if ch == quote as char {
                if multiline {
                    // closing requires three consecutive quotes
                    if self.peek2() == Some(quote) && self.peek3() == Some(quote) {
                        self.bump();
                        self.bump();
                        self.bump();
                        closed = true;
                        break 'scan;
                    }
                    content.push(ch);
                    self.bump_char();
                } else {
                    self.bump_char();
                    closed = true;
                    break 'scan;
                }
            } else if ch == '\\' {
                self.process_escape(&mut content, is_byte);
            } else {
                content.push(ch);
                self.bump_char();
            }
        }
        if !closed {
            let msg = if multiline {
                "unterminated multiline string literal"
            } else if is_rune {
                "unterminated character literal"
            } else {
                "unterminated string literal"
            };
            self.errors.push(LexError {
                message: msg.into(),
                pos: begin,
                is_warning: false,
            });
        }
        let end = self.cur_pos();
        let kind = if is_rune {
            if is_byte {
                TokenKind::RUNE_BYTE_LITERAL
            } else {
                TokenKind::RUNE_LITERAL
            }
        } else if multiline {
            TokenKind::MULTILINE_STRING
        } else {
            TokenKind::STRING_LITERAL
        };
        Token::new(kind, content, begin, end)
    }

    /// Process an escape sequence starting at `\\` (already positioned on `\\`).
    /// Legal escapes: t b r n f v 0 ' \" \\ (IsLegalEscape). `\\u{...}` is
    /// handled specially with scalar-value range checking.
    fn process_escape(&mut self, content: &mut String, is_byte: bool) {
        self.bump(); // consume backslash
        let Some((ch, _)) = self.peek_char() else {
            content.push('\\');
            return;
        };
        const LEGAL: &[char] = &['t', 'b', 'r', 'n', 'f', 'v', '0', '\'', '\"', '\\'];
        if LEGAL.contains(&ch) || (!is_byte && ch == '$') {
            content.push('\\');
            content.push(ch);
            self.bump_char();
            return;
        }
        if ch == 'u' {
            self.process_unicode_escape(content);
            return;
        }
        self.errors.push(LexError {
            message: format!("unrecognized escape '\\{ch}'"),
            pos: self.cur_pos(),
            is_warning: false,
        });
        content.push('\\');
        content.push(ch);
        self.bump_char();
    }

    /// Process `\\u{...}` unicode escape. Legal scalar range: 0x0..=0xD7FF or
    /// 0xE000..=0x10FFFF (IsLegalUnicode). Off-range values get a diagnostic
    /// whose position points at the `\\` start of the escape (matching official).
    fn process_unicode_escape(&mut self, content: &mut String) {
        // process_escape already consumed the backslash; step back one column so
        // diagnostics point at the `\\` (official: MakeRange(old, ...), old=\\ pos).
        let esc_begin = self.pos_before_backslash();
        self.bump(); // consume 'u'
        let Some((ch, _)) = self.peek_char() else {
            return;
        };
        if ch != '{' {
            self.errors.push(LexError {
                message: format!("expected '{{' in unicode escape, found '{ch}'"),
                pos: esc_begin,
                is_warning: false,
            });
            return;
        }
        self.bump_char(); // consume '{'
        let mut val: u32 = 0;
        let mut digits = 0u32;
        while let Some((h, _)) = self.peek_char() {
            if let Some(d) = h.to_digit(16) {
                val = (val << 4) | d;
                digits += 1;
                if digits > 8 {
                    break;
                }
                self.bump_char();
            } else {
                break;
            }
        }
        if digits == 0 {
            self.errors.push(LexError {
                message: "expected hexadecimal digit in unicode escape".into(),
                pos: esc_begin,
                is_warning: false,
            });
            return;
        }
        let Some((close, _)) = self.peek_char() else {
            return;
        };
        if close != '}' {
            self.errors.push(LexError {
                message: format!(
                    "expected '}}' or hexadecimal digit in unicode escape, found '{close}'"
                ),
                pos: esc_begin,
                is_warning: false,
            });
            return;
        }
        self.bump_char(); // consume '}'
        let legal = val <= 0xD7FF || (0xE000..=0x10FFFF).contains(&val);
        if !legal {
            self.errors.push(LexError {
                message: format!("illegal unicode scalar value '\\u{{{val:x}}}'"),
                pos: esc_begin,
                is_warning: false,
            });
        }
        content.push_str(&format!("\\u{{{val:x}}}"));
    }

    /// `#` — multiline raw string (e.g. `#\"...\"#`). M1 simplified: scan
    /// until a `#` after a quote.
    fn scan_hash(&mut self, begin: Position) -> Token {
        // Count leading '#' (delimiter count).
        let mut hashes = 0usize;
        while self.peek() == Some(b'#') {
            self.bump();
            hashes += 1;
        }
        let content_start = self.pos;
        // Scan until `#` at top level (raw: no escapes, quotes are literal).
        let mut closed = false;
        let mut quote = None;
        while let Some((ch, _)) = self.peek_char() {
            if ch == '"' || ch == '\'' {
                quote = Some(ch);
            }
            if ch == '#' {
                // Raw string closes on '#' when we've seen a quote (delimiter).
                if quote.is_some() {
                    self.bump_char();
                    closed = true;
                    break;
                }
                self.bump_char();
            } else {
                self.bump_char();
            }
        }
        let text = self.src[content_start..self.pos].to_string();
        if !closed {
            self.errors.push(LexError {
                message: "unterminated raw string".into(),
                pos: begin,
                is_warning: false,
            });
        }
        let end = self.cur_pos();
        let _ = hashes;
        Token::new(TokenKind::MULTILINE_RAW_STRING, text, begin, end)
    }
    // ---- symbols / operators ----

    fn scan_symbol(&mut self, begin: Position) -> Token {
        let c = self.peek().expect("scan_symbol at EOF");
        let (kind, len): (TokenKind, usize) = match c {
            b'.' => {
                if self.peek2() == Some(b'.') {
                    if self.peek3() == Some(b'.') {
                        (TokenKind::ELLIPSIS, 3)
                    } else if self.peek3() == Some(b'=') {
                        (TokenKind::CLOSEDRANGEOP, 3)
                    } else {
                        (TokenKind::RANGEOP, 2)
                    }
                } else {
                    (TokenKind::DOT, 1)
                }
            }
            b',' => (TokenKind::COMMA, 1),
            b'(' => (TokenKind::LPAREN, 1),
            b')' => (TokenKind::RPAREN, 1),
            b'[' => (TokenKind::LSQUARE, 1),
            b']' => (TokenKind::RSQUARE, 1),
            b'{' => (TokenKind::LCURL, 1),
            b'}' => (TokenKind::RCURL, 1),
            b';' => (TokenKind::SEMI, 1),
            b':' => {
                if self.peek2() == Some(b':') {
                    (TokenKind::DOUBLE_COLON, 2)
                } else {
                    (TokenKind::COLON, 1)
                }
            }
            b'#' => (TokenKind::HASH, 1),
            b'@' => {
                if self.peek2() == Some(b'!') {
                    (TokenKind::AT_EXCL, 2)
                } else {
                    (TokenKind::AT, 1)
                }
            }
            b'$' => (TokenKind::DOLLAR, 1),
            b'?' => {
                if self.peek2() == Some(b'?') {
                    (TokenKind::COALESCING, 2)
                } else {
                    (TokenKind::QUEST, 1)
                }
            }
            b'~' => {
                if self.peek2() == Some(b'>') {
                    (TokenKind::COMPOSITION, 2)
                } else {
                    (TokenKind::BITNOT, 1)
                }
            }
            b'^' => {
                if self.peek2() == Some(b'=') {
                    (TokenKind::BITXOR_ASSIGN, 2)
                } else {
                    (TokenKind::BITXOR, 1)
                }
            }
            b'&' => match self.peek2() {
                Some(b'&') => {
                    if self.peek3() == Some(b'=') {
                        (TokenKind::AND_ASSIGN, 3)
                    } else {
                        (TokenKind::AND, 2)
                    }
                }
                Some(b'=') => (TokenKind::BITAND_ASSIGN, 2),
                _ => (TokenKind::BITAND, 1),
            },
            b'|' => match self.peek2() {
                Some(b'|') => {
                    if self.peek3() == Some(b'=') {
                        (TokenKind::OR_ASSIGN, 3)
                    } else {
                        (TokenKind::OR, 2)
                    }
                }
                Some(b'=') => (TokenKind::BITOR_ASSIGN, 2),
                Some(b'>') => (TokenKind::PIPELINE, 2),
                _ => (TokenKind::BITOR, 1),
            },
            b'!' => match self.peek2() {
                Some(b'=') => (TokenKind::NOTEQ, 2),
                Some(b'i') if self.peek3() == Some(b'n') => (TokenKind::NOT_IN, 3),
                _ => (TokenKind::NOT, 1),
            },
            b'<' => match self.peek2() {
                Some(b'=') => (TokenKind::LE, 2),
                Some(b':') => (TokenKind::UPPERBOUND, 2),
                Some(b'-') => (TokenKind::BACKARROW, 2),
                Some(b'<') => {
                    if self.peek3() == Some(b'=') {
                        (TokenKind::LSHIFT_ASSIGN, 3)
                    } else {
                        (TokenKind::LSHIFT, 2)
                    }
                }
                _ => (TokenKind::LT, 1),
            },
            b'>' => match self.peek2() {
                Some(b'=') => (TokenKind::GE, 2),
                Some(b'>') => {
                    if self.peek3() == Some(b'=') {
                        (TokenKind::RSHIFT_ASSIGN, 3)
                    } else {
                        (TokenKind::RSHIFT, 2)
                    }
                }
                _ => (TokenKind::GT, 1),
            },
            b'=' => match self.peek2() {
                Some(b'=') => (TokenKind::EQUAL, 2),
                Some(b'>') => (TokenKind::DOUBLE_ARROW, 2),
                _ => (TokenKind::ASSIGN, 1),
            },
            b'+' => match self.peek2() {
                Some(b'+') => (TokenKind::INCR, 2),
                Some(b'=') => (TokenKind::ADD_ASSIGN, 2),
                _ => (TokenKind::ADD, 1),
            },
            b'-' => match self.peek2() {
                Some(b'-') => (TokenKind::DECR, 2),
                Some(b'=') => (TokenKind::SUB_ASSIGN, 2),
                Some(b'>') => (TokenKind::ARROW, 2),
                _ => (TokenKind::SUB, 1),
            },
            b'*' => match self.peek2() {
                Some(b'*') => {
                    if self.peek3() == Some(b'=') {
                        (TokenKind::EXP_ASSIGN, 3)
                    } else {
                        (TokenKind::EXP, 2)
                    }
                }
                Some(b'=') => (TokenKind::MUL_ASSIGN, 2),
                _ => (TokenKind::MUL, 1),
            },
            b'/' => match self.peek2() {
                Some(b'/') => return self.scan_line_comment(begin),
                Some(b'*') => return self.scan_block_comment(begin),
                Some(b'=') => (TokenKind::DIV_ASSIGN, 2),
                _ => (TokenKind::DIV, 1),
            },
            b'%' => match self.peek2() {
                Some(b'=') => (TokenKind::MOD_ASSIGN, 2),
                _ => (TokenKind::MOD, 1),
            },
            _ => {
                self.bump();
                let end = self.cur_pos();
                return Token::new(TokenKind::ILLEGAL, (c as char).to_string(), begin, end);
            }
        };

        let start = self.pos;
        for _ in 0..len {
            self.bump();
        }
        let text = &self.src[start..start + len];
        let end = self.cur_pos();
        Token::new(kind, text.to_string(), begin, end)
    }

    fn scan_line_comment(&mut self, begin: Position) -> Token {
        self.bump(); // /
        self.bump(); // /
        while let Some(c) = self.peek() {
            if c == b'\n' {
                break;
            }
            self.bump();
        }
        let end = self.cur_pos();
        Token::new(TokenKind::COMMENT, String::new(), begin, end)
    }

    fn scan_block_comment(&mut self, begin: Position) -> Token {
        self.bump(); // /
        self.bump(); // *
        let mut depth = 1usize;
        while depth > 0 {
            match self.peek() {
                None => break,
                Some(b'*') if self.peek2() == Some(b'/') => {
                    self.bump();
                    self.bump();
                    depth -= 1;
                }
                Some(b'/') if self.peek2() == Some(b'*') => {
                    self.bump();
                    self.bump();
                    depth += 1;
                }
                _ => {
                    self.bump();
                }
            }
        }
        let end = self.cur_pos();
        Token::new(TokenKind::COMMENT, String::new(), begin, end)
    }
}

/// Convenience: tokenize a source string, returning only non-comment, non-NL tokens.
pub fn lex_all(src: &str) -> Vec<Token> {
    let mut lexer = Lexer::new(src);
    lexer
        .tokenize()
        .into_iter()
        .filter(|t| {
            t.kind != TokenKind::COMMENT && t.kind != TokenKind::NL && t.kind != TokenKind::END
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex_all(src).into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn basic_tokens() {
        assert_eq!(
            kinds("let x: Int64 = 42"),
            vec![
                TokenKind::LET,
                TokenKind::IDENTIFIER,
                TokenKind::COLON,
                TokenKind::INT64,
                TokenKind::ASSIGN,
                TokenKind::INTEGER_LITERAL,
            ]
        );
    }

    #[test]
    fn operators_multi_char() {
        assert_eq!(
            kinds("a <= b ..= c **= d ?? e"),
            vec![
                TokenKind::IDENTIFIER,
                TokenKind::LE,
                TokenKind::IDENTIFIER,
                TokenKind::CLOSEDRANGEOP,
                TokenKind::IDENTIFIER,
                TokenKind::EXP_ASSIGN,
                TokenKind::IDENTIFIER,
                TokenKind::COALESCING,
                TokenKind::IDENTIFIER,
            ]
        );
    }

    #[test]
    fn ellipsis_vs_range() {
        assert_eq!(kinds("..."), vec![TokenKind::ELLIPSIS]);
        assert_eq!(kinds(".."), vec![TokenKind::RANGEOP]);
        assert_eq!(kinds("..="), vec![TokenKind::CLOSEDRANGEOP]);
    }

    #[test]
    fn not_in_is_three_chars() {
        assert_eq!(kinds("!in"), vec![TokenKind::NOT_IN]);
        assert_eq!(kinds("!="), vec![TokenKind::NOTEQ]);
        assert_eq!(kinds("!"), vec![TokenKind::NOT]);
    }

    #[test]
    fn hex_float_boundary() {
        // 0x1.foo -> 0x1. is an (invalid) float attempt; foo is an identifier.
        // Our M1 lexer: 0x1. followed by non-hex digit 'f'? 'f' IS a hex digit, so
        // it consumes .foo entirely as float digits. This matches official behavior
        // where 0x1.foo gives "undeclared identifier 'foo'" — meaning official lexes
        // `0x1.` as float and `foo` separately. We refine in M1-iteration 2.
        let t = kinds("0x1.foo");
        // Current behavior: [FLOAT_LITERAL] consuming "0x1.foo"; refinement pending.
        assert!(!t.is_empty());
    }

    #[test]
    fn comments_skipped() {
        let t = kinds("let x = 1 // comment\n/* block */ var y = 2");
        assert_eq!(
            t,
            vec![
                TokenKind::LET,
                TokenKind::IDENTIFIER,
                TokenKind::ASSIGN,
                TokenKind::INTEGER_LITERAL,
                TokenKind::VAR,
                TokenKind::IDENTIFIER,
                TokenKind::ASSIGN,
                TokenKind::INTEGER_LITERAL,
            ]
        );
    }

    #[test]
    fn identifiers_and_keywords() {
        assert_eq!(
            kinds("class Foo"),
            vec![TokenKind::CLASS, TokenKind::IDENTIFIER]
        );
        assert_eq!(
            kinds("match x"),
            vec![TokenKind::MATCH, TokenKind::IDENTIFIER]
        );
        assert_eq!(kinds("foo_bar"), vec![TokenKind::IDENTIFIER]);
        assert_eq!(kinds("_"), vec![TokenKind::WILDCARD]);
    }

    #[test]
    fn backquoted_identifier() {
        let t = lex_all("let `class` = 1");
        assert_eq!(t[1].kind, TokenKind::IDENTIFIER);
        assert_eq!(t[1].text, "class");
    }

    #[test]
    fn unicode_identifiers_xid_rules() {
        // U+00B7 MIDDLE DOT = XID_Continue (not XID_Start): valid inside, invalid first.
        assert_eq!(
            kinds("var _· = 1"),
            vec![
                TokenKind::VAR,
                TokenKind::IDENTIFIER,
                TokenKind::ASSIGN,
                TokenKind::INTEGER_LITERAL,
            ]
        );
        assert_eq!(
            kinds("var a· = 1"),
            vec![
                TokenKind::VAR,
                TokenKind::IDENTIFIER,
                TokenKind::ASSIGN,
                TokenKind::INTEGER_LITERAL,
            ]
        );
        // U+00BA FEMININE ORDINAL = XID_Start: valid as first char.
        assert_eq!(
            kinds("var ºº = 1"),
            vec![
                TokenKind::VAR,
                TokenKind::IDENTIFIER,
                TokenKind::ASSIGN,
                TokenKind::INTEGER_LITERAL,
            ]
        );
        assert_eq!(
            kinds("var 中文 = 1"),
            vec![
                TokenKind::VAR,
                TokenKind::IDENTIFIER,
                TokenKind::ASSIGN,
                TokenKind::INTEGER_LITERAL,
            ]
        );
        assert_eq!(
            kinds("var _élève__扣_ = 1"),
            vec![
                TokenKind::VAR,
                TokenKind::IDENTIFIER,
                TokenKind::ASSIGN,
                TokenKind::INTEGER_LITERAL,
            ]
        );
        assert_eq!(
            kinds("var _1_allowed = 0"),
            vec![
                TokenKind::VAR,
                TokenKind::IDENTIFIER,
                TokenKind::ASSIGN,
                TokenKind::INTEGER_LITERAL,
            ]
        );
    }

    #[test]
    fn unicode_backquoted_identifier() {
        // Backquoted identifiers must still satisfy XID rules: `中文`, `a·`.
        let t = lex_all("let `中文` = 1");
        assert_eq!(t[1].kind, TokenKind::IDENTIFIER);
        assert_eq!(t[1].text, "中文");
        let t2 = lex_all("var `a·` = 4");
        assert_eq!(t2[1].kind, TokenKind::IDENTIFIER);
        assert_eq!(t2[1].text, "a·");
    }

    #[test]
    fn rune_and_byte_prefixes() {
        // r'a' is a rune literal (char_literal); the `r` prefix is consumed.
        let t = lex_all("let x = r'a'");
        assert_eq!(t[3].kind, TokenKind::RUNE_LITERAL);
        assert_eq!(t[3].text, "a");
        // b'a' is a byte literal.
        let t2 = lex_all("let y = b'a'");
        assert_eq!(t2[3].kind, TokenKind::RUNE_BYTE_LITERAL);
        // Bare `r`/`b` not followed by a quote stays an identifier.
        assert_eq!(
            kinds("let r = 1"),
            vec![
                TokenKind::LET,
                TokenKind::IDENTIFIER,
                TokenKind::ASSIGN,
                TokenKind::INTEGER_LITERAL,
            ]
        );
        assert_eq!(
            kinds("var range = 1"),
            vec![
                TokenKind::VAR,
                TokenKind::IDENTIFIER,
                TokenKind::ASSIGN,
                TokenKind::INTEGER_LITERAL,
            ]
        );
    }

    #[test]
    fn string_literals_basic() {
        let t = lex_all(r#"let s = "hello""#);
        assert_eq!(t[3].kind, TokenKind::STRING_LITERAL);
        assert_eq!(t[3].text, "hello");
        // r"..." string form.
        let t2 = lex_all(r#"let s = r"raw""#);
        assert_eq!(t2[3].kind, TokenKind::STRING_LITERAL);
        assert_eq!(t2[3].text, "raw");
    }

    #[test]
    fn unicode_escape_illegal_scalar() {
        // \\u{D800} is a surrogate (illegal scalar); error recorded, position
        // points at the backslash (col 15 in the full source).
        let mut lx = Lexer::new("let s = r'\\u{D800}'");
        lx.tokenize();
        assert_eq!(lx.errors.len(), 1);
        assert!(lx.errors[0]
            .message
            .contains("illegal unicode scalar value"));
        assert_eq!(lx.errors[0].pos.column, 11);
        // \\u{10ffff} is legal — no error.
        let mut lx2 = Lexer::new("let s = r'\\u{10ffff}'");
        lx2.tokenize();
        assert_eq!(lx2.errors.len(), 0);
    }

    #[test]
    fn multiline_string() {
        let src = "\"\"\"line1\nline2\"\"\"";
        let t = lex_all(src);
        assert_eq!(t[0].kind, TokenKind::MULTILINE_STRING);
    }
}

#[cfg(test)]
mod t15_probe {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        let mut lx = Lexer::new(src);
        lx.tokenize()
            .into_iter()
            .filter(|t| t.kind != TokenKind::END && t.kind != TokenKind::NL)
            .map(|t| t.kind)
            .collect()
    }

    fn lex_errors(src: &str) -> Vec<(String, u32, u32)> {
        let mut lx = Lexer::new(src);
        let toks = lx.tokenize();
        let _ = toks;
        lx.errors
            .iter()
            .map(|e| (e.message.clone(), e.pos.line, e.pos.column))
            .collect()
    }

    #[test]
    fn probe_012_unknown_suffix() {
        // let 1var0052 = 'x'
        let toks = kinds("let 1var0052 = 'x'");
        assert_eq!(
            toks,
            vec![
                TokenKind::LET,
                TokenKind::INTEGER_LITERAL,
                TokenKind::ASSIGN,
                TokenKind::RUNE_LITERAL
            ]
        );
        let errs = lex_errors("let 1var0052 = 'x'");
        assert_eq!(
            errs,
            vec![(
                "unknown suffix 'var0052' for number literal".to_string(),
                1,
                6
            )]
        );
    }

    #[test]
    fn probe_007_illegal_suffix() {
        let errs = lex_errors("package 1diagnosticsTest.pkg_error");
        assert_eq!(
            errs[0].0,
            "unexpected digit 'd' in decimal, decimal may only contain digit within 0~9"
        );
        assert_eq!(errs[0].1, 1);
        assert_eq!(errs[0].2, 10);
        assert_eq!(errs[1].0, "illegal integer suffix 'iagnosticsTest', integer literal type suffix can only be 'u8', 'u16', 'u32', 'u64', 'i8', 'i16', 'i32', 'i64'");
        assert_eq!(errs[1].1, 1);
        assert_eq!(errs[1].2, 11);
        // token should be a single ILLEGAL
        let toks = kinds("package 1diagnosticsTest.pkg_error");
        assert_eq!(toks[1], TokenKind::ILLEGAL);
    }

    #[test]
    fn probe_008_exponent() {
        let errs = lex_errors("package diagnosticsTest.1pkg_error");
        assert_eq!(errs[0].0, "unexpected exponent part 'p__' in decimal");
        assert_eq!(errs[0].1, 1);
        assert_eq!(errs[0].2, 26);
        let toks = kinds("package diagnosticsTest.1pkg_error");
        assert_eq!(toks[1], TokenKind::IDENTIFIER);
        assert_eq!(toks[2], TokenKind::FLOAT_LITERAL);
    }

    #[test]
    fn probe_valid_suffixes_still_work() {
        assert_eq!(kinds("1u8"), vec![TokenKind::INTEGER_LITERAL]);
        assert_eq!(kinds("0xffu8"), vec![TokenKind::INTEGER_LITERAL]);
        assert_eq!(kinds("1.5f32"), vec![TokenKind::FLOAT_LITERAL]);
        assert_eq!(kinds("1.5e3"), vec![TokenKind::FLOAT_LITERAL]);
        assert_eq!(
            kinds("1.foo"),
            vec![
                TokenKind::INTEGER_LITERAL,
                TokenKind::DOT,
                TokenKind::IDENTIFIER
            ]
        );
        assert_eq!(
            kinds("1..2"),
            vec![
                TokenKind::INTEGER_LITERAL,
                TokenKind::RANGEOP,
                TokenKind::INTEGER_LITERAL
            ]
        );
        assert_eq!(kinds(".5"), vec![TokenKind::FLOAT_LITERAL]);
    }
}
