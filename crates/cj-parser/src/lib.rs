// cj-parser: Cangjie parser (token stream -> AST).

pub mod decl;
pub mod expr;
pub mod parser;
pub mod ty;

pub use parser::{modifier_display, parse_source, token_display_text, Diag, Parser};

#[cfg(test)]
mod tests {
    use super::*;
    use cj_ast::*;
    use cj_lexer::Lexer;

    fn parse(src: &str) -> (File, Vec<Diag>) {
        let tokens = Lexer::new(src).tokenize();
        let mut p = Parser::new(src, tokens);
        let file = p.run();
        (file, p.diags)
    }

    #[test]
    fn parse_simple_func() {
        let (file, diags) = parse("func main() { println(\"hi\") }");
        assert!(diags.is_empty(), "diags: {:?}", diags);
        assert_eq!(file.decls.len(), 1);
        match &file.decls[0] {
            Decl::Func { name, .. } => assert_eq!(name, "main"),
            other => panic!("expected Func, got {:?}", other),
        }
    }

    #[test]
    fn parse_var_decl() {
        let (file, diags) = parse("let x: Int64 = 42");
        assert!(diags.is_empty(), "diags: {:?}", diags);
        match &file.decls[0] {
            Decl::Var {
                name,
                is_mutable,
                ty,
                init,
                ..
            } => {
                assert_eq!(name, "x");
                assert!(!is_mutable);
                assert!(ty.is_some());
                assert!(init.is_some());
            }
            other => panic!("expected Var, got {:?}", other),
        }
    }

    #[test]
    fn parse_binary_expr() {
        let (file, diags) = parse("func f() { let y = 1 + 2 * 3 }");
        assert!(diags.is_empty(), "diags: {:?}", diags);
        let decl = &file.decls[0];
        if let Decl::Func {
            body: Body::Block(stmts),
            ..
        } = decl
        {
            assert_eq!(stmts.len(), 1);
            // let y = 1 + 2*3  -> top-level Binary(Add, 1, Binary(Mul,2,3))
        } else {
            panic!("expected Func");
        }
    }

    #[test]
    fn parse_class() {
        let (file, diags) = parse("public class A <: B { let x = 1 }");
        assert!(diags.is_empty(), "diags: {:?}", diags);
        match &file.decls[0] {
            Decl::Class {
                name,
                parents,
                members,
                ..
            } => {
                assert_eq!(name, "A");
                assert_eq!(parents.len(), 1);
                assert_eq!(members.len(), 1);
            }
            other => panic!("expected Class, got {:?}", other),
        }
    }

    #[test]
    fn parse_if_while() {
        let (_file, diags) = parse("func f() { if (x) { 1 } else { 2 } while (y) { 3 } }");
        assert!(diags.is_empty(), "diags: {:?}", diags);
    }

    #[test]
    fn parse_errors_collected() {
        let (_, diags) = parse("func f( { }");
        assert!(!diags.is_empty(), "expected parse errors");
        assert!(diags.iter().any(|diag| diag.code.is_some()));
        assert!(diags
            .iter()
            .filter(|diag| diag.code.is_some())
            .all(|diag| diag.category == Some("parser")));
    }

    #[test]
    fn parse_package_import() {
        let (file, _) = parse("package demo\nimport std.collection.*\nfunc main() {}");
        assert_eq!(file.package.as_deref(), Some("demo"));
        assert_eq!(file.imports.len(), 1);
        assert!(file.imports[0].glob);
    }

