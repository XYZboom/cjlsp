// cj-sema: semantic analysis — symbol table + collector.
//
// Behavioral reference: cangjie_compiler/src/Sema/Collector.cpp (scope-based
// symbol collection, redefinition detection). Minimal first milestone: collect
// top-level + class-body declarations into scoped symbol tables and report
// `sema_redefinition` for duplicate names, matching cjc output.

use cj_ast::{CodePos, Decl, File};
use cj_diag::Diag;
use std::collections::HashMap;

/// Kind of a collected symbol (for future resolution / LSP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Func,
    Class,
    Interface,
    Struct,
    Enum,
    TypeAlias,
    Var,
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
            _ => return None, // decls without a namespace name (extend, ctor, ...)
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
}

/// Collects declarations from a File into scopes, emitting diagnostics.
#[derive(Debug, Default)]
pub struct Collector {
    pub table: SymbolTable,
    pub diags: Vec<Diag>,
}

impl Collector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Collect a whole file: one top-level scope, then class bodies recurse.
    pub fn collect_file(&mut self, file: &File) {
        self.table.push_scope();
        for d in &file.decls {
            self.collect_decl(d);
        }
        self.table.pop_scope();
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
            let mut diag = Diag::error(
                pos.line,
                pos.col,
                format!("redefinition of declaration '{}'", name),
            )
            .with_span(pos.end_line, pos.end_col)
            .with_note(format!("'{}' is previously declared here", prev.name));
            // official attaches the note at the previous declaration's location
            // (our Diag model attaches notes without positions; keep the text).
            let _ = &mut diag;
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
        let mut c = Collector::new();
        c.collect_file(&file);
        assert!(!c.diags.is_empty(), "expected redefinition diag");
        assert!(c.diags[0]
            .message
            .contains("redefinition of declaration 'x'"));
    }

    #[test]
    fn distinct_names_ok() {
        let (file, _) = parse_source("let x = 1\nlet y = 2\n");
        let mut c = Collector::new();
        c.collect_file(&file);
        assert!(
            c.diags.is_empty(),
            "no redefinition expected: {:?}",
            c.diags
        );
    }

    #[test]
    fn class_member_redefinition() {
        let (file, _) = parse_source("class A { let x = 1; let x = 2 }");
        let mut c = Collector::new();
        c.collect_file(&file);
        assert!(!c.diags.is_empty(), "expected member redefinition");
    }
}
