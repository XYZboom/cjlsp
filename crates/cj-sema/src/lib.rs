// cj-sema: semantic analysis — symbol table + parallel file collector.
//
// Design follows the Cangjie language spec (Chapter 03 — 名字、作用域、变量):
//   * keywords/vars/funcs/types/generic params/packages share ONE namespace per
//     scope; no same-name decls allowed except overloads
//   * top-level funcs/types are visible across the whole package
//   * top-level vars are visible from their definition point onward (order
//     matters — creates a dependency graph for initialization)
//   * local names shadow outer names; inner scopes have higher level
//
// For parallelism, collection is per-file (no cross-file state in the collector):
// each worker builds its file's local scope; the merged package table resolves
// cross-file references. Dependency analysis then runs over the merged table.

use cj_ast::{CodePos, Decl, File};
use cj_diag::Diag;
use std::collections::HashMap;

pub mod checks;
pub mod dep_graph;
pub mod expander;
pub mod macro_cache;
pub mod overload;
pub mod package;
pub mod parallel;
pub mod resolver;
pub mod typecheck;
pub mod unused;

/// Kind of a collected symbol (for resolution / LSP).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Func,
    Class,
    Interface,
    Struct,
    Enum,
    TypeAlias,
    Var,
    Const,
    Prop,
    Package,
}

impl SymbolKind {
    fn from_decl(d: &Decl) -> Option<SymbolKind> {
        Some(match d {
            Decl::Func { .. } => SymbolKind::Func,
            Decl::Class { .. } => SymbolKind::Class,
            Decl::Interface { .. } => SymbolKind::Interface,
            Decl::Struct { .. } => SymbolKind::Struct,
            Decl::Enum { .. } => SymbolKind::Enum,
            Decl::TypeAlias { .. } => SymbolKind::TypeAlias,
            Decl::Var { .. } => SymbolKind::Var,
            Decl::Prop { .. } => SymbolKind::Prop,
            Decl::Package { .. } => SymbolKind::Package,
            _ => return None,
        })
    }
}

/// A named declaration in a scope.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub pos: CodePos,
}

/// A single function parameter signature (for cross-file call checks).
#[derive(Debug, Clone)]
pub struct ParamSig {
    pub name: String,
    /// named-call parameter (`a!: T` — must be passed with a name prefix)
    pub is_named: bool,
    pub ty: cj_ast::Type,
}

/// A function's signature: name + parameter signature + return type
/// (for cross-file call checks).
#[derive(Debug, Clone, Default)]
pub struct FuncSig {
    pub name: String,
    pub params: Vec<ParamSig>,
    pub ret: Option<cj_ast::Type>,
    pub pos: CodePos,
}

/// A lexical scope (name -> symbol).
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub symbols: HashMap<String, Symbol>,
}

/// Stack of scopes; the top is the innermost (current) scope.
#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    scopes: Vec<Scope>,
}

impl SymbolTable {
    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::default());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Insert a symbol into the current scope. Returns the previous symbol if
    /// a same-named declaration already exists there.
    pub fn declare(&mut self, sym: Symbol) -> Option<Symbol> {
        let scope = self.scopes.last_mut()?;
        // HashMap::insert returns the previous value bound to the key, if any.
        scope.symbols.insert(sym.name.clone(), sym)
    }

    /// Look up a name in the current scope only.
    pub fn lookup_local(&self, name: &str) -> Option<&Symbol> {
        self.scopes.last()?.symbols.get(name)
    }

    /// Look up a name walking from innermost to outermost scope.
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.scopes.iter().rev().find_map(|s| s.symbols.get(name))
    }

    /// All symbols declared at the outermost (package) scope.
    pub fn top_level_symbols(&self) -> Vec<Symbol> {
        self.scopes
            .first()
            .map(|s| s.symbols.values().cloned().collect())
            .unwrap_or_default()
    }
}

/// Result of collecting one file: its local top-level symbols and diagnostics.
/// This is the unit of parallelism — each file is collected independently.
#[derive(Debug, Default)]
pub struct FileResult {
    /// Top-level symbols declared in this file (package-visible).
    pub symbols: Vec<Symbol>,
    /// Top-level function signatures (name -> param types), package-visible.
    pub func_sigs: HashMap<String, FuncSig>,
    pub diags: Vec<Diag>,
}