    #[test]
    fn expression_diagnostics_match_message_position_and_count() {
        struct Case {
            name: &'static str,
            src: &'static str,
            expected: &'static [(&'static str, u32, u32, u32)],
        }

        let cases = [
            Case {
                name: "wildcard expression",
                src: "main() {\n    _()\n}",
                expected: &[("unexpected _ wildcard", 2, 5, 6)],
            },
            Case {
                name: "wildcards in ordinary tuple expression",
                src: "main() {\n    (_, _) << 1\n}",
                expected: &[
                    ("unexpected _ wildcard", 2, 6, 7),
                    ("unexpected _ wildcard", 2, 9, 10),
                ],
            },
            Case {
                name: "invalid assignment target",
                src: "main() {\n    a + b = 1\n}",
                expected: &[("invalid left-hand expression of assignment '='", 2, 11, 12)],
            },
            Case {
                name: "invalid compound assignment target",
                src: "main() {\n    1 <<= 2\n}",
                expected: &[("invalid left-hand expression of assignment '<<='", 2, 7, 10)],
            },
            Case {
                name: "assignment chain",
                src: "main() {\n    a = b = c\n}",
                expected: &[("assignment operators cannot be chained", 2, 11, 12)],
            },
            Case {
                name: "local initializer assignment",
                src: "main() {\n    var a = b = 1\n}",
                expected: &[(
                    "cannot have assignment expression in initializer",
                    2,
                    13,
                    18,
                )],
            },
            Case {
                name: "global initializer assignment",
                src: "var a = b += 1",
                expected: &[("cannot have assignment expression in initializer", 1, 9, 15)],
            },
            Case {
                name: "default value assignment",
                src: "func f(a!: Int64 = b = 1) {}",
                expected: &[(
                    "cannot have assignment expression in initializer",
                    1,
                    20,
                    25,
                )],
            },
            Case {
                name: "invalid increment target",
                src: "var a = 1 ++",
                expected: &[("cannot increment a un-assignable expression", 1, 11, 13)],
            },
            Case {
                name: "invalid prefix decrement target",
                src: "var a = --1",
                expected: &[("cannot decrement a un-assignable expression", 1, 9, 11)],
            },
        ];

        for case in cases {
            let (_, diags) = parse(case.src);
            let actual: Vec<_> = diags
                .iter()
                .map(|diag| (diag.message.as_str(), diag.line, diag.col, diag.end_col))
                .collect();
            assert_eq!(actual, case.expected, "case: {}", case.name);
        }
    }

    #[test]
    fn wildcard_patterns_and_assignable_expressions_are_accepted() {
        let cases = [
            (
                "pattern destructuring",
                "func f() { let (_, x) = (1, 2); match (x) { case _ => 0 } }",
            ),
            (
                "tuple and discard assignment",
                "main() {\n    var a = 1\n    var b = 2\n    (_, a) = (b, 3)\n    (a, _) = (4, 5)\n    _ = a\n}",
            ),
            (
                "member subscript and step targets",
                "main() { var a = [1]; a[0] = 2; a[0]++; --a[0] }",
            ),
            (
                "ordinary initializer and default",
                "func f(a!: Int64 = 1) { let x = 2 }",
            ),
        ];

        for (name, src) in cases {
            let (_, diags) = parse(src);
            assert!(diags.is_empty(), "case {name}: {diags:?}");
        }
    }

    #[test]
    fn unclosed_delimiter_diagnostics_and_recovery() {
        struct Case {
            name: &'static str,
            src: &'static str,
            expected: &'static [(cj_diag::DiagId, &'static str, u32, u32)],
        }

        let cases = [
            Case {
                name: "unclosed paren in expression",
                src: "main() {\n    let x = (1 + 2\n    let y = 3\n}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_RIGHT_DELIMITER,
                    "unclosed delimiter: '('",
                    2,
                    14,
                )],
            },
            Case {
                name: "unclosed paren in call args",
                src: "main() {\n    foo(1, 2\n    let y = 3\n}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_RIGHT_DELIMITER,
                    "unclosed delimiter: '('",
                    2,
                    9,
                )],
            },
            Case {
                name: "unclosed bracket in array literal",
                src: "main() {\n    let a = [1, 2\n    let b = 3\n}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_RIGHT_DELIMITER,
                    "unclosed delimiter: '['",
                    2,
                    14,
                )],
            },
            Case {
                name: "unclosed bracket in subscript",
                src: "main() {\n    let x = arr[1\n    let y = 2\n}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_RIGHT_DELIMITER,
                    "unclosed delimiter: '['",
                    2,
                    17,
                )],
            },
            Case {
                name: "unclosed curly brace in block",
                src: "func f() {\n    let x = 1\n",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_RIGHT_DELIMITER,
                    "unclosed delimiter: '{'",
                    1,
                    11,
                )],
            },
            Case {
                name: "bare dollar without identifier or lparen",
                src: "main() {\n    let s = $\n    let y = 3\n}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECT_ESCAPE_DOLLAR_TOKEN,
                    "expected identifier or '(' after '$'",
                    2,
                    13,
                )],
            },
        ];

        for case in cases {
            let (file, diags) = parse(case.src);
            let actual: Vec<_> = diags
                .iter()
                .map(|diag| {
                    (
                        diag.code.unwrap_or(""),
                        diag.message.as_str(),
                        diag.line,
                        diag.col,
                    )
                })
                .collect();
            let expected: Vec<_> = case
                .expected
                .iter()
                .map(|(id, msg, line, col)| (id.code(), *msg, *line, *col))
                .collect();
            assert_eq!(actual, expected, "case: {}", case.name);
            assert!(
                !file.decls.is_empty(),
                "case {} should recover AST",
                case.name
            );
        }
    }

    #[test]
    fn declaration_and_top_level_error_diagnostics() {
        struct Case {
            name: &'static str,
            src: &'static str,
            expected: &'static [(cj_diag::DiagId, &'static str, u32, u32)],
        }

        let cases = [
            Case {
                name: "class missing name",
                src: "class { }\nfunc next() {}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_NAME,
                    "expected class name, found '{'",
                    1,
                    7,
                )],
            },
            Case {
                name: "struct missing name",
                src: "struct { }\nfunc next() {}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_NAME,
                    "expected struct name, found '{'",
                    1,
                    8,
                )],
            },
            Case {
                name: "enum missing name",
                src: "enum { A }\nfunc next() {}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_NAME,
                    "expected enum name, found '{'",
                    1,
                    6,
                )],
            },
            Case {
                name: "interface missing name",
                src: "interface { }\nfunc next() {}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_NAME,
                    "expected interface name, found '{'",
                    1,
                    11,
                )],
            },
            Case {
                name: "func missing name",
                src: "func () {}\nfunc next() {}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_NAME,
                    "expected func name, found '('",
                    1,
                    6,
                )],
            },
            Case {
                name: "var missing identifier or pattern",
                src: "var = 1\nfunc next() {}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_ONE_OF_IDENTIFIER_OR_PATTERN,
                    "expected identifier or pattern after 'var', found '='",
                    1,
                    5,
                )],
            },
            Case {
                name: "top level statement without decl",
                src: "1 + 1\nfunc next() {}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_DECL,
                    "expected declaration, found literal '1'",
                    1,
                    1,
                )],
            },
        ];

        for case in cases {
            let (file, diags) = parse(case.src);
            let actual: Vec<_> = diags
                .iter()
                .map(|diag| {
                    (
                        diag.code.unwrap_or(""),
                        diag.message.as_str(),
                        diag.line,
                        diag.col,
                    )
                })
                .collect();
            let expected: Vec<_> = case
                .expected
                .iter()
                .map(|(id, msg, line, col)| (id.code(), *msg, *line, *col))
                .collect();
            assert_eq!(actual, expected, "case: {}", case.name);
            assert!(
                file.decls.iter().any(|d| match d {
                    Decl::Func { name, .. } => name == "next",
                    _ => false,
                }),
                "case {} should recover and parse following declarations",
                case.name
            );
        }
    }

    #[test]
    fn control_flow_missing_condition_and_delimiter_diagnostics() {
        struct Case {
            name: &'static str,
            src: &'static str,
            expected: &'static [(cj_diag::DiagId, &'static str, u32, u32)],
        }

        let cases = [
            Case {
                name: "if missing condition left paren",
                src: "main() {\n    if { }\n}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_LEFT_PAREN_AFTER,
                    "expected '(' after 'if', found '{'",
                    2,
                    8,
                )],
            },
            Case {
                name: "while missing condition left paren",
                src: "main() {\n    while { }\n}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_LEFT_PAREN_AFTER,
                    "expected '(' after 'while', found '{'",
                    2,
                    11,
                )],
            },
            Case {
                name: "if unclosed condition right paren",
                src: "main() {\n    if (x {\n        1\n    }\n}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_RIGHT_DELIMITER,
                    "unclosed delimiter: '('",
                    2,
                    9,
                )],
            },
            Case {
                name: "while unclosed condition right paren",
                src: "main() {\n    while (x {\n        1\n    }\n}",
                expected: &[(
                    cj_diag::DiagId::PARSE_EXPECTED_RIGHT_DELIMITER,
                    "unclosed delimiter: '('",
                    2,
                    12,
                )],
            },
        ];

        for case in cases {
            let (_, diags) = parse(case.src);
            let actual: Vec<_> = diags
                .iter()
                .map(|diag| {
                    (
                        diag.code.unwrap_or(""),
                        diag.message.as_str(),
                        diag.line,
                        diag.col,
                    )
                })
                .collect();
            let expected: Vec<_> = case
                .expected
                .iter()
                .map(|(id, msg, line, col)| (id.code(), *msg, *line, *col))
                .collect();
            assert_eq!(actual, expected, "case: {}", case.name);
        }
    }
}
