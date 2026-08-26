// cj-sema: dependency analysis over top-level declarations.
//
// Per spec Ch.03 (Top-level scope rules):
//   * top-level funcs/types are visible across the whole package;
//   * top-level variables are visible only AFTER their definition completes —
//     `let x = y` requires `y` to be defined before `x` is initialized.
//   * `let c = c` is an error (self-reference in initializer).
//
// This builds a dependency graph: nodes = top-level declarations; an edge
// a -> b means "a's initializer references b". Then:
//   * topological order detection — declarations must be processed in a valid
//     order (used to schedule parallel type-checking waves);
//   * cycle detection — mutual/self initialization is an error.
//
// The dependency extraction itself runs per-declaration in parallel (each
// initializer is scanned independently), matching the multi-threaded semantic
// analysis requirement.

use cj_ast::{Decl, Expr, File};
use cj_diag::Diag;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

/// A node in the dependency graph: one top-level declaration.
#[derive(Debug, Clone)]
pub struct DepNode {
    pub name: String,
    /// Names referenced by this declaration's initializer/body.
    pub deps: Vec<String>,
    /// Position of the declaration (for diagnostics).
    pub line: u32,
    pub col: u32,
}

/// Dependency graph over a set of top-level declarations.
#[derive(Debug, Default)]
pub struct DepGraph {
    /// name -> node
    pub nodes: HashMap<String, DepNode>,
}

impl DepGraph {
    /// Build from parsed files. Extraction is parallel per-file.
    pub fn build(files: &[&File]) -> Self {
        // Collect (name, deps) per file in parallel.
        type FileNodes = Vec<Vec<(String, Vec<String>, u32, u32)>>;
        let per_file: FileNodes = files
            .par_iter()
            .map(|file| {
                file.decls
                    .iter()
                    .filter_map(|d| {
                        let name = decl_name(d)?;
                        let (line, col) = decl_pos(d);
                        let deps = extract_deps(d);
                        Some((name, deps, line, col))
                    })
                    .collect()
            })
            .collect();

        let mut graph = DepGraph::default();
        for file_nodes in per_file {
            for (name, deps, line, col) in file_nodes {
                graph.nodes.insert(
                    name.clone(),
                    DepNode {
                        name,
                        deps,
                        line,
                        col,
                    },
                );
            }
        }
        graph
    }

    /// Detect dependency cycles among top-level variable initializers.
    /// Returns diagnostics for each cycle found (spec: `let a = b; let b = a`
    /// and `let c = c` are errors).
    pub fn detect_cycles(&self) -> Vec<Diag> {
        let mut diags = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut in_stack: HashSet<String> = HashSet::new();
        let mut stack: Vec<String> = Vec::new();

        for name in self.nodes.keys() {
            if !visited.contains(name) {
                self.dfs_cycle(name, &mut visited, &mut in_stack, &mut stack, &mut diags);
            }
        }
        diags
    }

    fn dfs_cycle(
        &self,
        name: &str,
        visited: &mut HashSet<String>,
        in_stack: &mut HashSet<String>,
        stack: &mut Vec<String>,
        diags: &mut Vec<Diag>,
    ) {
        if in_stack.contains(name) {
            // Found a cycle: report the part of the stack from name onward.
            let pos = stack.iter().position(|n| n == name).unwrap_or(0);
            let cycle: Vec<&str> = stack[pos..].iter().map(|s| s.as_str()).collect();
            if let Some(node) = self.nodes.get(name) {
                let msg = format!("circular reference involving '{}'", cycle.join("' and '"));
                diags.push(Diag::error(node.line, node.col, msg));
            }
            return;
        }
        if visited.contains(name) {
            return;
        }
        visited.insert(name.to_string());
        in_stack.insert(name.to_string());
        stack.push(name.to_string());

        if let Some(node) = self.nodes.get(name) {
            for dep in &node.deps {
                // Only follow deps that are themselves top-level decls (others
                // resolve to builtins/imports and are ignored for cycle).
                if self.nodes.contains_key(dep) {
                    self.dfs_cycle(dep, visited, in_stack, stack, diags);
                }
            }
        }

        stack.pop();
        in_stack.remove(name);
    }