/// Collects declarations from one File into a local scope, emitting
/// diagnostics for same-scope redefinition (spec: shared namespace).
#[derive(Debug, Default)]
pub struct Collector {
    table: SymbolTable,
    diags: Vec<Diag>,
}

impl Collector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Collect a whole file: one top-level scope, then class bodies recurse.
    pub fn collect_file(mut self, file: &File) -> FileResult {
        self.table.push_scope();
        let mut func_sigs = HashMap::new();
        for d in &file.decls {
            // Record top-level function signatures for cross-file call checks.
            if let Decl::Func {
                name,
                params,
                ret,
                pos,
                ..
            } = d
            {
                func_sigs.entry(name.clone()).or_insert_with(|| FuncSig {
                    name: name.clone(),
                    params: params
                        .iter()
                        .map(|p| ParamSig {
                            name: p.name.clone(),
                            is_named: p.is_named,
                            ty: p.ty.clone(),
                        })
                        .collect(),
                    ret: ret.clone(),
                    pos: *pos,
                });
            }
            self.collect_decl(d);
        }
        let symbols = self.table.top_level_symbols();
        FileResult {
            symbols,
            func_sigs,
            diags: std::mem::take(&mut self.diags),
        }
    }

    /// Collect one declaration: declare its name (reporting redefinition),
    /// then recurse into members (class/struct/enum/interface bodies).
    fn collect_decl(&mut self, d: &Decl) {
        let Some(kind) = SymbolKind::from_decl(d) else {
            return;
        };
        let name = decl_name(d).unwrap_or_default();
        // Report redefinition at the *name* (not the decl start keyword) —
        // official reports `let zzzz` at the variable/function name position.
        let pos = decl_name_pos(d).unwrap_or_else(|| decl_pos(d));
        let sym = Symbol {
            name: name.clone(),
            kind,
            pos,
        };
        if let Some(prev) = self.table.declare(sym) {
            // Functions may share a scope (overloading, spec Ch.10); a
            // redefinition is only reported for non-function decls. Identical
            // signatures are flagged separately (overload.rs).
            if !(kind == SymbolKind::Func && prev.kind == SymbolKind::Func) {
                let diag = Diag::error(
                    pos.line,
                    pos.col,
                    format!("redefinition of declaration '{name}'"),
                )
                .with_span(pos.end_line, pos.end_col)
                .with_note(format!("'{}' is previously declared here", prev.name));
                self.diags.push(diag);
            }
        }
        // Function parameters share the function's own scope: duplicate
        // parameter names are redefinitions too (official func_error cases).
        if let Decl::Func { params, .. } = d {
            let mut seen: std::collections::HashMap<&str, &cj_ast::Param> =
                std::collections::HashMap::new();
            for p in params {
                if let Some(prev) = seen.insert(p.name.as_str(), p) {
                    let d = Diag::error(
                        p.pos.line,
                        p.pos.col,
                        format!("redefinition of declaration '{}'", p.name),
                    )
                    .with_span(p.pos.end_line, p.pos.end_col)
                    .with_note(format!("'{}' is previously declared here", prev.name));
                    self.diags.push(d);
                }
            }
        }
        // recurse into members
        match d {
            Decl::Class { members, .. }
            | Decl::Interface { members, .. }
            | Decl::Struct { members, .. } => {
                self.table.push_scope();
                for m in members {
                    self.collect_decl(m);
                }
                self.table.pop_scope();
            }
            _ => {}
        }
    }
}

/// Package-level symbol table: merged from all files' top-level symbols.
/// This is where cross-file name resolution happens (spec: top-level names are
/// visible across the whole package).
#[derive(Debug, Default)]
pub struct PackageTable {
    /// name -> symbols declared at package level across all files.
    pub symbols: HashMap<String, Vec<Symbol>>,
    /// name -> function signature declared at package level across all files.
    pub func_sigs: HashMap<String, FuncSig>,
}

