// cj-ast: Cangjie AST node definitions.
//
// GENERATED from the official cangjie_compiler AST spec — do not hand-edit the
// node types (see generated.rs header). Regenerate with:
//   python3 tools/gen_ast.py > crates/cj-ast/src/generated.rs
//
// Node inventory mirrors the official ASTKind.inc (Decl/Expr/Type/Pattern +
// auxiliary nodes); compiler-internal fields are excluded (frontend/LSP only).

/// A source span (line/col are 1-based; offset is byte offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CodePos {
    pub line: u32,
    pub col: u32,
    pub offset: usize,
    pub end_line: u32,
    pub end_col: u32,
    pub end_offset: usize,
}

impl CodePos {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        line: u32,
        col: u32,
        offset: usize,
        end_line: u32,
        end_col: u32,
        end_offset: usize,
    ) -> Self {
        CodePos {
            line,
            col,
            offset,
            end_line,
            end_col,
            end_offset,
        }
    }
}

pub mod generated;

pub mod program {
    //! Program / package / file containers (thin re-export of top-level types).

    pub use crate::{CodePos, Decl, Expr, File, ImportSpec};
}

// Re-export the generated node types at the crate root (CodePos already lives here).
pub use generated::*;
