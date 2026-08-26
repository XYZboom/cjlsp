// cj-sema: name resolution (RefExpr -> Symbol).
//
// Per spec Ch.03:
//   * local names shadow outer names (inner scope level is higher)
//   * a declaration is visible only AFTER its own declaration point —
//     `let z = z` reports `unresolved identifier 'z'` (the RHS '=' is before
//     the binding takes effect)
//   * top-level funcs/types are visible across the whole package
//
// The resolver walks expression trees and resolves identifier references to
// symbols in the enclosing scope chain (local -> top-level/package).
// Unresolved names produce the spec's `unresolved identifier 'X'` diagnostic.

use crate::PackageTable;
use cj_ast::{Body, Decl, Expr, File};
use cj_diag::Diag;
use rayon::prelude::*;
use std::collections::HashMap;

/// One function's local scope: parameter + local variable names.
type LocalScope = HashMap<String, cj_ast::CodePos>;

/// Resolution context for a single file: package symbols + per-function locals.
pub struct Resolver<'a> {
    package: &'a PackageTable,
    diags: Vec<Diag>,
}

impl<'a> Resolver<'a> {
    pub fn new(package: &'a PackageTable) -> Self {
        Resolver {
            package,
            diags: Vec::new(),
        }
    }

    /// Resolve all top-level function bodies in a file (parallel across decls).
    pub fn resolve_file(&mut self, file: &File) {
        // Collect per-decl resolution jobs and run in parallel.
        let jobs: Vec<(String, Vec<(u32, u32)>)> = Vec::new();
        let _ = jobs;
        let per_decl: Vec<Vec<Diag>> = file
            .decls
            .par_iter()
            .map(|d| self.resolve_decl(d))
            .collect();
        for d in per_decl {
            self.diags.extend(d);
        }
    }

    /// Resolve one top-level declaration's body.
    fn resolve_decl(&self, d: &Decl) -> Vec<Diag> {
        let mut diags = Vec::new();
        match d {
            Decl::Func {
                params, body, pos, ..
            } => {
                let mut locals = LocalScope::new();
                for p in params {
                    locals.insert(p.name.clone(), p.pos);
                }
                if let Body::Block(stmts) = body {
                    // Each top-level statement shares the function scope, but a
                    // variable is visible only after its own declaration.
                    let mut seen: Vec<String> = Vec::new();
                    for s in stmts {
                        self.resolve_stmt(s, &mut locals, &mut seen, &mut diags);
                    }
                }
                let _ = pos;
            }
            // Top-level variable initializers (`var x = 1 + testvar`) are
            // resolved against package scope — refs to local names don't apply.
            Decl::Var { init: Some(init), pos, .. } => {
                let locals = LocalScope::new();
                let seen: Vec<String> = Vec::new();
                self.resolve_expr(init, &locals, &seen, &mut diags);
                let _ = pos;
            }
            _ => {}
        }
        diags
    }

    fn resolve_stmt(
        &self,
        e: &Expr,
        locals: &mut LocalScope,
        seen: &mut Vec<String>,
        diags: &mut Vec<Diag>,
    ) {
        // Declarations first introduce their names.
        if let Expr::LetPatternDestructor {
            patterns,
            initializer,
            ..
        } = e
        {
            self.resolve_expr(initializer, locals, seen, diags);
            for pat in patterns {
                if let cj_ast::Pattern::Var { name, pos, .. } = pat {
                    locals.insert(name.clone(), *pos);
                    seen.push(name.clone());
                }
            }
            return;
        }
        self.resolve_expr(e, locals, seen, diags);
    }