impl PackageTable {
    pub fn merge(&mut self, file_result: &FileResult) {
        for sym in &file_result.symbols {
            self.symbols
                .entry(sym.name.clone())
                .or_default()
                .push(sym.clone());
        }
        for (name, sig) in &file_result.func_sigs {
            self.func_sigs
                .entry(name.clone())
                .or_insert_with(|| sig.clone());
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.symbols.get(name)?.first()
    }

    /// Look up a top-level function signature by name (cross-file).
    pub fn lookup_func(&self, name: &str) -> Option<&FuncSig> {
        self.func_sigs.get(name)
    }
}

/// Extract the namespace name of a declaration.
fn decl_name(d: &Decl) -> Option<String> {
    Some(match d {
        Decl::Func { name, .. }
        | Decl::Macro { name, .. }
        | Decl::Class { name, .. }
        | Decl::Interface { name, .. }
        | Decl::Enum { name, .. }
        | Decl::Struct { name, .. }
        | Decl::TypeAlias { name, .. }
        | Decl::Builtin { name, .. }
        | Decl::Var { name, .. }
        | Decl::Prop { name, .. }
        | Decl::Package { name, .. }
        | Decl::MacroExpand { name, .. } => name.clone(),
        _ => return None,
    })
}

/// Extract the position of a declaration.
fn decl_pos(d: &Decl) -> CodePos {
    use Decl::*;
    match d {
        Func { pos, .. }
        | Macro { pos, .. }
        | Class { pos, .. }
        | Interface { pos, .. }
        | Extend { pos, .. }
        | Enum { pos, .. }
        | Struct { pos, .. }
        | TypeAlias { pos, .. }
        | PrimaryCtor { pos, .. }
        | Builtin { pos, .. }
        | Var { pos, .. }
        | Prop { pos, .. }
        | FuncParam { pos, .. }
        | VarWithPattern { pos, .. }
        | GenericParam { pos, .. }
        | Package { pos, .. }
        | MacroExpand { pos, .. } => *pos,
        Main { pos, .. } => *pos,
        Invalid(pos) => *pos,
    }
}

/// Extract the position of a declaration's *name* token (where the official
/// compiler anchors redefinition diagnostics): `let zzzz` -> `zzzz`, not `let`.
fn decl_name_pos(d: &Decl) -> Option<CodePos> {
    use Decl::*;
    match d {
        Func { name_pos, .. }
        | Class { name_pos, .. }
        | Interface { name_pos, .. }
        | Enum { name_pos, .. }
        | Struct { name_pos, .. }
        | Var { name_pos, .. } => Some(*name_pos),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cj_parser::parse_source;

    #[test]
    fn top_level_redefinition() {
        let (file, _) = parse_source("let x = 1\nlet x = 2\n");
        let c = Collector::new();
        let r = c.collect_file(&file);
        assert!(!r.diags.is_empty(), "expected redefinition diag");
        assert!(r.diags[0]
            .message
            .contains("redefinition of declaration 'x'"));
        // Official anchors at the *name* (line 2 col 5), not the `let` keyword.
        assert_eq!(r.diags[0].line, 2);
        assert_eq!(r.diags[0].col, 5);
    }

    #[test]
    fn duplicate_param_redefinition() {
        let (file, _) = parse_source("func f(a: Int8, a: Bool) {}\n");
        let c = Collector::new();
        let r = c.collect_file(&file);
        assert!(
            r.diags
                .iter()
                .any(|d| d.message.contains("redefinition of declaration 'a'")),
            "{:?}",
            r.diags
        );
        // anchored at the second `a` (line 1 col 17)
        assert!(
            r.diags.iter().any(|d| d.line == 1 && d.col == 17),
            "{:?}",
            r.diags
        );
    }

    #[test]
    fn distinct_names_ok() {
        let (file, _) = parse_source("let x = 1\nlet y = 2\n");
        let c = Collector::new();
        let r = c.collect_file(&file);
        assert!(
            r.diags.is_empty(),
            "no redefinition expected: {:?}",
            r.diags
        );
    }

    #[test]
    fn class_member_redefinition() {
        let (file, _) = parse_source("class A { let x = 1; let x = 2 }");
        let c = Collector::new();
        let r = c.collect_file(&file);
        assert!(!r.diags.is_empty(), "expected member redefinition");
    }

    #[test]
    fn package_merge_cross_file() {
        // Simulate two files in one package: top-level names merge.
        let (f1, _) = parse_source("func a() {}");
        let (f2, _) = parse_source("func b() {}");
        let r1 = Collector::new().collect_file(&f1);
        let r2 = Collector::new().collect_file(&f2);
        let mut pkg = PackageTable::default();
        pkg.merge(&r1);
        pkg.merge(&r2);
        assert!(pkg.lookup("a").is_some());
        assert!(pkg.lookup("b").is_some());
    }
}
