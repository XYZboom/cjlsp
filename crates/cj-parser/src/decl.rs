// cj-parser: declaration parsing.

use super::expr::{parse_block_expr, parse_expr_prec};
use super::ty::parse_type;
use super::Parser;
use cj_ast::{Body, Decl, EnumCase, Expr, Param, Type, TypeParam};
use cj_lexer::TokenKind;

/// Parse one declaration. `is_member` indicates class-like body context.
pub fn parse_decl(p: &mut Parser, is_member: bool) -> Option<Decl> {
    // modifiers
    let mut is_public = false;
    let mut is_static = false;
    let mut is_abstract = false;
    let mut is_open = false;
    let mut is_sealed = false;
    let mut is_mutable = false;
    loop {
        match p.peek() {
            TokenKind::PUBLIC => {
                is_public = true;
                p.advance();
            }
            TokenKind::PRIVATE | TokenKind::PROTECTED | TokenKind::INTERNAL => {
                p.advance();
            }
            TokenKind::STATIC => {
                is_static = true;
                p.advance();
            }
            TokenKind::ABSTRACT => {
                is_abstract = true;
                p.advance();
            }
            TokenKind::OPEN => {
                is_open = true;
                p.advance();
            }
            TokenKind::SEALED => {
                is_sealed = true;
                p.advance();
            }
            TokenKind::MUT => {
                is_mutable = true;
                p.advance();
            }
            _ => break,
        }
    }

    let tok = p.peek_token().clone();
    match tok.kind {
        TokenKind::FUNC => {
            p.advance();
            // operator func? `operator func [](...)`
            let _is_operator = p.eat(TokenKind::OPERATOR);
            let name_tok = p.peek_token().clone();
            let name = match name_tok.kind {
                k if k.is_name_like() => p.advance().text,
                // operator overload names: `+`, `==`, `[]`, `()` etc.
                k if k.operator_like() => p.advance().text,
                _ => {
                    p.error(&name_tok, "expected function name");
                    String::new()
                }
            };
            let type_params = parse_type_params(p);
            let params = parse_param_list(p);
            // return type after `:`
            let ret = if p.eat(TokenKind::COLON) {
                Some(parse_type(p))
            } else {
                None
            };
            let body = parse_body(p);
            Some(Decl::Func {
                name,
                is_public,
                is_abstract,
                type_params,
                params,
                ret,
                body,
                pos: pos_of(&tok),
            })
        }
        TokenKind::LET | TokenKind::VAR => {
            let is_mut = tok.kind == TokenKind::VAR;
            p.advance();
            let name_tok = p.peek_token().clone();
            if !name_tok.kind.is_name_like() {
                p.error(&name_tok, "expected variable name");
                return None;
            }
            p.advance();
            let name = name_tok.text.clone();
            let ty = if p.eat(TokenKind::COLON) {
                Some(parse_type(p))
            } else {
                None
            };
            let init = if p.eat(TokenKind::ASSIGN) {
                Some(parse_expr_prec(p, 0))
            } else {
                None
            };
            Some(Decl::Var {
                name,
                is_mutable: is_mut || is_mutable,
                is_public,
                ty,
                init,
                pos: pos_of(&tok),
            })
        }
        TokenKind::CLASS => {
            p.advance();
            let name_tok = p.expect_ident("class name");
            let name = name_tok.text.clone();
            let type_params = parse_type_params(p);
            let parents = parse_parents(p);
            // class body { ... } or `:` inheritance then body
            let members = if p.at(TokenKind::LCURL) || p.at(TokenKind::COLON) {
                parse_class_body(p)
            } else {
                Vec::new()
            };
            Some(Decl::Class {
                name,
                is_public,
                is_abstract,
                is_open,
                is_sealed,
                type_params,
                parents,
                members,
                pos: pos_of(&tok),
            })
        }
        TokenKind::STRUCT => {
            p.advance();
            let name_tok = p.expect_ident("struct name");
            let name = name_tok.text.clone();
            let type_params = parse_type_params(p);
            let members = if p.at(TokenKind::LCURL) {
                parse_class_body(p)
            } else {
                Vec::new()
            };
            Some(Decl::Struct {
                name,
                is_public,
                is_open,
                type_params,
                members,
                pos: pos_of(&tok),
            })
        }
        TokenKind::ENUM => {
            p.advance();
            let name_tok = p.expect_ident("enum name");
            let name = name_tok.text.clone();
            let type_params = parse_type_params(p);
            let cases = parse_enum_cases(p);
            Some(Decl::Enum {
                name,
                is_public,
                type_params,
                cases,
                pos: pos_of(&tok),
            })
        }
        TokenKind::INTERFACE => {
            p.advance();
            let name_tok = p.expect_ident("interface name");
            let name = name_tok.text.clone();
            let type_params = parse_type_params(p);
            let parents = parse_parents(p);
            let members = if p.at(TokenKind::LCURL) {
                parse_class_body(p)
            } else {
                Vec::new()
            };
            Some(Decl::Interface {
                name,
                is_public,
                type_params,
                parents,
                members,
                pos: pos_of(&tok),
            })
        }
        TokenKind::EXTEND => {
            p.advance();
            let target = parse_type(p);
            let members = parse_class_body(p);
            Some(Decl::Extend {
                is_public,
                target,
                members,
                pos: pos_of(&tok),
            })
        }
        TokenKind::TYPE => {
            p.advance();
            let name_tok = p.expect_ident("type alias name");
            let name = name_tok.text.clone();
            let _ = p.expect(TokenKind::ASSIGN);
            let target = parse_type(p);
            Some(Decl::TypeAlias {
                name,
                is_public,
                target,
                pos: pos_of(&tok),
            })
        }
        TokenKind::PROP => {
            p.advance();
            let name_tok = p.expect_ident("property name");
            let name = name_tok.text.clone();
            let _ = p.expect(TokenKind::COLON);
            let ty = parse_type(p);
            Some(Decl::Prop {
                name,
                is_public,
                is_static,
                ty,
                pos: pos_of(&tok),
            })
        }
        TokenKind::INIT => parse_init(p, is_public, pos_of(&tok)),
        TokenKind::MACRO => {
            p.advance();
            let name_tok = p.expect_ident("macro name");
            let name = name_tok.text.clone();
            let params = parse_param_list(p);
            Some(Decl::Macro {
                name,
                is_public,
                params,
                body: parse_body(p),
                pos: pos_of(&tok),
            })
        }
        TokenKind::PACKAGE => {
            // nested package (rare); treat as package decl
            p.advance();
            let name_tok = p.peek_token().clone();
            let name = name_tok.text.clone();
            p.advance();
            Some(Decl::Package {
                name,
                pos: pos_of(&tok),
            })
        }
        _ => {
            if is_member {
                // inside a class-like body, treat unknown token as a member decl attempt
                None
            } else {
                None
            }
        }
    }
}

