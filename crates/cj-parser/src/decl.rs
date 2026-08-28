// cj-parser: declaration parsing.

use super::expr::{parse_block_expr, parse_expr_prec};
use super::ty::parse_type;
use super::Parser;
use cj_ast::{Body, Decl, EnumCase, Expr, Param, Type, TypeParam};
use cj_lexer::TokenKind;

/// Parse one declaration. `is_member` indicates class-like body context.
pub fn parse_decl(p: &mut Parser, is_member: bool) -> Option<Decl> {
    // modifiers
    let mut mods: Vec<cj_lexer::Token> = Vec::new(); // for diagnostic reporting
    let mut is_public = false;
    let mut is_static = false;
    let mut is_abstract = false;
    let mut is_open = false;
    let mut is_sealed = false;
    let mut is_mutable = false;
    let mut is_const = false;
    loop {
        match p.peek() {
            TokenKind::PUBLIC => {
                is_public = true;
                mods.push(p.advance());
            }
            TokenKind::PRIVATE | TokenKind::PROTECTED | TokenKind::INTERNAL => {
                mods.push(p.advance());
            }
            TokenKind::STATIC => {
                is_static = true;
                mods.push(p.advance());
            }
            TokenKind::ABSTRACT => {
                is_abstract = true;
                mods.push(p.advance());
            }
            TokenKind::OPEN => {
                is_open = true;
                mods.push(p.advance());
            }
            TokenKind::SEALED => {
                is_sealed = true;
                mods.push(p.advance());
            }
            TokenKind::MUT => {
                is_mutable = true;
                mods.push(p.advance());
            }
            // `const` prefix: const var / const func / const init (spec Ch.16)
            TokenKind::CONST => {
                is_const = true;
                mods.push(p.advance());
            }
            // `override` on members (spec Ch.09: overriding members of a base
            // class-like). Consumed and recorded; sema verifies the member
            // actually overrides one. Legal only in class-like bodies.
            TokenKind::OVERRIDE => {
                mods.push(p.advance());
            }
            // `foreign` — FFI function declaration (spec Ch.15): `foreign
            // func bar(): Int` / `foreign let x`. Consumed, recorded.
            TokenKind::FOREIGN => {
                mods.push(p.advance());
            }
            // `redef` on enum cases (spec Ch.07) — consumed, recorded.
            TokenKind::REDEF => {
                mods.push(p.advance());
            }
            // `operator func ...` — operator keyword precedes `func`
            TokenKind::OPERATOR => {
                mods.push(p.advance());
            }
            _ => break,
        }
    }

    let tok = p.peek_token().clone();
    // Validate modifier/decl-kind combinations (official ParseModifiers rules).
    let mod_info = ModInfo {
        mods: &mods,
        is_sealed,
        is_member,
    };
    check_modifier_usage(p, &mod_info, &tok);
    match tok.kind {
        // Type-named constructor: `[mods] TypeName(params) { body }` inside a
        // class-like body. Official reports it as an unused *function*.
        k if k.is_name_like() && is_member && p.peek_ahead(1) == TokenKind::LPAREN => {
            p.advance();
            let mut params = parse_param_list(p);
            parse_extra_param_lists(p, &mut params);
            let body = parse_body(p);
            Some(Decl::Func {
                name: tok.text.clone(),
                name_pos: pos_of(&tok),
                is_public,
                is_static,
                is_abstract: false,
                type_params: Vec::new(),
                params,
                ret: None,
                body,
                pos: pos_of(&tok),
            })
        }
        // Finalizer: `~init(params) { body }` inside a class-like body.
        TokenKind::BITNOT if is_member && p.peek_ahead(1) == TokenKind::INIT => {
            p.advance(); // ~
            p.advance(); // init
            let mut params = parse_param_list(p);
            parse_extra_param_lists(p, &mut params);
            let body = parse_body(p);
            Some(Decl::Func {
                name: "~init".to_string(),
                name_pos: pos_of(&tok),
                is_public,
                is_static,
                is_abstract: false,
                type_params: Vec::new(),
                params,
                ret: None,
                body,
                pos: pos_of(&tok),
            })
        }
        TokenKind::FUNC => {
            p.advance();
            // operator func? `operator func [](...)`
            let _is_operator = p.eat(TokenKind::OPERATOR);
            let name_tok = p.peek_token().clone();
            let name = match name_tok.kind {
                k if k.is_name_like() => p.advance().text,
                // operator overload names: `+`, `==`, `[]`, `()`, `!` etc.
                // The call/index operator names span both delimiters: `operator
                // func [](...)` / `operator func ()(...)`. `!`/`~` (NOT/BITNOT)
                // are unary operator overloads (`operator func !()`).
                k if k.operator_like() || matches!(k, TokenKind::NOT | TokenKind::BITNOT) => {
                    let t = p.advance();
                    let close = match t.kind {
                        TokenKind::LPAREN => Some(TokenKind::RPAREN),
                        TokenKind::LSQUARE => Some(TokenKind::RSQUARE),
                        _ => None,
                    };
                    if let Some(c) = close {
                        if p.peek() == c {
                            p.advance();
                        }
                    }
                    t.text.clone()
                }
                _ => {
                    let found = crate::token_display_text(&name_tok);
                    p.error_id(
                        &name_tok,
                        cj_diag::DiagId::PARSE_EXPECTED_NAME,
                        &["func", "name", &found],
                    );
                    String::new()
                }
            };
            let type_params = parse_type_params(p);
            let mut params = parse_param_list(p);
            parse_extra_param_lists(p, &mut params);
            // return type after `:`
            let ret = if p.eat(TokenKind::COLON) {
                if crate::ty::is_type_start(p.peek()) {
                    Some(parse_type(p))
                } else {
                    // `func f(): {` — official DiagExpectedIdentifierFuncBody
                    // emits `expected a type name after ':' in function
                    // declaration, found '{'` anchored at the found token (019).
                    let t = p.peek_token().clone();
                    let found = crate::token_display_text(&t);
                    p.error_id(
                        &t,
                        cj_diag::DiagId::PARSE_EXPECTED_NAME,
                        &["a type name", "after ':' in function declaration", &found],
                    );
                    None
                }
            } else {
                None
            };
            // generic constraints: `func foo<T>(a: T) where T <: C { ... }`
            parse_where_clause(p);
            let body = parse_body(p);
            Some(Decl::Func {
                name,
                name_pos: pos_of(&name_tok),
                is_public,
                is_static,
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
            if !name_tok.kind.is_name_like() && name_tok.kind != TokenKind::WILDCARD {
                // Official DiagExpectedIdentifierOrPattern (ParseDecl.cpp): the
                // found token may be a literal/keyword/etc — emit
                // `expected identifier or pattern after 'let', found X` and
                // recover by consuming to the end of line (returns InvalidDecl,
                // no further diagnostics). (012)
                let found = crate::token_display_text(&name_tok);
                p.error_id(
                    &name_tok,
                    cj_diag::DiagId::PARSE_EXPECTED_ONE_OF_IDENTIFIER_OR_PATTERN,
                    &[if is_mut { "var" } else { "let" }, &found],
                );
                p.consume_until_nl();
                return Some(Decl::Var {
                    name: String::new(),
                    name_pos: pos_of(&tok),
                    is_mutable: is_mut || is_mutable,
                    is_public,
                    is_static,
                    ty: None,
                    init: None,
                    pos: pos_of(&tok),
                });
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
                name_pos: pos_of(&name_tok),
                is_mutable: is_mut || is_mutable,
                is_public,
                is_static,
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
            parse_where_clause(p);
            let members = if p.at(TokenKind::LCURL) || p.at(TokenKind::COLON) {
                parse_class_body(p)
            } else {
                Vec::new()
            };
            Some(Decl::Class {
                name,
                name_pos: pos_of(&name_tok),
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
            // `struct R <: I { ... }` — structs can inherit interfaces too
            // (spec Ch.06), so consume the supertype list like classes do.
            // (Decl::Struct has no parents slot yet; discard at parser layer.)
            let _parents = parse_parents(p);
            parse_where_clause(p);
            let members = if p.at(TokenKind::LCURL) {
                parse_class_body(p)
            } else {
                Vec::new()
            };
            Some(Decl::Struct {
                name,
                name_pos: pos_of(&name_tok),
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
            // `enum E<T> <: I { ... }` — enums can implement interfaces
            // (spec Ch.07/Ch.09); consume the supertype list like classes.
            let _parents = parse_parents(p);
            parse_where_clause(p);
            let cases = parse_enum_cases(p);
            Some(Decl::Enum {
                name,
                name_pos: pos_of(&name_tok),
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
            parse_where_clause(p);
            let members = if p.at(TokenKind::LCURL) {
                parse_class_body(p)
            } else {
                Vec::new()
            };
            Some(Decl::Interface {
                name,
                name_pos: pos_of(&name_tok),
                is_public,
                type_params,
                parents,
                members,
                pos: pos_of(&tok),
            })
        }
        TokenKind::EXTEND => {
            p.advance();
            // optional generic type params: `extend<U> A<U> {` / `extend<K> A<K>`
            let _type_params = parse_type_params(p);
            let target = parse_type(p);
            // optional parent types: `extend Int64 <: Eqq { ... }`
            let _parents = parse_parents(p);
            parse_where_clause(p);
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
            // generic type alias: `type A<T, K> = ...`
            let _type_params = parse_type_params(p);
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
            // optional accessor block: `prop p: T { get() {...} set(v) {...} }`
            if p.at(TokenKind::LCURL) {
                parse_prop_accessors(p);
            }
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
            // optional return type: `macro M(...): Tokens { ... }`
            if p.eat(TokenKind::COLON) {
                let _ = parse_type(p);
            }
            Some(Decl::Macro {
                name,
                is_public,
                params,
                body: parse_body(p),
                pos: pos_of(&tok),
            })
        }
        TokenKind::MAIN => {
            // bare `main()` — Cangjie allows top-level main without `func`
            p.advance();
            // consume (and discard) params / return type — Main decl carries
            // only the body in our AST; the tokens still must be read.
            let _ = parse_param_list(p);
            if p.eat(TokenKind::COLON) {
                let _ = parse_type(p);
            }
            let body = parse_body(p);
            Some(Decl::Main {
                body,
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
        // Top-level macro invocation expanding to a declaration: `@M(...)`
        TokenKind::AT | TokenKind::AT_EXCL => {
            p.advance();
            let name_tok = p.peek_token().clone();
            if !name_tok.kind.is_name_like() {
                // bare `@`/`@!` — let top-level recovery report it
                return None;
            }
            let name = p.advance().text;
            let mut args: Vec<cj_ast::Tokenish> = Vec::new();
            if p.eat(TokenKind::LPAREN) {
                while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
                    let a = p.advance();
                    args.push(cj_ast::Tokenish {
                        text: a.text.clone(),
                        pos: pos_of(&a),
                    });
                }
                let _ = p.expect(TokenKind::RPAREN);
            } else if p.eat(TokenKind::LSQUARE) {
                // annotation argument block: `@A[a < b, c >= d]` or
                // `@A[target: [EnumConstructor]]` (nested brackets).
                let mut depth = 0i32;
                while !p.at(TokenKind::END) {
                    let k = p.peek();
                    if k == TokenKind::LSQUARE {
                        depth += 1;
                    } else if k == TokenKind::RSQUARE {
                        if depth == 0 {
                            p.advance();
                            break;
                        }
                        depth -= 1;
                    }
                    let a = p.advance();
                    args.push(cj_ast::Tokenish {
                        text: a.text.clone(),
                        pos: pos_of(&a),
                    });
                }
            }
            Some(Decl::MacroExpand {
                name,
                args,
                pos: pos_of(&tok),
            })
        }
        // `const name = expr` / `const name: T = expr` — const variable
        k if is_const && k.is_name_like() => {
            let name_tok = p.advance();
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
                name_pos: pos_of(&name_tok),
                is_mutable: false,
                is_public,
                is_static,
                ty,
                init,
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
        while !p.at(TokenKind::GT) && !p.at(TokenKind::RSHIFT) && !p.at(TokenKind::END) {
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
        let _ = p.eat_gt_close();
    }
    out
}

/// Heuristic: `<` followed by IDENTIFIER `,`/`>` is a type param list, not `<`.
fn looks_like_type_param_list(p: &Parser) -> bool {
    matches!(p.peek_ahead(1), TokenKind::IDENTIFIER)
        && matches!(p.peek_ahead(2), TokenKind::GT | TokenKind::COMMA)
}

/// Parse a rejected second parameter list: `func f(a: T)(b: U)` is invalid in
/// Cangjie, but the official parser reports `expected '{', found '('` and then
/// recovers by parsing `(...)` as more params so they stay analyzable for
/// later diagnostics (diagnostics_022: `Parameter 's'`).
fn parse_extra_param_lists(p: &mut Parser, params: &mut Vec<Param>) {
    while p.at(TokenKind::LPAREN) {
        let l = p.peek_token().clone();
        // Official ParseFuncDecl/ParseMainDecl hardcode `(` (no quotes) as the
        // found token for the second param list: `expected '{', found (` (022).
        p.error_id(&l, cj_diag::DiagId::PARSE_EXPECTED_LEFT_BRACE, &["("]);
        params.extend(parse_param_list(p));
    }
}

/// Parse a `where` clause (generic constraints), per spec Ch.09:
/// `'where' identifier '<:' upperBounds (',' identifier '<:' upperBounds)*`,
/// where upperBounds may chain with `&` (`T <: Comparable<T> & Seqence`).
/// Constraints are consumed and discarded at the parser layer (sema checks
/// them later); this only keeps the declaration from desyncing.
fn parse_where_clause(p: &mut Parser) {
    if !p.eat(TokenKind::WHERE) {
        return;
    }
    loop {
        // lower bound: a type variable name
        if p.peek().is_name_like() {
            p.advance();
        }
        let _ = p.expect(TokenKind::UPPERBOUND);
        let _ = parse_type(p);
        while p.eat(TokenKind::BITAND) {
            let _ = parse_type(p);
        }
        if !p.eat(TokenKind::COMMA) {
            break;
        }
    }
}

fn parse_param_list(p: &mut Parser) -> Vec<Param> {
    let mut out = Vec::new();
    // First named (`a!`) parameter seen; later non-named params are errors
    // (official: `unnamed parameters must come before named parameters`, 021).
    let mut met_named = false;
    let _ = p.expect(TokenKind::LPAREN);
    while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
        // named param: `a!: Int64` or `a: Int64`; varargs `...`
        if p.eat(TokenKind::ELLIPSIS) {
            // variadic — skip rest handling for now
            if p.peek() == TokenKind::IDENTIFIER {
                p.advance();
            }
        }
        // param annotations: `func f(@APILevel[11] v: Int64)`
        while eat_annotation(p) {}
        // constructor params may carry access modifiers + let/var:
        // `public let domain: CString`
        while let TokenKind::PUBLIC
        | TokenKind::PRIVATE
        | TokenKind::PROTECTED
        | TokenKind::INTERNAL
        | TokenKind::LET
        | TokenKind::VAR = p.peek()
        {
            p.advance();
        }
        let name_tok = p.peek_token().clone();
        // `_` (WILDCARD) is a valid anonymous parameter name: `func f(_: Int64)`
        // (spec Ch.05: `parameterName` may be `_`). It also permits the
        // `_!:` named form which sema rejects — the parser accepts both.
        if name_tok.kind.is_name_like() || name_tok.kind == TokenKind::WILDCARD {
            p.advance();
            // `name!:` or `name:`
            let is_named = p.eat(TokenKind::NOT);
            if met_named && !is_named {
                // an unnamed param after a named one (official 021)
                p.error_id(
                    &name_tok,
                    cj_diag::DiagId::PARSE_NAMED_PARAMETER_AFTER_UNNAMED,
                    &[],
                );
            }
            let _ = p.expect(TokenKind::COLON);
            let ty = parse_type(p);
            // Default value: `name: T = expr`. Legal only on named params
            // (`a!:`); on positional ones the official parser reports
            // `expected ',' or ')', found '='` but still parses the default
            // so the following params stay analyzable (diagnostics_021).
            let mut default = None;
            if p.at(TokenKind::ASSIGN) {
                let eq_tok = p.peek_token().clone();
                if !is_named {
                    let found = crate::token_display_text(&eq_tok);
                    p.error_id(
                        &eq_tok,
                        cj_diag::DiagId::PARSE_EXPECTED_DOT_LPAREN,
                        &[&found],
                    );
                }
                p.advance();
                default = Some(parse_expr_prec(p, 0));
            }
            if is_named {
                met_named = true;
            }
            out.push(Param {
                name: name_tok.text.clone(),
                is_named,
                ty,
                default,
                pos: pos_of(&name_tok),
            });
        } else {
            // param without name? error recovery
            let found = crate::token_display_text(&name_tok);
            p.error_id(
                &name_tok,
                cj_diag::DiagId::PARSE_EXPECTED_NAME,
                &["parameter", "name", &found],
            );
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
    // `class A <: B, C` / `class A <: B & I1 & I2` (multiple interfaces
    // separated by `&`, per official supertype-list grammar) / `class A : B`
    if p.at(TokenKind::UPPERBOUND) {
        p.advance();
        loop {
            out.push(parse_type(p));
            // interface separator is `&`; `,` also accepted
            if !p.eat(TokenKind::COMMA) && !p.eat(TokenKind::BITAND) {
                break;
            }
        }
    } else if p.eat(TokenKind::COLON) {
        loop {
            out.push(parse_type(p));
            if !p.eat(TokenKind::COMMA) && !p.eat(TokenKind::BITAND) {
                break;
            }
        }
    }
    out
}

/// Consume a property accessor block: `prop p: T { get() {...} set(v) {...} }`
/// or `prop p: T { init(v) {...} }`. The accessor keyword is a plain
/// identifier (`get`/`set`); `init` is a keyword token. Bodies are parsed and
/// discarded at the parser layer (the AST has no accessor nodes yet).
fn parse_prop_accessors(p: &mut Parser) {
    let _ = p.expect(TokenKind::LCURL);
    while !p.at(TokenKind::RCURL) && !p.at(TokenKind::END) {
        if p.eat(TokenKind::SEMI) {
            continue;
        }
        // optional `mut` / `static` before the accessor name
        let mut is_accessor_mod = false;
        while matches!(p.peek(), TokenKind::MUT | TokenKind::STATIC) {
            p.advance();
            is_accessor_mod = true;
        }
        let k = p.peek();
        let is_accessor = if k == TokenKind::IDENTIFIER {
            let text = p.peek_token().text.as_str();
            text == "get" || text == "set"
        } else {
            k == TokenKind::INIT
        };
        if !is_accessor {
            // consumed a `mut`/`static` that turned out not to be an accessor
            // — treat the whole thing as a member decl attempt and bail
            if is_accessor_mod {
                break;
            }
            break;
        }
        p.advance();
        // accessor params: `get()` / `set(v)` / `set(value: Int64)` — the
        // type annotation is optional, so use a lenient param list
        if p.at(TokenKind::LPAREN) {
            p.advance();
            while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
                if p.peek().is_name_like() || p.peek() == TokenKind::WILDCARD {
                    p.advance();
                    if p.eat(TokenKind::COLON) {
                        let _ = parse_type(p);
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
        if p.at(TokenKind::LCURL) {
            let _ = parse_block_expr(p);
        }
    }
    let _ = p.expect(TokenKind::RCURL);
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
            let found = crate::token_display_text(&t);
            p.error_id(&t, cj_diag::DiagId::PARSE_EXPECTED_DECL, &[&found]);
            p.advance();
        }
    }
    let _ = p.expect(TokenKind::RCURL);
    out
}

/// Consume one annotation prefix: `@Name` with an optional argument block
/// `@Name[...]` / `@Name(...)` (official `SeeingIfAvailable`/ParseAnnotation).
/// Nested delimiters are tracked so the scan stops at the annotation's OWN
/// closing delimiter, not an inner one (`@A[target: [EnumConstructor]]`).
/// Returns true when an annotation was consumed.
fn eat_annotation(p: &mut Parser) -> bool {
    if !matches!(p.peek(), TokenKind::AT | TokenKind::AT_EXCL) {
        return false;
    }
    p.advance(); // @ / @!
    let name_tok = p.peek_token().clone();
    if name_tok.kind.is_name_like() {
        p.advance();
    }
    let open = match p.peek() {
        TokenKind::LSQUARE => Some((TokenKind::LSQUARE, TokenKind::RSQUARE)),
        TokenKind::LPAREN => Some((TokenKind::LPAREN, TokenKind::RPAREN)),
        _ => None,
    };
    if let Some((o, c)) = open {
        let mut depth = 0i32;
        p.advance(); // consume the opening delimiter
        while !p.at(TokenKind::END) {
            let k = p.peek();
            if k == o {
                depth += 1;
            } else if k == c {
                if depth == 0 {
                    p.advance();
                    break;
                }
                depth -= 1;
            }
            p.advance();
        }
    }
    true
}

fn parse_enum_cases(p: &mut Parser) -> Vec<EnumCase> {
    let mut out = Vec::new();
    let _ = p.expect(TokenKind::LCURL);
    p.eat(TokenKind::BITOR); // optional leading `|` (official: skip the first BITOR)
    loop {
        // `...` — non-exhaustive enum marker; must be the last case (official
        // ParseEnumBody consumes it and breaks out of the case loop).
        if p.at(TokenKind::ELLIPSIS) {
            p.advance();
            break;
        }
        if p.at(TokenKind::RCURL) || p.at(TokenKind::END) {
            break;
        }
        // annotations on a case: `| @APILevel[11] caseB(Int64)`
        while eat_annotation(p) {
            // skip until the case name
            if p.at(TokenKind::BITOR) {
                p.advance();
            }
            if p.at(TokenKind::RCURL) || p.at(TokenKind::END) {
                break;
            }
        }
        if p.at(TokenKind::RCURL) || p.at(TokenKind::END) {
            break;
        }
        let name_tok = p.peek_token().clone();
        // A contextual keyword (public/private/open/...) is a case NAME when
        // not followed by an identifier (`| public(Int64)`, `public | private`)
        // — official `SeeingKeywordAndOperater`. Followed by an identifier it
        // is a (forbidden) modifier: `| private a (Int32)` is case `a` plus
        // `expected no modifier before enum constructor, found 'private'`.
        if is_contextual_keyword(name_tok.kind) && p.peek_ahead(1) == TokenKind::IDENTIFIER {
            let mod_tok = p.advance();
            let mname = crate::modifier_display(&mod_tok);
            p.error_id(
                &mod_tok,
                cj_diag::DiagId::PARSE_EXPECTED_NO_MODIFIER,
                &["enum constructor", &mname],
            );
            let name_tok = p.peek_token().clone();
            let name = p.advance().text;
            let payloads = parse_enum_case_payload(p);
            out.push(EnumCase {
                name,
                payloads,
                pos: pos_of(&name_tok),
            });
        } else if name_tok.kind == TokenKind::IDENTIFIER || is_contextual_keyword(name_tok.kind) {
            let name = p.advance().text;
            let payloads = parse_enum_case_payload(p);
            out.push(EnumCase {
                name,
                payloads,
                pos: pos_of(&name_tok),
            });
        } else {
            // Not a case start (member-decl keyword such as `func`, or
            // garbage). Report and swallow up to the next `|` (mirrors
            // official DiagExpectedIdentifierEnumDecl + TryConsumeUntilAny),
            // so the case loop always makes progress.
            let found = crate::token_display_text(&name_tok);
            p.error_id(
                &name_tok,
                cj_diag::DiagId::PARSE_EXPECTED_NAME,
                &["enum", "case name", &found],
            );
            while !p.at(TokenKind::BITOR) && !p.at(TokenKind::RCURL) && !p.at(TokenKind::END) {
                p.advance();
            }
            p.eat(TokenKind::BITOR);
            continue;
        }
        // separator: `|` or `,` continues the case list; otherwise the
        // member-decl loop below takes over (official: `while (Skip(BITOR))`).
        if !p.eat(TokenKind::BITOR) && !p.eat(TokenKind::COMMA) {
            break;
        }
    }
    // enum members after the cases: `enum E { A private func f() {} ... }`
    while !p.at(TokenKind::RCURL) && !p.at(TokenKind::END) {
        if p.eat(TokenKind::SEMI) {
            continue;
        }
        if parse_decl(p, true).is_some() {
            continue;
        }
        // No-progress guard: `parse_decl` returned None without consuming —
        // report and advance one token so the loop always makes progress
        // (mirrors parse_class_body recovery). Without this, tokens like `|`
        // or `...` in a trailing member position spin forever.
        let t = p.peek_token().clone();
        let found = crate::token_display_text(&t);
        p.error_id(&t, cj_diag::DiagId::PARSE_EXPECTED_DECL, &[&found]);
        p.advance();
    }
    let _ = p.expect(TokenKind::RCURL);
    out
}

/// Payload types of an enum case: `| A(Int64, Bool)`.
fn parse_enum_case_payload(p: &mut Parser) -> Vec<Type> {
    if !p.at(TokenKind::LPAREN) {
        return Vec::new();
    }
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
}

/// Contextual keywords usable as identifiers (official `GetContextualKeyword`,
/// Lexer.cpp): they may appear as enum case names (`| public(Int64)`).
fn is_contextual_keyword(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::PUBLIC
            | TokenKind::PRIVATE
            | TokenKind::INTERNAL
            | TokenKind::PROTECTED
            | TokenKind::OVERRIDE
            | TokenKind::REDEF
            | TokenKind::ABSTRACT
            | TokenKind::SEALED
            | TokenKind::OPEN
            | TokenKind::COMMON
            | TokenKind::SPECIFIC
            | TokenKind::FEATURES
    )
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

/// Modifier state collected while parsing a declaration (for diagnostics).
struct ModInfo<'a> {
    mods: &'a [cj_lexer::Token],
    is_sealed: bool,
    is_member: bool,
}

/// Validate modifier/decl-kind combinations, emitting official diagnostics:
/// - `parse_illegal_modifier_in_scope`: unexpected modifier on this decl kind
/// - `parse_conflict_modifier`: mutually exclusive modifiers
/// - `parse_redundant_modifier` (warning): implied by another modifier
fn check_modifier_usage(p: &mut Parser, info: &ModInfo, decl_tok: &cj_lexer::Token) {
    // Decl kind name for diagnostics: "enum declaration", "struct declaration",
    // "function declaration in 'top-level' scope", "variable declaration ...".
    // hintMes for `unexpected modifier '%s' on %s%s` (official KIND_TO_STR).
    let (kind_name, needs_scope): (&str, bool) = match decl_tok.kind {
        TokenKind::ENUM => ("enum declaration", false),
        TokenKind::STRUCT => ("struct declaration", false),
        TokenKind::FUNC => ("function declaration", !info.is_member),
        TokenKind::LET | TokenKind::VAR => ("variable declaration", !info.is_member),
        TokenKind::PROP => ("property declaration", !info.is_member),
        TokenKind::EXTEND => ("extend declaration", false),
        TokenKind::CLASS => ("class", false),
        TokenKind::INTERFACE => ("interface", false),
        _ => return,
    };
    let scope_suffix = if needs_scope {
        " in 'top-level' scope"
    } else {
        ""
    };

    // 1. `sealed` only valid on classes/interfaces (abstract allowed); report
    //    unexpected modifier on every other decl kind (extend handled below).
    if info.is_sealed && decl_tok.kind != TokenKind::EXTEND {
        let ok_on = matches!(decl_tok.kind, TokenKind::CLASS | TokenKind::INTERFACE);
        if !ok_on {
            for m in info.mods {
                if m.kind == TokenKind::SEALED {
                    let found = crate::modifier_display(m);
                    p.error_id(
                        m,
                        cj_diag::DiagId::PARSE_ILLEGAL_MODIFIER_IN_SCOPE,
                        &[&found, kind_name, scope_suffix],
                    );
                    break;
                }
            }
        }
    }

    // 2. `extend` accepts no modifiers at all.
    if decl_tok.kind == TokenKind::EXTEND && !info.mods.is_empty() {
        for m in info.mods {
            let found = crate::modifier_display(m);
            p.error_id(
                m,
                cj_diag::DiagId::PARSE_EXPECTED_NO_MODIFIER,
                &[kind_name, &found],
            );
        }
    }

    // 3. redundant modifier warnings: sealed implies open/public.
    if info.is_sealed {
        for m in info.mods {
            if m.kind == TokenKind::OPEN || m.kind == TokenKind::PUBLIC {
                let found = crate::modifier_display(m);
                let sealed_tok = info
                    .mods
                    .iter()
                    .find(|x| x.kind == TokenKind::SEALED)
                    .unwrap();
                let implied = crate::modifier_display(sealed_tok);
                p.warn_id(
                    m,
                    cj_diag::DiagId::PARSE_REDUNDANT_MODIFIER,
                    &[&found, &implied],
                );
            }
        }
    }

    // 4. conflict: abstract + sealed is allowed; but public+private impossible
    //    (only one access modifier token). static on top-level func is illegal
    //    in some cases — covered by scope rules later.
}
