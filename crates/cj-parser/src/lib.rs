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
        let (file, diags) = parse("func f() { if (x) { 1 } else { 2 } while (y) { 3 } }");
        assert!(diags.is_empty(), "diags: {:?}", diags);
    }

    #[test]
    fn parse_errors_collected() {
        let (_, diags) = parse("func f( { }");
        assert!(!diags.is_empty(), "expected parse errors");
    }

    #[test]
    fn parse_package_import() {
        let (file, _) = parse("package demo\nimport std.collection.*\nfunc main() {}");
        assert_eq!(file.package.as_deref(), Some("demo"));
        assert_eq!(file.imports.len(), 1);
        assert!(file.imports[0].glob);
    }
}
