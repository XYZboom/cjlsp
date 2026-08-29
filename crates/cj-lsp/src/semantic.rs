// cj-lsp: semantic tokens (syntax highlighting) for textDocument/semanticTokens/full.
//
// LSP semantic tokens are an encoded flat array: 5 ints per token
// [deltaLine, deltaStartChar, length, tokenType, tokenModifiers] with
// relative positions. The legend is fixed in server.rs initialize:
//   tokenTypes:  namespace,type,class,enum,interface,struct,typeParameter,
//                parameter,variable,property,enumMember,event,function,
//                method,macro,keyword,modifier,comment,string,number,
//                regexp,operator,member,label
//   tokenModifiers: declaration,definition,readonly,static,deprecated,
//                abstract,async,modification,documentation,defaultLibrary
//
// Strategy: classify the lexer token stream first (keywords/strings/numbers/
// comments/operators are self-evident), then overlay the parsed AST so
// identifier tokens that name declarations get the richer type
// (class/struct/func/method/type), and name usages get member/type where
// derivable.

use cj_ast::{Decl, File};
use cj_lexer::{Lexer, Token, TokenKind};

/// Legend indices matching the server's initialize tokenTypes order.
mod ty {
    pub const NAMESPACE: u32 = 0;
    pub const TYPE: u32 = 1;
    pub const CLASS: u32 = 2;
    pub const ENUM: u32 = 3;
    pub const INTERFACE: u32 = 4;
    pub const STRUCT: u32 = 5;
    pub const TYPE_PARAMETER: u32 = 6;
    pub const PARAMETER: u32 = 7;
    pub const VARIABLE: u32 = 8;
    pub const PROPERTY: u32 = 9;
    pub const ENUM_MEMBER: u32 = 10;
    pub const FUNCTION: u32 = 12;
    pub const METHOD: u32 = 13;
    pub const MACRO: u32 = 14;
    pub const KEYWORD: u32 = 15;
    pub const MODIFIER: u32 = 16;
    pub const COMMENT: u32 = 17;
    pub const STRING: u32 = 18;
    pub const NUMBER: u32 = 19;
    pub const OPERATOR: u32 = 21;
    pub const MEMBER: u32 = 22;
}

/// A single semantic token in absolute (line, col) coordinates before
/// delta-encoding.
struct Sem {
    line: u32,
    col: u32, // byte column (LSP char = utf16, but cj is mostly ASCII)
    len: u32,
    typ: u32,
    mods: u32,
}

/// Produce the LSP semanticTokens/full `data` array for `src`.
pub fn semantic_tokens_full(src: &str, file: &File) -> Vec<u32> {
    let tokens = Lexer::new(src).tokenize();
    // 1. Base pass: classify every token from its kind.
    let mut sems: Vec<Sem> = Vec::new();
    for t in &tokens {
        if let Some(sem) = classify_token(t) {
            sems.push(sem);
        }
    }
    // 2. Overlay AST: declaration names get precise types.
    for d in &file.decls {
        overlay_decl(d, &mut sems);
    }
    // 3. Delta-encode.
    encode(sems)
}

/// Classify a single token purely from its kind. Returns None for tokens
/// that should not be highlighted (whitespace already skipped by lexer).
fn classify_token(t: &Token) -> Option<Sem> {
    let k = t.kind;
    // Comments.
    if k == TokenKind::COMMENT {
        return Some(sem(t, ty::COMMENT, 0));
    }
    // Strings (incl. raw / multiline / jstring).
    if matches!(
        k,
        TokenKind::STRING_LITERAL
            | TokenKind::JSTRING_LITERAL
            | TokenKind::MULTILINE_STRING
            | TokenKind::MULTILINE_RAW_STRING
    ) {
        return Some(sem(t, ty::STRING, 0));
    }
    // Numbers.
    if matches!(
        k,
        TokenKind::INTEGER_LITERAL | TokenKind::FLOAT_LITERAL | TokenKind::RUNE_BYTE_LITERAL
    ) {
        return Some(sem(t, ty::NUMBER, 0));
    }
    // Keywords (control flow etc.) and modifiers (visibility).
    if k.is_keyword() {
        if matches!(
            k,
            TokenKind::PUBLIC
                | TokenKind::PRIVATE
                | TokenKind::PROTECTED
                | TokenKind::INTERNAL
                | TokenKind::STATIC
                | TokenKind::ABSTRACT
                | TokenKind::OPEN
                | TokenKind::SEALED
                | TokenKind::MUT
                | TokenKind::OVERRIDE
                | TokenKind::FOREIGN
                | TokenKind::REDEF
                | TokenKind::CONST
        ) {
            return Some(sem(t, ty::MODIFIER, 0));
        }
        return Some(sem(t, ty::KEYWORD, 0));
    }
    // Operators / punctuation.
    if k.operator_like() || k.symbol_like() {
        return Some(sem(t, ty::OPERATOR, 0));
    }
    // Identifiers / name-like tokens: base type is variable; the AST overlay
    // refines declarations and type/member usage.
    if k.is_name_like() {
        return Some(sem(t, ty::VARIABLE, 0));
    }
    None
}

/// Overlay a declaration: mark its name token with a precise type.
fn overlay_decl(d: &Decl, sems: &mut Vec<Sem>) {
    let (name, name_pos, typ) = match d {
        Decl::Class { name, name_pos, .. } => (name, *name_pos, ty::CLASS),
        Decl::Struct { name, name_pos, .. } => (name, *name_pos, ty::STRUCT),
        Decl::Interface { name, name_pos, .. } => (name, *name_pos, ty::INTERFACE),
        Decl::Enum { name, name_pos, .. } => (name, *name_pos, ty::ENUM),
        Decl::Func { name, name_pos, .. } => (name, *name_pos, ty::FUNCTION),
        Decl::Var { name, name_pos, .. } => (name, *name_pos, ty::VARIABLE),
        Decl::Prop { name, pos, .. } => (name, *pos, ty::PROPERTY),
        _ => return,
    };
    refine(
        sems,
        name_pos.line.saturating_sub(1),
        name_pos.col.saturating_sub(1),
        name.len() as u32,
        typ,
    );
}

/// Find a semantic token at the given position and bump its type + mark
/// declaration modifier. Falls back to pushing a new token.
fn refine(sems: &mut Vec<Sem>, line: u32, col: u32, len: u32, typ: u32) {
    if let Some(s) = sems.iter_mut().find(|s| s.line == line && s.col == col) {
        s.typ = typ;
        s.mods |= 1; // declaration
    } else {
        sems.push(Sem {
            line,
            col,
            len,
            typ,
            mods: 1,
        });
    }
}

fn sem(t: &Token, typ: u32, mods: u32) -> Sem {
    Sem {
        line: t.begin.line.saturating_sub(1),
        col: t.begin.column.saturating_sub(1),
        len: (t.end.offset - t.begin.offset) as u32,
        typ,
        mods,
    }
}

/// Delta-encode a sorted list of absolute-position tokens into the LSP
/// 5-int-per-token array.
fn encode(mut sems: Vec<Sem>) -> Vec<u32> {
    sems.sort_by_key(|s| (s.line, s.col));
    let mut out = Vec::with_capacity(sems.len() * 5);
    let (mut prev_line, mut prev_col) = (0u32, 0u32);
    for s in sems {
        let d_line = s.line - prev_line;
        let d_col = if d_line == 0 { s.col - prev_col } else { s.col };
        out.push(d_line);
        out.push(d_col);
        out.push(s.len);
        out.push(s.typ);
        out.push(s.mods);
        prev_line = s.line;
        prev_col = s.col;
    }
    out
}