    /// Topological order of declaration names (a valid evaluation order).
    /// Returns None if the graph has a cycle.
    pub fn topo_order(&self) -> Option<Vec<String>> {
        // indeg[name] = how many deps this node still has unmet.
        let mut indeg: HashMap<&str, usize> = HashMap::new();
        for name in self.nodes.keys() {
            indeg.insert(name.as_str(), self.nodes[name].deps.len());
        }
        // Kahn's algorithm: start with nodes that have 0 deps.
        let mut ready: Vec<String> = self
            .nodes
            .keys()
            .filter(|k| indeg[k.as_str()] == 0)
            .cloned()
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(name) = ready.pop() {
            order.push(name.clone());
            // Every node that depends on `name` has one less unmet dep.
            for (other, node) in &self.nodes {
                if node.deps.contains(&name) {
                    if let Some(c) = indeg.get_mut(other.as_str()) {
                        *c -= 1;
                        if *c == 0 {
                            ready.push(other.clone());
                        }
                    }
                }
            }
        }
        if order.len() == self.nodes.len() {
            Some(order)
        } else {
            None
        }
    }
}

/// Extract names referenced by a top-level declaration's initializer / body.
/// This is intentionally shallow for now (identifier references in the init
/// expression tree); refinement follows spec Ch.03 exactly.
fn extract_deps(d: &Decl) -> Vec<String> {
    let mut deps = Vec::new();
    if let Decl::Var {
        init: Some(init), ..
    } = d
    {
        collect_refs(init, &mut deps);
    }
    deps
}

/// Collect identifier references from an expression tree.
fn collect_refs(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Name { name, .. } => out.push(name.clone()),
        Expr::Call { callee, args, .. } => {
            collect_refs(callee, out);
            for a in args {
                collect_refs(&a.value, out);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_refs(lhs, out);
            collect_refs(rhs, out);
        }
        Expr::Unary { inner, .. } => collect_refs(inner, out),
        Expr::Paren { inner, .. } => collect_refs(inner, out),
        Expr::Member { object, .. } => collect_refs(object, out),
        Expr::Subscript { object, index, .. } => {
            collect_refs(object, out);
            collect_refs(index, out);
        }
        Expr::Assign { lhs, rhs, .. } => {
            collect_refs(lhs, out);
            collect_refs(rhs, out);
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                collect_refs(s, out);
            }
        }
        Expr::If {
            cond, then, els, ..
        } => {
            collect_refs(cond, out);
            collect_refs(then, out);
            if let Some(e) = els {
                collect_refs(e, out);
            }
        }
        _ => {}
    }
}

fn decl_name(d: &Decl) -> Option<String> {
    match d {
        Decl::Var { name, .. } => Some(name.clone()),
        _ => None,
    }
}

fn decl_pos(d: &Decl) -> (u32, u32) {
    use Decl::*;
    match d {
        Var { pos, .. } => (pos.line, pos.col),
        _ => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cj_parser::parse_source;

    fn parse(src: &str) -> File {
        let (f, _) = parse_source(src);
        f
    }

    #[test]
    fn simple_dependency() {
        let f = parse("let x = 1\nlet y = x + 1\n");
        let g = DepGraph::build(&[&f]);
        let y = g.nodes.get("y").unwrap();
        assert!(y.deps.contains(&"x".to_string()), "y should depend on x");
        assert!(g.topo_order().is_some());
    }

    #[test]
    fn cycle_detected() {
        let f = parse("let a = b\nlet b = a\n");
        let g = DepGraph::build(&[&f]);
        let diags = g.detect_cycles();
        assert!(!diags.is_empty(), "expected cycle diag: {:?}", diags);
    }

    #[test]
    fn self_reference_cycle() {
        let f = parse("let c = c\n");
        let g = DepGraph::build(&[&f]);
        let diags = g.detect_cycles();
        assert!(!diags.is_empty(), "expected self-ref cycle diag");
    }

    #[test]
    fn acyclic_order_valid() {
        let f = parse("let x = 1\nlet y = x\nlet z = y\n");
        let g = DepGraph::build(&[&f]);
        let order = g.topo_order().expect("no cycle");
        // z must come after y, y after x.
        let ix = order.iter().position(|n| n == "x").unwrap();
        let iy = order.iter().position(|n| n == "y").unwrap();
        let iz = order.iter().position(|n| n == "z").unwrap();
        assert!(ix < iy && iy < iz, "topo order invalid: {order:?}");
    }
}
