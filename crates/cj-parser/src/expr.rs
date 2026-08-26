// cj-parser: expression parsing (Pratt / precedence-climbing).
//
// Reference: cangjie_compiler/src/Parse/ParseExpr.cpp — the Pratt loop
// `ParseExpr(preT, expr)` where preP > curP stops, preP == curP with
// assignment/coalescing/exponent is right-associative.

use super::Parser;
use cj_ast::{AssignOp, BinOp, Expr, LitKind, Pattern, UnOp};
use cj_lexer::TokenKind;

/// Parse an expression at the current token.
pub fn parse_expr(p: &mut Parser) -> Expr {
    parse_expr_prec(p, 0)
}

/// Parse an expression with a minimum precedence floor (0 = any).
pub fn parse_expr_prec(p: &mut Parser, min_prec: u8) -> Expr {
    let mut lhs = parse_unary(p);
    loop {
        let tok_kind = p.peek();
        let prec = tok_kind.precedence();
        if prec == 0 || prec < min_prec {
            break;
        }
        // Handle assignment operators specially (right-assoc, precedence 0).
        if let Some(op) = assign_op_from_token(tok_kind) {
            // assignment binds looser than any binary op; only at min_prec == 0
            if min_prec > 0 {
                break;
            }
            let assign_tok = p.advance();
            let rhs = parse_expr_prec(p, 0);
            let pos = cj_ast::CodePos::new(
                assign_tok.begin.line,
                assign_tok.begin.column,
                assign_tok.begin.offset,
                assign_tok.end.line,
                assign_tok.end.column,
                assign_tok.end.offset,
            );
            lhs = Expr::Assign {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                pos,
            };
            continue;
        }
        let op = match bin_op_from_token(tok_kind) {
            Some(o) => o,
            None => break,
        };
        // For left-assoc: only consume if prec >= min_prec (Pratt: preP > curP stop).
        // For right-assoc (??, **): rhs parsed with min_prec = prec (same level).
        let tok = p.advance();
        let is_right_assoc = tok_kind == TokenKind::COALESCING || tok_kind == TokenKind::EXP;
        let next_min = if is_right_assoc { prec } else { prec + 1 };
        let rhs = parse_expr_prec(p, next_min);
        let pos = cj_ast::CodePos::new(
            tok.begin.line,
            tok.begin.column,
            tok.begin.offset,
            tok.end.line,
            tok.end.column,
            tok.end.offset,
        );
        lhs = Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            pos,
        };
    }
    lhs
}

/// Parse a unary-prefix expression then fall through to postfix.
fn parse_unary(p: &mut Parser) -> Expr {
    match p.peek() {
        TokenKind::SUB => {
            let tok = p.advance();
            let inner = parse_unary(p);
            return Expr::Unary {
                op: UnOp::Neg,
                inner: Box::new(inner),
                pos: pos_of(&tok),
            };
        }
        TokenKind::ADD => {
            let tok = p.advance();
            let inner = parse_unary(p);
            return Expr::Unary {
                op: UnOp::Pos,
                inner: Box::new(inner),
                pos: pos_of(&tok),
            };
        }
        TokenKind::NOT => {
            let tok = p.advance();
            let inner = parse_unary(p);
            return Expr::Unary {
                op: UnOp::Not,
                inner: Box::new(inner),
                pos: pos_of(&tok),
            };
        }
        TokenKind::BITNOT => {
            let tok = p.advance();
            let inner = parse_unary(p);
            return Expr::Unary {
                op: UnOp::BitNot,
                inner: Box::new(inner),
                pos: pos_of(&tok),
            };
        }
        _ => {}
    }
    parse_postfix(p)
}

