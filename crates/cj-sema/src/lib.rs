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

pub mod parallel;

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
        for d in &file.decls {
            self.collect_decl(d);
        }
        let symbols = self.table.top_level_symbols();
        FileResult {
            symbols,
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
        let pos = decl_pos(d);
        let sym = Symbol {
            name: name.clone(),
            kind,
            pos,
        };
        if let Some(prev) = self.table.declare(sym) {
            let diag = Diag::error(
                pos.line,
                pos.col,
                format!("redefinition of declaration '{name}'"),
            )
            .with_span(pos.end_line, pos.end_col)
            .with_note(format!("'{}' is previously declared here", prev.name));
            self.diags.push(diag);
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
}

impl PackageTable {
    pub fn merge(&mut self, file_result: &FileResult) {
        for sym in &file_result.symbols {
            self.symbols
                .entry(sym.name.clone())
                .or_default()
                .push(sym.clone());
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.symbols.get(name)?.first()
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
