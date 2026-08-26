// cj-parser: type parsing.

use super::Parser;
use cj_ast::{PrimitiveKind, Type};
use cj_lexer::TokenKind;

/// Parse a type (starts at a type name or `(`).
pub fn parse_type(p: &mut Parser) -> Type {
    let tok = p.peek_token().clone();
    match tok.kind {
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
        TokenKind::IDENTIFIER | TokenKind::THISTYPE => {
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
            // paren type / tuple type / func type
            p.advance();
            let mut elems = Vec::new();
            while !p.at(TokenKind::RPAREN) && !p.at(TokenKind::END) {
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
            let found = crate::token_display_text(tok.kind);
            p.error_id(
                &tok,
                cj_diag::DiagId::PARSE_EXPECTED_TYPE,
                &["'::'", &found],
            );
            p.advance();
            Type::Invalid(pos_of(&tok))
        }
    }
}

fn parse_generic_args(p: &mut Parser) -> Vec<Type> {
    let _ = p.expect(TokenKind::LT);
    let mut args = Vec::new();
    while !p.at(TokenKind::GT) && !p.at(TokenKind::END) {
        args.push(parse_type(p));
        if !p.eat(TokenKind::COMMA) {
            break;
        }
    }
    let _ = p.expect(TokenKind::GT);
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