fn parse_type_params(p: &mut Parser) -> Vec<TypeParam> {
    let mut out = Vec::new();
    if p.at(TokenKind::LT) && looks_like_type_param_list(p) {
        p.advance(); // <
        while !p.at(TokenKind::GT) && !p.at(TokenKind::END) {
            let name_tok = p.peek_token().clone();
            if name_tok.kind != TokenKind::IDENTIFIER {
                break;
            }
            let name = p.advance().text;
            // where constraint after name
            let bounds = Vec::new();
            out.push(TypeParam {
                name,
                bounds,
                pos: pos_of(&name_tok),
            });
            if !p.eat(TokenKind::COMMA) {
                break;
            }
        }
        let _ = p.expect(TokenKind::GT);
    }
    out
}

/// Heuristic: `<` followed by IDENTIFIER `,`/`>` is a type param list, not `<`.
fn looks_like_type_param_list(p: &Parser) -> bool {
    matches!(p.peek_ahead(1), TokenKind::IDENTIFIER)
        && matches!(p.peek_ahead(2), TokenKind::GT | TokenKind::COMMA)
}

fn parse_param_list(p: &mut Parser) -> Vec<Param> {
    let mut out = Vec::new();
    let _ = p.expect(TokenKind::LPAREN);
    while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
        // named param: `a!: Int64` or `a: Int64`; varargs `...`
        if p.eat(TokenKind::ELLIPSIS) {
            // variadic — skip rest handling for now
            if p.peek() == TokenKind::IDENTIFIER {
                p.advance();
            }
        }
        let name_tok = p.peek_token().clone();
        if name_tok.kind.is_name_like() {
            p.advance();
            // `name!:` or `name:`
            let is_named = p.eat(TokenKind::NOT);
            let _ = p.expect(TokenKind::COLON);
            let ty = parse_type(p);
            let default = None;
            out.push(Param {
                name: name_tok.text.clone(),
                is_named,
                ty,
                default,
                pos: pos_of(&name_tok),
            });
        } else {
            // param without name? error recovery
            let msg = format!(
                "expected parameter name, found '{}'",
                name_tok.kind.value_str()
            );
            p.error(&name_tok, &msg);
            p.advance();
        }
        if !p.eat(TokenKind::COMMA) {
            break;
        }
    }
    let _ = p.expect(TokenKind::RPAREN);
    out
}