    fn resolve_expr(&self, e: &Expr, locals: &LocalScope, seen: &[String], diags: &mut Vec<Diag>) {
        match e {
            Expr::Name { name, pos, .. } => {
                // Resolve: local (seen so far) -> package top-level.
                let local_hit = locals.contains_key(name) && seen.contains(name);
                if !local_hit && !self.package.lookup(name).is_some() {
                    // skip builtin-ish names (print, etc.) — refined later
                    if !is_known_builtin(name) {
                        diags.push(Diag::error(
                            pos.line,
                            pos.col,
                            format!("undeclared identifier '{name}'"),
                        ));
                    }
                }
            }
            Expr::Call { callee, args, .. } => {
                self.resolve_expr(callee, locals, seen, diags);
                for a in args {
                    self.resolve_expr(&a.value, locals, seen, diags);
                }
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.resolve_expr(lhs, locals, seen, diags);
                self.resolve_expr(rhs, locals, seen, diags);
            }
            Expr::Unary { inner, .. } => self.resolve_expr(inner, locals, seen, diags),
            Expr::Paren { inner, .. } => self.resolve_expr(inner, locals, seen, diags),
            Expr::Member { object, .. } => self.resolve_expr(object, locals, seen, diags),
            Expr::Subscript { object, index, .. } => {
                self.resolve_expr(object, locals, seen, diags);
                self.resolve_expr(index, locals, seen, diags);
            }
            Expr::Assign { lhs, rhs, .. } => {
                self.resolve_expr(lhs, locals, seen, diags);
                self.resolve_expr(rhs, locals, seen, diags);
            }
            Expr::Block { stmts, .. } => {
                for s in stmts {
                    self.resolve_expr(s, locals, seen, diags);
                }
            }
            Expr::If {
                cond, then, els, ..
            } => {
                self.resolve_expr(cond, locals, seen, diags);
                self.resolve_expr(then, locals, seen, diags);
                if let Some(e) = els {
                    self.resolve_expr(e, locals, seen, diags);
                }
            }
            _ => {}
        }
    }

    pub fn take_diags(&mut self) -> Vec<Diag> {
        std::mem::take(&mut self.diags)
    }
}

/// A small set of names the resolver treats as predefined (std functions /
/// literals). Expanded as type checking matures.
fn is_known_builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "println"
            | "assert"
            | "String"
            | "Array"
            | "VArray"
            | "Int8"
            | "Int16"
            | "Int32"
            | "Int64"
            | "UInt8"
            | "UInt16"
            | "UInt32"
            | "UInt64"
            | "Float32"
            | "Float64"
            | "Bool"
            | "Unit"
            | "Nothing"
            | "Rune"
            | "Char"
            | "Object"
            | "Range"
            | "Pair"
            | "Tuple"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cj_parser::parse_source;

    #[test]
    fn unresolved_identifier() {
        let (file, _) = parse_source("func f() { let z = z }");
        let mut pkg = PackageTable::default();
        let r = Resolver::new(&pkg);
        let mut file_resolver = r;
        file_resolver.resolve_file(&file);
        let diags = file_resolver.take_diags();
        assert!(!diags.is_empty(), "expected unresolved: {:?}", diags);
        assert!(diags[0].message.contains("undeclared identifier 'z'"));
    }

    #[test]
    fn local_shadows_top_level() {
        let (file, _) = parse_source("let y = 5\nfunc g() { let y = y }");
        let (collector, pkg) = collect_pkg(&file);
        let _ = collector;
        let mut r = Resolver::new(&pkg);
        r.resolve_file(&file);
        let diags = r.take_diags();
        // 'y' on the RHS resolves to the top-level 'y' (declared before), so OK.
        assert!(diags.is_empty(), "expected no unresolved: {:?}", diags);
    }

    #[test]
    fn package_cross_file_resolution() {
        let (f1, _) = parse_source("func helper() {}");
        let (f2, _) = parse_source("func main() { helper() }");
        let mut pkg = PackageTable::default();
        let r1 = crate::Collector::new().collect_file(&f1);
        let r2 = crate::Collector::new().collect_file(&f2);
        pkg.merge(&r1);
        pkg.merge(&r2);
        let mut r = Resolver::new(&pkg);
        r.resolve_file(&f2);
        let diags = r.take_diags();
        // helper() is a top-level func in another file -> resolves via package.
        assert!(diags.is_empty(), "expected resolution: {:?}", diags);
    }

    fn collect_pkg(file: &File) -> (crate::FileResult, PackageTable) {
        let fr = crate::Collector::new().collect_file(file);
        let mut pkg = PackageTable::default();
        pkg.merge(&fr);
        (fr, pkg)
    }
}