/// Parse an atom then postfix chains (call / member / index / ...).
fn parse_postfix(p: &mut Parser) -> Expr {
    let mut e = parse_atom(p);
    loop {
        match p.peek() {
            TokenKind::LPAREN => {
                // Call
                let lparen = p.advance();
                let mut args = Vec::new();
                while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
                    let arg_name = if p.peek() == TokenKind::IDENTIFIER
                        && p.peek_ahead(1) == TokenKind::COLON
                    {
                        let name = p.advance().text;
                        p.advance(); // colon
                        Some(name)
                    } else {
                        None
                    };
                    let value = parse_expr_prec(p, 1);
                    args.push(cj_ast::FuncArg {
                        name: arg_name,
                        value,
                        pos: cj_ast::CodePos::default(),
                    });
                    if !p.eat(TokenKind::COMMA) {
                        break;
                    }
                }
                let _ = p.expect(TokenKind::RPAREN);
                let pos = pos_of(&lparen);
                e = Expr::Call {
                    callee: Box::new(e),
                    args,
                    type_args: Vec::new(),
                    pos,
                };
            }
            TokenKind::DOT => {
                let dot = p.advance();
                let name_tok = p.peek_token().clone();
                if name_tok.kind == TokenKind::IDENTIFIER || name_tok.kind.is_identifier_like() {
                    let name = p.advance().text;
                    let pos = pos_of(&dot);
                    e = Expr::Member {
                        object: Box::new(e),
                        name,
                        pos,
                    };
                } else {
                    let found = crate::token_display_text(name_tok.kind);
                    p.error_id(
                        &name_tok,
                        cj_diag::DiagId::PARSE_EXPECTED_NAME,
                        &["member", "name", &found],
                    );
                    break;
                }
            }
            TokenKind::LSQUARE => {
                let lb = p.advance();
                let idx = parse_expr_prec(p, 1);
                let _ = p.expect(TokenKind::RSQUARE);
                let pos = pos_of(&lb);
                e = Expr::Subscript {
                    object: Box::new(e),
                    index: Box::new(idx),
                    pos,
                };
            }
            _ => break,
        }
    }
    e
}

/// Parse a primary expression (atom).
fn parse_atom(p: &mut Parser) -> Expr {
    let tok = p.peek_token().clone();
    match tok.kind {
        TokenKind::INTEGER_LITERAL | TokenKind::FLOAT_LITERAL => {
            p.advance();
            let pos = pos_of(&tok);
            // preserve raw text; lit kind refined by suffix
            let kind = if tok.kind == TokenKind::FLOAT_LITERAL {
                LitKind::Float
            } else {
                LitKind::Integer
            };
            Expr::Lit {
                kind,
                value: tok.text.clone(),
                pos,
            }
        }
        TokenKind::STRING_LITERAL | TokenKind::MULTILINE_STRING => {
            p.advance();
            let pos = pos_of(&tok);
            Expr::Lit {
                kind: LitKind::String,
                value: tok.text.clone(),
                pos,
            }
        }
        TokenKind::RUNE_LITERAL | TokenKind::RUNE_BYTE_LITERAL => {
            p.advance();
            let pos = pos_of(&tok);
            let kind = if tok.kind == TokenKind::RUNE_BYTE_LITERAL {
                LitKind::RuneByte
            } else {
                LitKind::Rune
            };
            Expr::Lit {
                kind,
                value: tok.text.clone(),
                pos,
            }
        }
        TokenKind::BOOL_LITERAL => {
            p.advance();
            let pos = pos_of(&tok);
            Expr::Lit {
                kind: LitKind::Bool,
                value: tok.text.clone(),
                pos,
            }
        }
        TokenKind::IDENTIFIER => {
            p.advance();
            let pos = pos_of(&tok);
            Expr::Name {
                name: tok.text.clone(),
                type_args: Vec::new(),
                pos,
            }
        }
        TokenKind::LPAREN => {
            p.advance();
            // tuple or parenthesized expr
            let first = parse_expr_prec(p, 1);
            if p.eat(TokenKind::COMMA) {
                let mut elems = vec![first];
                while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
                    elems.push(parse_expr_prec(p, 1));
                    if !p.eat(TokenKind::COMMA) {
                        break;
                    }
                }
                let _ = p.expect(TokenKind::RPAREN);
                let pos = pos_of(&tok);
                Expr::Tuple {
                    elements: elems,
                    pos,
                }
            } else {
                let _ = p.expect(TokenKind::RPAREN);
                let pos = pos_of(&tok);
                Expr::Paren {
                    inner: Box::new(first),
                    pos,
                }
            }
        }
        TokenKind::LSQUARE => {
            p.advance();
            let mut elems = Vec::new();
            while !p.at(TokenKind::RSQUARE) && !p.at(TokenKind::END) {
                elems.push(parse_expr_prec(p, 1));
                if !p.eat(TokenKind::COMMA) {
                    break;
                }
            }
            let _ = p.expect(TokenKind::RSQUARE);
            let pos = pos_of(&tok);
            Expr::ArrayLit {
                elements: elems,
                pos,
            }
        }
        TokenKind::LCURL => parse_block_expr(p),
        TokenKind::LET | TokenKind::VAR => {
            // `let pattern = expr` — LetPatternDestructor (a statement-like expr)
            p.advance();
            let pattern = parse_pattern(p);
            let init = if p.eat(TokenKind::ASSIGN) {
                parse_expr_prec(p, 1)
            } else {
                Expr::Lit {
                    kind: LitKind::Unit,
                    value: String::new(),
                    pos: pos_of(&tok),
                }
            };
            let pos = pos_of(&tok);
            Expr::LetPatternDestructor {
                patterns: vec![pattern],
                initializer: Box::new(init),
                pos,
            }
        }
        TokenKind::IF => parse_if_expr(p),
        TokenKind::WHILE => parse_while_expr(p),
        TokenKind::FOR => parse_for_in(p),
        TokenKind::MATCH => parse_match(p),
        TokenKind::RETURN => {
            p.advance();
            let pos = pos_of(&tok);
            let value = if is_expr_start(p.peek()) {
                Some(Box::new(parse_expr_prec(p, 1)))
            } else {
                None
            };
            Expr::Return { value, pos }
        }
        TokenKind::BREAK => {
            p.advance();
            let pos = pos_of(&tok);
            Expr::Jump {
                is_break: true,
                pos,
            }
        }
        TokenKind::CONTINUE => {
            p.advance();
            let pos = pos_of(&tok);
            Expr::Jump {
                is_break: false,
                pos,
            }
        }
        TokenKind::THROW => {
            p.advance();
            let pos = pos_of(&tok);
            let inner = parse_expr_prec(p, 1);
            Expr::Throw {
                inner: Box::new(inner),
                pos,
            }
        }
        TokenKind::TRY => parse_try(p),
        TokenKind::SPAWN => {
            p.advance();
            let pos = pos_of(&tok);
            let inner = parse_expr_prec(p, 1);
            Expr::Spawn {
                inner: Box::new(inner),
                pos,
            }
        }
        TokenKind::DOLLAR => {
            // $identifier — dollar identifier
            p.advance();
            if p.peek() == TokenKind::IDENTIFIER {
                let id = p.advance();
                let pos = pos_of(&tok);
                Expr::Name {
                    name: format!("${}", id.text),
                    type_args: Vec::new(),
                    pos,
                }
            } else {
                let pos = pos_of(&tok);
                Expr::Invalid(pos)
            }
        }
        TokenKind::END => {
            let pos = pos_of(&tok);
            Expr::Invalid(pos)
        }
        _ => {
            // unknown atom — emit error, recover
            let found = crate::token_display_text(tok.kind);
            p.error_id(&tok, cj_diag::DiagId::PARSE_EXPECTED_EXPRESSION, &[&found]);
            p.advance();
            let pos = pos_of(&tok);
            Expr::Invalid(pos)
        }
    }
}

