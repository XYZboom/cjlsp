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
}
