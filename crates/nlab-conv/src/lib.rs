//! nlab-conv: convert nLab wiki source to typst.
//!
//! The nLab's pages are Instiki Markdown (Maruku dialect) with itex math,
//! whose diagrams come in several dialects: itex arrays drawn as
//! commutative diagrams, tikz-cd, xymatrix and tikzpicture. This crate
//! turns a whole page ([`page_to_typst`]) or a single formula
//! ([`formula_to_typst`]) into typst, rendering diagrams natively with
//! fletcher / cetz and math through our mitex fork (vendor/mitex).
//!
//! Modules: `prose` (pages), `emit` (formulas and array diagrams),
//! `grid` (array-diagram cell grids), `tikzcd`, `tikzpic`, `xy`.
//!
//! Wiki links and images resolve through a [`Context`] (page index,
//! downloaded-files map) installed per call with [`with_context`]. Used by
//! the `nlab-conv` CLI and, with the `python` feature, as the Python
//! module `nlab._conv` (wrapped by `nlab.conv`).

mod context;
mod emit;
mod grid;
mod prose;
mod tikzcd;
mod tikzpic;
mod xy;

#[cfg(feature = "python")]
mod python;

pub use context::{parse_files_map, parse_page_index, with_context, Context};
pub use prose::canonical_file_url;

/// A whole page (Instiki Markdown + itex) as a typst document. Run under
/// [`with_context`] with `local_mitex = true` for in-site links, embedded
/// images and the local mitex package.
pub fn page_to_typst(src: &str, title: Option<&str>) -> String {
    prose::page_to_typst(src, title)
}

/// One itex formula: `(class, status, code)`, where class names the
/// diagram shape recognized and status is `ok` or why there is no code.
pub fn formula_to_typst(tex: &str) -> (String, String, Option<String>) {
    emit::emit_formula(tex)
}

/// The cell grid of an array diagram, as JSON; `Err` carries the status.
pub fn formula_grid_json(tex: &str) -> Result<String, String> {
    grid::parse_formula(tex)
}

/// A tikz-cd diagram as fletcher code, plus the options it ignored.
pub fn tikzcd_to_fletcher(src: &str) -> Result<(String, Vec<String>), String> {
    tikzcd::tikzcd_to_fletcher(src)
}

/// The mitex concrete syntax tree of a formula, one node per line.
pub fn dump_tree(tex: &str) -> String {
    fn walk(node: &mitex_parser::syntax::SyntaxNode, depth: usize, out: &mut String) {
        let indent = "  ".repeat(depth);
        out.push_str(&format!("{indent}{:?}\n", node.kind()));
        for child in node.children_with_tokens() {
            match child {
                rowan::NodeOrToken::Node(n) => walk(&n, depth + 1, out),
                rowan::NodeOrToken::Token(t) => {
                    out.push_str(&format!("{indent}  {:?} {:?}\n", t.kind(), t.text()))
                }
            }
        }
    }
    let node = mitex_parser::parse(tex, mitex_spec_gen::DEFAULT_SPEC.clone());
    let mut out = String::new();
    walk(&node, 0, &mut out);
    out
}