/// Parse `{ stmts }` block expression.
pub fn parse_block_expr(p: &mut Parser) -> Expr {
    let lc = p.expect(TokenKind::LCURL);
    let mut stmts = Vec::new();
    while !p.at(TokenKind::RCURL) && !p.at(TokenKind::END) {
        if p.eat(TokenKind::SEMI) {
            continue;
        }
        stmts.push(parse_expr_prec(p, 0));
        // statement separators: semicolon or newline handled by token skipping;
        // advance past a trailing semicolon
        p.eat(TokenKind::SEMI);
    }
    let _ = p.expect(TokenKind::RCURL);
    let pos = pos_of(&lc);
    Expr::Block { stmts, pos }
}

fn parse_if_expr(p: &mut Parser) -> Expr {
    let if_tok = p.expect(TokenKind::IF);
    let cond = parse_expr_prec(p, 1);
    let then = parse_block_expr(p);
    let els = if p.eat(TokenKind::ELSE) {
        let e = if p.at(TokenKind::IF) {
            parse_if_expr(p)
        } else {
            parse_block_expr(p)
        };
        Some(Box::new(e))
    } else {
        None
    };
    let pos = pos_of(&if_tok);
    Expr::If {
        cond: Box::new(cond),
        then: Box::new(then),
        els,
        pos,
    }
}

fn parse_while_expr(p: &mut Parser) -> Expr {
    let w = p.expect(TokenKind::WHILE);
    let cond = parse_expr_prec(p, 1);
    let body = parse_block_expr(p);
    let pos = pos_of(&w);
    Expr::While {
        cond: Box::new(cond),
        body: Box::new(body),
        pos,
    }
}

