// cj-parser: expression parsing (Pratt / precedence-climbing).
//
// Reference: cangjie_compiler/src/Parse/ParseExpr.cpp — the Pratt loop
// `ParseExpr(preT, expr)` where preP > curP stops, preP == curP with
// assignment/coalescing/exponent is right-associative.

use super::Parser;
use crate::ty::parse_type;
use cj_ast::{AssignOp, BinOp, Expr, LitKind, Param, Pattern, Type, UnOp};
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
        // Handle assignment operators specially (right-assoc, precedence 0):
        // they bind looser than any binary op, only parsed at min_prec == 0.
        // Must be checked BEFORE the `prec == 0` break below.
        if let Some(op) = assign_op_from_token(tok_kind) {
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
        let prec = tok_kind.precedence();
        if prec == 0 || prec < min_prec {
            break;
        }
        // `is` / `as` — type test / cast (official IsExpr/AsExpr). Their RHS is
        // a TYPE, not an expression: `b is D`, `d as B`, `x is (Int64, Bool)`.
        if tok_kind == TokenKind::IS || tok_kind == TokenKind::AS {
            let op_tok = p.advance();
            let ty = parse_type(p);
            let pos = pos_of(&op_tok);
            lhs = if tok_kind == TokenKind::IS {
                Expr::Is {
                    inner: Box::new(lhs),
                    ty,
                    pos,
                }
            } else {
                Expr::As {
                    inner: Box::new(lhs),
                    ty,
                    pos,
                }
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
        TokenKind::INCR | TokenKind::DECR => {
            // prefix `++x` / `--x`
            let tok = p.advance();
            let inner = parse_unary(p);
            return Expr::IncOrDec {
                is_inc: tok.kind == TokenKind::INCR,
                is_prefix: true,
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
                    // generic args after member name: `a.foo<Int64>(1)`
                    if p.peek() == TokenKind::LT && lt_is_generic_args(p) {
                        let _ = crate::ty::parse_generic_args(p);
                    }
                    let pos = pos_of(&dot);
                    e = Expr::Member {
                        object: Box::new(e),
                        name,
                        pos,
                    };
                } else {
                    let found = crate::token_display_text(&name_tok);
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
            TokenKind::LT => {
                // `foo<T>` or `A<Int64>.foo<Int64>(1)` — generic args on a
                // name or member expression (bare function reference or
                // qualified call).  Only consume if the lookahead confirms
                // this is a generic-args list, not a less-than comparison.
                if lt_is_generic_args(p) {
                    let args = crate::ty::parse_generic_args(p);
                    if let Expr::Name { type_args, .. } = &mut e {
                        *type_args = args;
                    }
                    // For other expr kinds (Member, Call, ...) we discard the
                    // args; the parse succeeds and sema can recover type info.
                    continue;
                }
                break;
            }
            TokenKind::INCR | TokenKind::DECR => {
                // postfix `x++` / `x--`
                let tok = p.advance();
                let pos = pos_of(&tok);
                e = Expr::IncOrDec {
                    is_inc: tok.kind == TokenKind::INCR,
                    is_prefix: false,
                    inner: Box::new(e),
                    pos,
                };
            }
            _ => break,
        }
    }
    e
}

/// Heuristic: does `<` at the cursor open a generic-argument list rather than
/// a less-than operator? Scans the raw token stream for a matching `>` (with
/// proper nesting) and checks that the token right after it is one that a
/// type-argument list can legally precede — `(`, `.`, `)`, `;`, `,`, `>`, etc.
/// Returns `false` when the `<` is a comparison (e.g. `a < b`).
fn lt_is_generic_args(p: &Parser) -> bool {
    let mut depth = 0usize;
    let mut i = p.cursor();
    while i < p.token_len() {
        let k = p.raw_kind_at(i);
        i += 1;
        if k == TokenKind::COMMENT || k == TokenKind::NL {
            continue;
        }
        match k {
            TokenKind::LT => depth += 1,
            TokenKind::GT => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
                if depth == 0 {
                    // token right after matching `>` determines the verdict
                    while i < p.token_len() {
                        let n = p.raw_kind_at(i);
                        i += 1;
                        if n == TokenKind::COMMENT || n == TokenKind::NL {
                            continue;
                        }
                        return matches!(
                            n,
                            TokenKind::LPAREN
                                | TokenKind::DOT
                                | TokenKind::LT
                                | TokenKind::RPAREN
                                | TokenKind::RSQUARE
                                | TokenKind::RCURL
                                | TokenKind::SEMI
                                | TokenKind::COMMA
                                | TokenKind::COLON
                                | TokenKind::DOUBLE_COLON
                                | TokenKind::END
                                | TokenKind::ASSIGN
                                | TokenKind::QUEST
                                | TokenKind::DOUBLE_ARROW
                                | TokenKind::NL
                        );
                    }
                    return true;
                }
            }
            TokenKind::END => return false,
            _ => {}
        }
    }
    false
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
        TokenKind::IDENTIFIER
        | TokenKind::THIS
        | TokenKind::SUPER
        | TokenKind::IS
        | TokenKind::AS => {
            p.advance();
            let pos = pos_of(&tok);
            Expr::Name {
                name: tok.text.clone(),
                type_args: Vec::new(),
                pos,
            }
        }
        // Primitive type keywords used in expression position:
        // `Int64(x)`, `Float64.tryParse(...)`, `UInt64(a)`, `Bool` ...
        // The type name itself is a value (constructor call / static member
        // access), matching official expression parsing.
        k if is_primitive_type_kw(k) => {
            p.advance();
            let pos = pos_of(&tok);
            Expr::Name {
                name: tok.kind.literal().to_string(),
                type_args: Vec::new(),
                pos,
            }
        }
        // quote expression: `quote( ... )` — quoted code as a token sequence.
        // Per spec Ch.14: body is tokens, `$(expr)` interpolation, `@Foo(...)`
        // nested macro calls; `\@`/`\$`/`\(`/`\)` escape as literal tokens.
        TokenKind::QUOTE => {
            p.advance();
            let open = p.expect(TokenKind::LPAREN);
            let pos = pos_of(&open);
            let mut parts: Vec<Expr> = Vec::new();
            // Nesting-aware quote-body scan (official ParseQuoteTokens): the
            // quote's own `(` starts depth 1; every nested `(` increments and
            // every `)` decrements; the scan stops at the matching closing `)`
            // (depth 0). Without this, `quote((a, b) += (2, 3))` terminated at
            // the FIRST `)` and the rest desynced the enclosing expression.
            let mut depth = 1i32;
            let mut closed = false;
            while depth > 0 && !p.at(TokenKind::END) {
                let t = p.peek_token().clone();
                // `$(expr)` interpolation
                if t.kind == TokenKind::DOLLAR && p.peek_ahead(1) == TokenKind::LPAREN {
                    let dpos = pos_of(&t);
                    p.advance(); // $
                    p.advance(); // (
                    let inner = parse_expr(p);
                    let rp = p.expect(TokenKind::RPAREN);
                    let mut ipos = dpos;
                    ipos.end_line = rp.end.line;
                    ipos.end_col = rp.end.column;
                    ipos.end_offset = rp.end.offset;
                    parts.push(Expr::Paren {
                        inner: Box::new(inner),
                        pos: ipos,
                    });
                    continue;
                }
                // bare `$` -> literal token
                if t.kind == TokenKind::DOLLAR {
                    p.advance();
                    parts.push(Expr::TokenPart {
                        text: "$".to_string(),
                        pos: pos_of(&t),
                    });
                    continue;
                }
                // `@Foo(...)` nested macro call inside quote
                if t.kind == TokenKind::AT && p.peek_ahead(1).is_name_like() {
                    // nested macro call: parse `@Foo(...)` inline
                    p.advance(); // @
                    let name_tok = p.advance();
                    let mut mpos = pos_of(&t);
                    let mname = name_tok.text.clone();
                    let mut margs: Vec<cj_ast::Tokenish> = Vec::new();
                    if p.eat(TokenKind::LPAREN) {
                        // nesting-aware: stop at the macro's own closing `)`
                        let mut mdepth = 1i32;
                        while mdepth > 0 && !p.at(TokenKind::END) {
                            let a = p.advance();
                            margs.push(cj_ast::Tokenish {
                                text: a.text.clone(),
                                pos: pos_of(&a),
                            });
                            match a.kind {
                                TokenKind::LPAREN => mdepth += 1,
                                TokenKind::RPAREN => mdepth -= 1,
                                _ => {}
                            }
                            if mdepth == 0 {
                                mpos.end_line = a.end.line;
                                mpos.end_col = a.end.column;
                                mpos.end_offset = a.end.offset;
                                break;
                            }
                        }
                    }
                    parts.push(Expr::MacroExpand {
                        name: mname,
                        args: margs,
                        pos: mpos,
                    });
                    continue;
                }
                // plain token — capture its text as a TokenPart
                p.advance();
                match t.kind {
                    TokenKind::LPAREN => depth += 1,
                    TokenKind::RPAREN => {
                        depth -= 1;
                        // the quote's own closing `)` ends the scan
                        if depth == 0 {
                            closed = true;
                            break;
                        }
                    }
                    _ => {}
                }
                parts.push(Expr::TokenPart {
                    text: t.text.clone(),
                    pos: pos_of(&t),
                });
            }
            if !closed {
                let _ = p.expect(TokenKind::RPAREN);
            }
            Expr::Quote { parts, pos }
        }
        // `@` macro invocation: `@Foo(args...)`
        TokenKind::AT | TokenKind::AT_EXCL => {
            p.advance();
            let name_tok = p.peek_token().clone();
            if !name_tok.kind.is_name_like() {
                // bare `@`/`@!` not followed by a name — treat as error token
                return Expr::Invalid(pos_of(&tok));
            }
            p.advance();
            let name = name_tok.text.clone();
            let mut pos = pos_of(&tok);
            let mut args: Vec<cj_ast::Tokenish> = Vec::new();
            if p.eat(TokenKind::LPAREN) {
                // macro args are tokens until matching RPAREN
                while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
                    let a = p.advance();
                    args.push(cj_ast::Tokenish {
                        text: a.text.clone(),
                        pos: pos_of(&a),
                    });
                    pos.end_line = a.end.line;
                    pos.end_col = a.end.column;
                    pos.end_offset = a.end.offset;
                }
                // extend the call span through the closing `)`
                let rp = p.expect(TokenKind::RPAREN);
                if rp.kind == TokenKind::RPAREN {
                    pos.end_line = rp.end.line;
                    pos.end_col = rp.end.column;
                    pos.end_offset = rp.end.offset;
                }
            }
            Expr::MacroExpand { name, args, pos }
        }
        TokenKind::LPAREN => {
            p.advance();
            // empty parens `()` — the Unit value (spec Ch.02/Ch.05). Parse it
            // as a Unit literal; the inner expression path would otherwise
            // call parse_atom on `)` and emit a spurious diagnostic.
            if p.at(TokenKind::RPAREN) {
                p.advance();
                let pos = pos_of(&tok);
                Expr::Lit {
                    kind: LitKind::Unit,
                    value: "()".to_string(),
                    pos,
                }
            } else {
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
        TokenKind::LCURL => {
            if lambda_brace_ahead(p) {
                parse_lambda(p)
            } else {
                parse_block_expr(p)
            }
        }
        TokenKind::LET | TokenKind::VAR | TokenKind::CONST => {
            // `let pattern = expr` / `const pattern = expr` — LetPatternDestructor
            // (a statement-like expr); const is an immutable local
            p.advance();
            let mut pattern = parse_pattern(p);
            // Typed pattern: `let x: Int64 = expr` — attach the annotation to a
            // simple Var pattern so the typechecker sees the declared type (020).
            if p.eat(TokenKind::COLON) {
                if let cj_ast::Pattern::Var { ty, .. } = &mut pattern {
                    *ty = Some(parse_type(p));
                }
            }
            let init = if p.eat(TokenKind::ASSIGN) {
                parse_expr_prec(p, 1)
            } else if p.eat(TokenKind::BACKARROW) {
                // pattern-match bind: `let Some(x) <- v` (spec Ch.12 if-let /
                // match-with-pattern). Same shape as `=`, different operator.
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
        TokenKind::DO => parse_do_while(p),
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
        TokenKind::FUNC => {
            // local function declaration inside a block: `func f() { ... }`.
            // The AST has no local-decl expression node yet, so the decl is
            // parsed (validating its tokens) and discarded as Invalid.
            let t = p.peek_token().clone();
            let _ = crate::decl::parse_decl(p, false);
            Expr::Invalid(pos_of(&t))
        }
        TokenKind::END => {
            let pos = pos_of(&tok);
            Expr::Invalid(pos)
        }
        _ => {
            // unknown atom — emit error, recover
            let found = crate::token_display_text(&tok);
            p.error_id(&tok, cj_diag::DiagId::PARSE_EXPECTED_EXPRESSION, &[&found]);
            p.advance();
            let pos = pos_of(&tok);
            Expr::Invalid(pos)
        }
    }
}

/// True if the `{` at the cursor opens a lambda (closure) literal rather than
/// a block: a `=>` appears at nesting depth 0 before the matching `}`.
/// Per spec Ch.05: `'{' lambdaParameters? '=>' expressionOrDeclarations '}'`.
fn lambda_brace_ahead(p: &Parser) -> bool {
    let mut depth = 0usize;
    let mut i = p.cursor() + 1; // skip the opening `{`
    while i < p.token_len() {
        match p.raw_kind_at(i) {
            TokenKind::LCURL | TokenKind::LPAREN | TokenKind::LSQUARE => depth += 1,
            TokenKind::RCURL | TokenKind::RPAREN | TokenKind::RSQUARE => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            TokenKind::DOUBLE_ARROW if depth == 0 => return true,
            TokenKind::END => return false,
            _ => {}
        }
        i += 1;
    }
    false
}

/// Parse a lambda (closure) literal: `{a: Int64, b => a + b}`, `{=> 123}`,
/// `{i =>}` — `{` params? `=>` body `}`.
fn parse_lambda(p: &mut Parser) -> Expr {
    let lc = p.expect(TokenKind::LCURL);
    let mut params = Vec::new();
    // lambda parameters until `=>`; may be empty (`{=> ...}`)
    while !p.at(TokenKind::DOUBLE_ARROW) && !p.at(TokenKind::END) {
        let name_tok = p.peek_token().clone();
        if name_tok.kind != TokenKind::IDENTIFIER
            && !name_tok.kind.is_name_like()
            && name_tok.kind != TokenKind::WILDCARD
        {
            break;
        }
        let name = p.advance().text;
        // '!' named-param marker comes right after the name: `a!: T`
        let is_named = p.eat(TokenKind::NOT);
        let ty = if p.eat(TokenKind::COLON) {
            parse_type(p)
        } else {
            // unannotated lambda param — placeholder type (sema infers)
            Type::Invalid(pos_of(&name_tok))
        };
        params.push(Param {
            name,
            is_named,
            ty,
            default: None,
            pos: pos_of(&name_tok),
        });
        if !p.eat(TokenKind::COMMA) {
            break;
        }
    }
    let _ = p.expect(TokenKind::DOUBLE_ARROW);
    // body: expression / declaration sequence until `}`
    let mut stmts = Vec::new();
    while !p.at(TokenKind::RCURL) && !p.at(TokenKind::END) {
        if p.eat(TokenKind::SEMI) {
            continue;
        }
        stmts.push(parse_expr_prec(p, 0));
        p.eat(TokenKind::SEMI);
    }
    let _ = p.expect(TokenKind::RCURL);
    let pos = pos_of(&lc);
    let body = Expr::Block { stmts, pos };
    Expr::Lambda {
        params,
        body: Box::new(body),
        pos,
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
    if !p.eat(TokenKind::RCURL) {
        // Close the block (official DiagForBlock / DetectPrematureEnd): if we
        // hit a closing paren/bracket or EOF, this `{` was never closed ->
        // unclosed delimiter anchored at the opening `{` (014 expects L10:28).
        if p.at_any(&[TokenKind::RPAREN, TokenKind::RSQUARE, TokenKind::GT]) || p.at(TokenKind::END)
        {
            let t = if p.at(TokenKind::END) {
                // Official DiagExpectedRightDelimiter anchors at `lastToken.End()`
                // when the lookahead isn't a newline — the position just past the
                // opening `{` (014 expects L10:28, one past the `{` at 27).
                let mut tc = lc.clone();
                tc.begin = lc.end;
                tc
            } else {
                p.peek_token().clone()
            };
            p.error_id(
                &t,
                cj_diag::DiagId::PARSE_EXPECTED_RIGHT_DELIMITER,
                &["{", "}", "{"],
            );
        } else {
            let _ = p.expect(TokenKind::RCURL);
        }
    }
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

/// `do { body } while (cond)` — loop that runs the body before testing.
fn parse_do_while(p: &mut Parser) -> Expr {
    let d = p.expect(TokenKind::DO);
    let body = parse_block_expr(p);
    let _ = p.expect(TokenKind::WHILE);
    let cond = parse_expr_prec(p, 1);
    let pos = pos_of(&d);
    Expr::While {
        cond: Box::new(cond),
        body: Box::new(body),
        pos,
    }
}

fn parse_for_in(p: &mut Parser) -> Expr {
    let f = p.expect(TokenKind::FOR);
    // `for (pattern in iterable [where guard]) { body }` — the `(` belongs to
    // the for construct itself (official ParseForInExpr), NOT a tuple pattern.
    let _ = p.expect(TokenKind::LPAREN);
    let pattern = parse_pattern(p);
    let _ = p.expect(TokenKind::IN);
    let iter = parse_expr_prec(p, 1);
    // optional `where` pattern guard (consumed; no AST slot)
    if p.eat(TokenKind::WHERE) {
        let _ = parse_expr_prec(p, 1);
    }
    let _ = p.expect(TokenKind::RPAREN);
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
    // selectorless match: `match { case ... }` (each case is a bool expr)
    let scrutinee = if p.at(TokenKind::LCURL) {
        Expr::Lit {
            kind: LitKind::Unit,
            value: String::new(),
            pos: pos_of(&m),
        }
    } else {
        parse_expr_prec(p, 1)
    };
    let _ = p.expect(TokenKind::LCURL);
    let mut cases = Vec::new();
    while !p.at(TokenKind::RCURL) && !p.at(TokenKind::END) {
        let _ = p.expect(TokenKind::CASE);
        let pattern = parse_pattern(p);
        // guard: `case p if cond =>` or `case p where cond =>` (LLT uses both)
        let guard = if p.eat(TokenKind::IF) || p.eat(TokenKind::WHERE) {
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
    // try-with-resources: `try (x = expr, ...) { body }` — the resource
    // declarations sit in parens before the block; consume and discard
    // (the AST has no resource slot yet).
    if p.at(TokenKind::LPAREN) {
        p.advance();
        while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
            if p.peek().is_name_like() {
                p.advance();
                if p.eat(TokenKind::ASSIGN) {
                    let _ = parse_expr_prec(p, 1);
                }
            } else {
                break;
            }
            if !p.eat(TokenKind::COMMA) {
                break;
            }
        }
        let _ = p.expect(TokenKind::RPAREN);
    }
    let body = parse_block_expr(p);
    let mut catches = Vec::new();
    while p.at(TokenKind::CATCH) {
        let c = p.advance();
        // `catch (e: Exception)` — parens around the exception binding
        // are optional (`catch e: Exception` also legal).
        let has_paren = p.eat(TokenKind::LPAREN);
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
        if has_paren {
            let _ = p.expect(TokenKind::RPAREN);
        }
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
    let mut pat = match tok.kind {
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
                    name_pos: pos_of(&tok),
                    is_mutable: false,
                    ty: None,
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
                let found = crate::token_display_text(&tok);
                p.error_id(&tok, cj_diag::DiagId::PARSE_EXPECTED_PATTERN, &[&found]);
                p.advance();
                Pattern::Invalid(pos_of(&tok))
            }
        }
    };
    // typed pattern: `pattern : Type` — consume `:` and parse type,
    // attaching to Var if applicable, discarding for other patterns.
    // (Spec Ch.12: match-case patterns can carry a type annotation.)
    if p.eat(TokenKind::COLON) {
        let ty = parse_type(p);
        if let Pattern::Var { ty: slot, .. } = &mut pat {
            *slot = Some(ty);
        }
    }
    pat
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

/// True if the token is a primitive type keyword (`Int64`, `Float32`, `Bool`,
/// `Rune`, `Unit`, ...) usable as a type name OR as an expression atom
/// (constructor call / static access).
pub fn is_primitive_type_kw(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::INT8
            | TokenKind::INT16
            | TokenKind::INT32
            | TokenKind::INT64
            | TokenKind::INTNATIVE
            | TokenKind::UINT8
            | TokenKind::UINT16
            | TokenKind::UINT32
            | TokenKind::UINT64
            | TokenKind::UINTNATIVE
            | TokenKind::FLOAT16
            | TokenKind::FLOAT32
            | TokenKind::FLOAT64
            | TokenKind::RUNE
            | TokenKind::BOOLEAN
            | TokenKind::NOTHING
            | TokenKind::UNIT
            | TokenKind::VARRAY
    )
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
            | TokenKind::THIS
            | TokenKind::SUPER
            | TokenKind::LPAREN
            | TokenKind::LSQUARE
            | TokenKind::LCURL
            | TokenKind::LET
            | TokenKind::VAR
            | TokenKind::CONST
            | TokenKind::IF
            | TokenKind::WHILE
            | TokenKind::DO
            | TokenKind::FOR
            | TokenKind::MATCH
            | TokenKind::RETURN
            | TokenKind::BREAK
            | TokenKind::CONTINUE
            | TokenKind::THROW
            | TokenKind::TRY
            | TokenKind::SPAWN
            | TokenKind::FUNC
            | TokenKind::DOLLAR
            | TokenKind::SUB
            | TokenKind::ADD
            | TokenKind::NOT
            | TokenKind::BITNOT
            | TokenKind::BOOL_LITERAL
    ) || is_primitive_type_kw(k)
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