/// Function body: `{ ... }` or nothing (abstract/foreign).
fn parse_body(p: &mut Parser) -> Body {
    if p.at(TokenKind::LCURL) {
        let block = parse_block_expr(p);
        if let Expr::Block { stmts, .. } = block {
            Body::Block(stmts)
        } else {
            Body::Empty
        }
    } else {
        Body::Empty
    }
}

fn parse_parents(p: &mut Parser) -> Vec<Type> {
    let mut out = Vec::new();
    // `class A <: B, C` or `class A : B`
    if p.at(TokenKind::UPPERBOUND) {
        p.advance();
        loop {
            out.push(parse_type(p));
            if !p.eat(TokenKind::COMMA) {
                break;
            }
        }
    } else if p.eat(TokenKind::COLON) {
        loop {
            out.push(parse_type(p));
            if !p.eat(TokenKind::COMMA) {
                break;
            }
        }
    }
    out
}

fn parse_class_body(p: &mut Parser) -> Vec<Decl> {
    let mut out = Vec::new();
    let _ = p.expect(TokenKind::LCURL);
    while !p.at(TokenKind::RCURL) && !p.at(TokenKind::END) {
        if p.eat(TokenKind::SEMI) {
            continue;
        }
        if let Some(d) = parse_decl(p, true) {
            out.push(d);
        } else {
            // recovery
            let t = p.peek_token().clone();
            p.error(&t, "unexpected token in class body");
            p.advance();
        }
    }
    let _ = p.expect(TokenKind::RCURL);
    out
}

fn parse_enum_cases(p: &mut Parser) -> Vec<EnumCase> {
    let mut out = Vec::new();
    let _ = p.expect(TokenKind::LCURL);
    while !p.at(TokenKind::RCURL) && !p.at(TokenKind::END) {
        p.eat(TokenKind::BITOR); // optional leading |
        let name_tok = p.peek_token().clone();
        if !name_tok.kind.is_name_like() {
            p.error(&name_tok, "expected enum case name");
            p.advance();
            continue;
        }
        let name = p.advance().text;
        // payload: `| A(Int64, Bool)`
        let payloads = if p.at(TokenKind::LPAREN) {
            p.advance();
            let mut pl = Vec::new();
            while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
                pl.push(parse_type(p));
                if !p.eat(TokenKind::COMMA) {
                    break;
                }
            }
            let _ = p.expect(TokenKind::RPAREN);
            pl
        } else {
            Vec::new()
        };
        out.push(EnumCase {
            name,
            payloads,
            pos: pos_of(&name_tok),
        });
        // separator: | or , or newline
        if !(p.at(TokenKind::BITOR) || p.at(TokenKind::COMMA)) && p.at(TokenKind::RCURL) {
            break;
        }
    }
    let _ = p.expect(TokenKind::RCURL);
    out
}

fn parse_init(p: &mut Parser, is_public: bool, pos: cj_ast::CodePos) -> Option<Decl> {
    let _ = p.expect(TokenKind::INIT);
    let params = parse_param_list(p);
    let body = parse_body(p);
    Some(Decl::PrimaryCtor {
        is_public,
        params,
        body,
        pos,
    })
}

impl<'a> Parser<'a> {
    /// Expect a name (identifier OR keyword used as a name — official
    /// `ParseIdentifierFromToken` accepts any token's text, e.g. `func main()`).
    pub fn expect_ident(&mut self, what: &str) -> cj_lexer::Token {
        let tok = self.peek_token().clone();
        if tok.kind.is_name_like() {
            self.advance();
            tok
        } else {
            self.error(&tok, &format!("expected {what}"));
            tok
        }
    }
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