fn parse_for_in(p: &mut Parser) -> Expr {
    let f = p.expect(TokenKind::FOR);
    // pattern `in` iterable
    let pattern = parse_pattern(p);
    let _ = p.expect(TokenKind::IN);
    let iter = parse_expr_prec(p, 1);
    let body = parse_block_expr(p);
    let pos = pos_of(&f);
    Expr::ForIn {
        pattern,
        iter: Box::new(iter),
        body: Box::new(body),
        pos,
    }
}

fn parse_match(p: &mut Parser) -> Expr {
    let m = p.expect(TokenKind::MATCH);
    let scrutinee = parse_expr_prec(p, 1);
    let _ = p.expect(TokenKind::LCURL);
    let mut cases = Vec::new();
    while !p.at(TokenKind::RCURL) && !p.at(TokenKind::END) {
        let _ = p.expect(TokenKind::CASE);
        let pattern = parse_pattern(p);
        let guard = if p.eat(TokenKind::IF) {
            Some(parse_expr_prec(p, 1))
        } else {
            None
        };
        let _ = p.expect(TokenKind::DOUBLE_ARROW);
        let body = parse_expr_prec(p, 1);
        cases.push(cj_ast::MatchCase {
            pattern,
            guard,
            body,
            pos: cj_ast::CodePos::default(),
        });
        p.eat(TokenKind::COMMA);
    }
    let _ = p.expect(TokenKind::RCURL);
    let pos = pos_of(&m);
    Expr::Match {
        scrutinee: Box::new(scrutinee),
        cases,
        pos,
    }
}

fn parse_try(p: &mut Parser) -> Expr {
    let t = p.expect(TokenKind::TRY);
    let body = parse_block_expr(p);
    let mut catches = Vec::new();
    while p.at(TokenKind::CATCH) {
        let c = p.advance();
        let name = if p.peek() == TokenKind::IDENTIFIER {
            Some(p.advance().text)
        } else {
            None
        };
        let ty = if p.eat(TokenKind::COLON) {
            Some(super::ty::parse_type(p))
        } else {
            None
        };
        let cbody = parse_block_expr(p);
        catches.push(cj_ast::CatchClause {
            name,
            ty,
            body: cbody,
            pos: pos_of(&c),
        });
    }
    let finally = if p.eat(TokenKind::FINALLY) {
        Some(Box::new(parse_block_expr(p)))
    } else {
        None
    };
    let pos = pos_of(&t);
    Expr::Try {
        body: Box::new(body),
        catches,
        finally,
        pos,
    }
}

/// Pattern for `let`/`match`/`for`.
pub fn parse_pattern(p: &mut Parser) -> Pattern {
    let tok = p.peek_token().clone();
    match tok.kind {
        TokenKind::WILDCARD => {
            p.advance();
            Pattern::Wildcard(pos_of(&tok))
        }
        TokenKind::IDENTIFIER => {
            p.advance();
            // constructor pattern if followed by (
            if p.at(TokenKind::LPAREN) {
                p.advance();
                let mut args = Vec::new();
                while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
                    args.push(parse_pattern(p));
                    if !p.eat(TokenKind::COMMA) {
                        break;
                    }
                }
                let _ = p.expect(TokenKind::RPAREN);
                Pattern::Enum {
                    name: tok.text.clone(),
                    args,
                    pos: pos_of(&tok),
                }
            } else {
                Pattern::Var {
                    name: tok.text.clone(),
                    is_mutable: false,
                    pos: pos_of(&tok),
                }
            }
        }
        TokenKind::LPAREN => {
            p.advance();
            let mut elems = Vec::new();
            while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
                elems.push(parse_pattern(p));
                if !p.eat(TokenKind::COMMA) {
                    break;
                }
            }
            let _ = p.expect(TokenKind::RPAREN);
            Pattern::Tuple {
                elements: elems,
                pos: pos_of(&tok),
            }
        }
        _ => {
            // literal pattern
            if let Some(pat) = parse_literal_pattern(p) {
                pat
            } else {
                let found = crate::token_display_text(tok.kind);
                p.error_id(&tok, cj_diag::DiagId::PARSE_EXPECTED_PATTERN, &[&found]);
                p.advance();
                Pattern::Invalid(pos_of(&tok))
            }
        }
    }
}

