// cj-parser: type parsing.

use super::Parser;
use cj_ast::{PrimitiveKind, Type};
use cj_lexer::TokenKind;

/// True if `kind` can begin a type name (the tokens `parse_type` handles).
pub fn is_type_start(kind: TokenKind) -> bool {
    matches!(
        kind,
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
            | TokenKind::IDENTIFIER
            | TokenKind::THISTYPE
            | TokenKind::QUEST
            | TokenKind::COALESCING
            | TokenKind::LPAREN
    )
}

/// Parse a type (starts at a type name or `(`).
pub fn parse_type(p: &mut Parser) -> Type {
    let tok = p.peek_token().clone();
    // Build the base type, then apply a postfix `?` (optional type, spec
    // Ch.08 `optionType`) uniformly: `Int64?`, `Foo<T>?`, `(Int64, Bool)?`.
    let mut ty = match tok.kind {
        // `$3`, `$x` — dollar identifier used as a type name (quote meta
        // vars / token placeholders, spec Ch.14). Lexed as `$` + name/number.
        TokenKind::DOLLAR => {
            let mut text = "$".to_string();
            p.advance();
            if matches!(p.peek(), TokenKind::IDENTIFIER | TokenKind::INTEGER_LITERAL) {
                text.push_str(&p.advance().text);
            }
            Type::Ref {
                name: text,
                args: Vec::new(),
                pos: pos_of(&tok),
            }
        }
        // `?T` — Option type (spec Ch.08: `'?' type`). e.g. `(?Point2D)`.
        TokenKind::QUEST => {
            p.advance();
            let inner = parse_type(p);
            Type::Option {
                inner: Box::new(inner),
                pos: pos_of(&tok),
            }
        }
        // `??T` — lexed as the COALESCING token `??`, but in TYPE position it
        // is a doubly-nested Option (spec Ch.02: `?Type` == `Option<Type>`,
        // so `??T` == `Option<Option<T>>`). e.g. `var x: ??T = None`.
        TokenKind::COALESCING => {
            p.advance();
            let inner = parse_type(p);
            let pos = pos_of(&tok);
            Type::Option {
                inner: Box::new(Type::Option {
                    inner: Box::new(inner),
                    pos,
                }),
                pos,
            }
        }
        // `VArray` used as a GENERIC type (not a bare primitive keyword):
        // `VArray<Int64, $3>`. VARRAY lexes as a keyword; when followed by
        // `<` it names the generic builtin, so parse it as a Ref with args.
        // Note: guard must look AHEAD — the cursor is still on VARRAY when
        // the match arm is evaluated, so `p.peek_ahead(1)` (not `p.at`).
        TokenKind::VARRAY if p.peek_ahead(1) == TokenKind::LT => {
            p.advance();
            let args = parse_generic_args(p);
            Type::Ref {
                name: "VArray".to_string(),
                args,
                pos: pos_of(&tok),
            }
        }
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
        | TokenKind::VARRAY => {
            p.advance();
            let kind = match tok.kind {
                TokenKind::INT8 => PrimitiveKind::Int8,
                TokenKind::INT16 => PrimitiveKind::Int16,
                TokenKind::INT32 => PrimitiveKind::Int32,
                TokenKind::INT64 => PrimitiveKind::Int64,
                TokenKind::INTNATIVE => PrimitiveKind::IntNative,
                TokenKind::UINT8 => PrimitiveKind::UInt8,
                TokenKind::UINT16 => PrimitiveKind::UInt16,
                TokenKind::UINT32 => PrimitiveKind::UInt32,
                TokenKind::UINT64 => PrimitiveKind::UInt64,
                TokenKind::UINTNATIVE => PrimitiveKind::UIntNative,
                TokenKind::FLOAT16 => PrimitiveKind::Float16,
                TokenKind::FLOAT32 => PrimitiveKind::Float32,
                TokenKind::FLOAT64 => PrimitiveKind::Float64,
                TokenKind::RUNE => PrimitiveKind::Rune,
                TokenKind::BOOLEAN => PrimitiveKind::Bool,
                TokenKind::NOTHING => PrimitiveKind::Nothing,
                TokenKind::UNIT => PrimitiveKind::Unit,
                TokenKind::VARRAY => PrimitiveKind::VArray,
                _ => unreachable!(),
            };
            Type::Primitive {
                kind,
                pos: pos_of(&tok),
            }
        }
        // RefType with optional generic args. A contextual keyword can be a
        // type name too (`class A <: public {}` — official Seeing + SeeingContextualKeyword).
        k if k == TokenKind::IDENTIFIER || k == TokenKind::THISTYPE || k.is_name_like() => {
            // RefType with optional generic args
            let name = p.advance().text;
            let args = if p.at(TokenKind::LT) {
                parse_generic_args(p)
            } else {
                Vec::new()
            };
            let ty = Type::Ref {
                name,
                args,
                pos: pos_of(&tok),
            };
            // optional `?`
            if p.eat(TokenKind::QUEST) {
                Type::Option {
                    inner: Box::new(ty),
                    pos: pos_of(&tok),
                }
            } else {
                ty
            }
        }
        TokenKind::LPAREN => {
            // paren type / tuple type / func type / named-field tuple type
            p.advance();
            let mut elems = Vec::new();
            // named fields: `(p1: Int64, p2: Int64)` — track names for dup check
            let mut name_map: std::collections::HashMap<String, ()> =
                std::collections::HashMap::new();
            while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
                // detect named field: IDENTIFIER followed by ':'
                if p.peek() == TokenKind::IDENTIFIER && p.peek_ahead(1) == TokenKind::COLON {
                    let name_tok = p.advance(); // name
                    let _ = p.expect(TokenKind::COLON);
                    if name_map.contains_key(&name_tok.text) {
                        // duplicated type parameter name (official template)
                        p.error_id(
                            &name_tok,
                            cj_diag::DiagId::PARSE_DUPLICATE_TYPE_PARAMETER_NAME,
                            &[&name_tok.text],
                        );
                    } else {
                        name_map.insert(name_tok.text.clone(), ());
                    }
                }
                elems.push(parse_type(p));
                if !p.eat(TokenKind::COMMA) {
                    break;
                }
            }
            let _ = p.expect(TokenKind::RPAREN);
            let pos = pos_of(&tok);
            if elems.len() == 1 {
                // could be paren or func type `(T) -> R`
                let inner = elems.pop().unwrap();
                if p.eat(TokenKind::ARROW) {
                    let ret = parse_type(p);
                    Type::Func {
                        params: vec![inner],
                        ret: Box::new(ret),
                        pos,
                    }
                } else {
                    Type::Paren {
                        inner: Box::new(inner),
                        pos,
                    }
                }
            } else {
                if p.eat(TokenKind::ARROW) {
                    let ret = parse_type(p);
                    Type::Func {
                        params: elems,
                        ret: Box::new(ret),
                        pos,
                    }
                } else {
                    Type::Tuple {
                        elements: elems,
                        pos,
                    }
                }
            }
        }
        _ => {
            let found = crate::token_display_text(&tok);
            p.error_id(
                &tok,
                cj_diag::DiagId::PARSE_EXPECTED_TYPE,
                &["'::'", &found],
            );
            p.advance();
            Type::Invalid(pos_of(&tok))
        }
    };
    // postfix optional `T?` — applies to any base type
    if p.eat(TokenKind::QUEST) {
        ty = Type::Option {
            inner: Box::new(ty),
            pos: pos_of(&tok),
        };
    }
    ty
}

pub(crate) fn parse_generic_args(p: &mut Parser) -> Vec<Type> {
    let _ = p.expect(TokenKind::LT);
    let mut args = Vec::new();
    // `>>` (RSHIFT) also closes a nested generic arg list — eat_gt_close
    // splits it in place, so stop the loop on GT | RSHIFT too.
    while !p.at(TokenKind::GT) && !p.at(TokenKind::RSHIFT) && !p.at(TokenKind::END) {
        args.push(parse_type(p));
        if !p.eat(TokenKind::COMMA) {
            break;
        }
    }
    let _ = p.eat_gt_close();
    args
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
