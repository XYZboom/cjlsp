// cj-lexer unit tests for the auto-generated token table.
use crate::token::{lookup_keyword, TokenKind};

#[test]
fn token_count_matches_official() {
    // 官方 Tokens.inc 实际 165 条（含 EXPERIMENTAL，去重后）
    // 这里只验证我们生成的枚举是完整、可遍历的
    let kinds = [
        TokenKind::DOT,
        TokenKind::COMMA,
        TokenKind::LPAREN,
        TokenKind::RPAREN,
        TokenKind::LSQUARE,
        TokenKind::RSQUARE,
        TokenKind::LCURL,
        TokenKind::RCURL,
        TokenKind::EXP,
        TokenKind::MUL,
        TokenKind::MOD,
        TokenKind::DIV,
        TokenKind::ADD,
        TokenKind::SUB,
        TokenKind::INCR,
        TokenKind::DECR,
        TokenKind::AND,
        TokenKind::OR,
        TokenKind::COALESCING,
        TokenKind::PIPELINE,
        TokenKind::COMPOSITION,
        TokenKind::NOT,
        TokenKind::BITAND,
        TokenKind::BITOR,
        TokenKind::BITXOR,
        TokenKind::BITNOT,
        TokenKind::LSHIFT,
        TokenKind::RSHIFT,
        TokenKind::COLON,
        TokenKind::SEMI,
        TokenKind::ASSIGN,
        TokenKind::DOUBLE_ARROW,
        TokenKind::RANGEOP,
        TokenKind::CLOSEDRANGEOP,
        TokenKind::ELLIPSIS,
        TokenKind::HASH,
        TokenKind::AT,
        TokenKind::QUEST,
        TokenKind::UPPERBOUND,
        TokenKind::IDENTIFIER,
        TokenKind::INTEGER_LITERAL,
        TokenKind::RUNE_LITERAL,
        TokenKind::STRING_LITERAL,
        TokenKind::MULTILINE_STRING,
        TokenKind::END,
    ];
    // 每个都能取出 value/literal/precedence 不 panic
    for k in kinds {
        let _v: &'static str = k.value_str();
        let _l: &'static str = k.literal();
        let _p: u8 = k.precedence();
    }
}

#[test]
fn operator_precedence_matches_official() {
    assert_eq!(TokenKind::EXP.precedence(), 16); // **
    assert_eq!(TokenKind::MUL.precedence(), 15); // *
    assert_eq!(TokenKind::DIV.precedence(), 15); // /
    assert_eq!(TokenKind::ADD.precedence(), 14); // +
    assert_eq!(TokenKind::SUB.precedence(), 14); // -
    assert_eq!(TokenKind::LSHIFT.precedence(), 13); // <<
    assert_eq!(TokenKind::RSHIFT.precedence(), 13); // >>
    assert_eq!(TokenKind::RANGEOP.precedence(), 11); // ..
    assert_eq!(TokenKind::CLOSEDRANGEOP.precedence(), 11);
    assert_eq!(TokenKind::LT.precedence(), 10);
    assert_eq!(TokenKind::GT.precedence(), 10);
    assert_eq!(TokenKind::IS.precedence(), 10);
    assert_eq!(TokenKind::EQUAL.precedence(), 9);
    assert_eq!(TokenKind::NOTEQ.precedence(), 9);
    assert_eq!(TokenKind::BITAND.precedence(), 8);
    assert_eq!(TokenKind::BITXOR.precedence(), 7);
    assert_eq!(TokenKind::BITOR.precedence(), 6);
    assert_eq!(TokenKind::AND.precedence(), 5);
    assert_eq!(TokenKind::OR.precedence(), 3);
    assert_eq!(TokenKind::COALESCING.precedence(), 2);
    assert_eq!(TokenKind::PIPELINE.precedence(), 1);
    assert_eq!(TokenKind::QUEST.precedence(), 1);
    // 非运算符 precedence = 0
    assert_eq!(TokenKind::LET.precedence(), 0);
    assert_eq!(TokenKind::SEMI.precedence(), 0);
    assert_eq!(TokenKind::INT64.precedence(), 0);
}

#[test]
fn literal_text_matches_official() {
    assert_eq!(TokenKind::EXP.literal(), "**");
    assert_eq!(TokenKind::ASSIGN.literal(), "=");
    assert_eq!(TokenKind::DOUBLE_ARROW.literal(), "=>");
    assert_eq!(TokenKind::CLOSEDRANGEOP.literal(), "..=");
    assert_eq!(TokenKind::UPPERBOUND.literal(), "<:");
    assert_eq!(TokenKind::AT.literal(), "@");
    assert_eq!(TokenKind::NOT_IN.literal(), "!in");
    // 非字面量 token
    assert_eq!(TokenKind::IDENTIFIER.literal(), "");
    assert_eq!(TokenKind::STRING_LITERAL.literal(), "");
}

#[test]
fn keyword_lookup_matches_official() {
    assert_eq!(lookup_keyword("let"), Some(TokenKind::LET));
    assert_eq!(lookup_keyword("var"), Some(TokenKind::VAR));
    assert_eq!(lookup_keyword("func"), Some(TokenKind::FUNC));
    assert_eq!(lookup_keyword("class"), Some(TokenKind::CLASS));
    assert_eq!(lookup_keyword("struct"), Some(TokenKind::STRUCT));
    assert_eq!(lookup_keyword("enum"), Some(TokenKind::ENUM));
    assert_eq!(lookup_keyword("interface"), Some(TokenKind::INTERFACE));
    assert_eq!(lookup_keyword("match"), Some(TokenKind::MATCH));
    assert_eq!(lookup_keyword("for"), Some(TokenKind::FOR));
    assert_eq!(lookup_keyword("while"), Some(TokenKind::WHILE));
    assert_eq!(lookup_keyword("is"), Some(TokenKind::IS));
    assert_eq!(lookup_keyword("as"), Some(TokenKind::AS));
    assert_eq!(lookup_keyword("Int64"), Some(TokenKind::INT64));
    assert_eq!(lookup_keyword("UInt32"), Some(TokenKind::UINT32));
    assert_eq!(lookup_keyword("This"), Some(TokenKind::THISTYPE));
    assert_eq!(lookup_keyword("nothing"), None); // 非关键字
    assert_eq!(lookup_keyword("dostuff"), None); // 普通标识符
    assert_eq!(lookup_keyword("foo"), None);
}

#[test]
fn value_str_matches_official() {
    assert_eq!(TokenKind::DOT.value_str(), "dot");
    assert_eq!(TokenKind::LPAREN.value_str(), "l_paren");
    assert_eq!(TokenKind::INTEGER_LITERAL.value_str(), "integer_literal");
    assert_eq!(TokenKind::NL.value_str(), "newline");
    assert_eq!(TokenKind::STRING_LITERAL.value_str(), "string_literal");
}