fn parse_literal_pattern(p: &mut Parser) -> Option<Pattern> {
    let tok = p.peek_token().clone();
    match tok.kind {
        TokenKind::INTEGER_LITERAL => {
            p.advance();
            let pos = pos_of(&tok);
            let lit = Expr::Lit {
                kind: LitKind::Integer,
                value: tok.text.clone(),
                pos,
            };
            Some(Pattern::Const {
                literal: Some(Box::new(lit)),
                pos,
            })
        }
        TokenKind::STRING_LITERAL => {
            p.advance();
            let pos = pos_of(&tok);
            let lit = Expr::Lit {
                kind: LitKind::String,
                value: tok.text.clone(),
                pos,
            };
            Some(Pattern::Const {
                literal: Some(Box::new(lit)),
                pos,
            })
        }
        _ => None,
    }
}

/// True if the token can start an expression.
pub fn is_expr_start(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::INTEGER_LITERAL
            | TokenKind::FLOAT_LITERAL
            | TokenKind::STRING_LITERAL
            | TokenKind::RUNE_LITERAL
            | TokenKind::RUNE_BYTE_LITERAL
            | TokenKind::MULTILINE_STRING
            | TokenKind::IDENTIFIER
            | TokenKind::LPAREN
            | TokenKind::LSQUARE
            | TokenKind::LCURL
            | TokenKind::LET
            | TokenKind::VAR
            | TokenKind::IF
            | TokenKind::WHILE
            | TokenKind::FOR
            | TokenKind::MATCH
            | TokenKind::RETURN
            | TokenKind::BREAK
            | TokenKind::CONTINUE
            | TokenKind::THROW
            | TokenKind::TRY
            | TokenKind::SPAWN
            | TokenKind::DOLLAR
            | TokenKind::SUB
            | TokenKind::ADD
            | TokenKind::NOT
            | TokenKind::BITNOT
            | TokenKind::BOOL_LITERAL
    )
}

fn pos_of(tok: &cj_lexer::Token) -> cj_ast::CodePos {
    cj_ast::CodePos::new(
        tok.begin.line,
        tok.begin.column,
        tok.begin.offset,
        tok.end.line,
        tok.end.column,
        tok.end.offset,
    )
}

fn bin_op_from_token(k: TokenKind) -> Option<BinOp> {
    Some(match k {
        TokenKind::ADD => BinOp::Add,
        TokenKind::SUB => BinOp::Sub,
        TokenKind::MUL => BinOp::Mul,
        TokenKind::DIV => BinOp::Div,
        TokenKind::MOD => BinOp::Mod,
        TokenKind::EXP => BinOp::Exp,
        TokenKind::AND => BinOp::And,
        TokenKind::OR => BinOp::Or,
        TokenKind::BITAND => BinOp::BitAnd,
        TokenKind::BITOR => BinOp::BitOr,
        TokenKind::BITXOR => BinOp::BitXor,
        TokenKind::LSHIFT => BinOp::LShift,
        TokenKind::RSHIFT => BinOp::RShift,
        TokenKind::EQUAL => BinOp::Eq,
        TokenKind::NOTEQ => BinOp::Ne,
        TokenKind::LT => BinOp::Lt,
        TokenKind::GT => BinOp::Gt,
        TokenKind::LE => BinOp::Le,
        TokenKind::GE => BinOp::Ge,
        TokenKind::COALESCING => BinOp::Coalesce,
        TokenKind::PIPELINE => BinOp::Pipe,
        TokenKind::RANGEOP => BinOp::Range,
        TokenKind::CLOSEDRANGEOP => BinOp::ClosedRange,
        _ => return None,
    })
}

fn assign_op_from_token(k: TokenKind) -> Option<AssignOp> {
    Some(match k {
        TokenKind::ASSIGN => AssignOp::Assign,
        TokenKind::ADD_ASSIGN => AssignOp::AddAssign,
        TokenKind::SUB_ASSIGN => AssignOp::SubAssign,
        TokenKind::MUL_ASSIGN => AssignOp::MulAssign,
        TokenKind::DIV_ASSIGN => AssignOp::DivAssign,
        TokenKind::MOD_ASSIGN => AssignOp::ModAssign,
        TokenKind::EXP_ASSIGN => AssignOp::ExpAssign,
        TokenKind::AND_ASSIGN => AssignOp::AndAssign,
        TokenKind::OR_ASSIGN => AssignOp::OrAssign,
        TokenKind::BITAND_ASSIGN => AssignOp::BitAndAssign,
        TokenKind::BITOR_ASSIGN => AssignOp::BitOrAssign,
        TokenKind::BITXOR_ASSIGN => AssignOp::BitXorAssign,
        TokenKind::LSHIFT_ASSIGN => AssignOp::LShiftAssign,
        TokenKind::RSHIFT_ASSIGN => AssignOp::RShiftAssign,
        _ => return None,
    })
}
