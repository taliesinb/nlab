//! The Python module `nlab._conv` (wrapped and documented by `nlab.conv`).
//! Every conversion releases the GIL, so threads convert in parallel.

use std::path::PathBuf;
use std::sync::Arc;

use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;

use crate::{with_context, Context};

/// Converts pages with a fixed page index and downloaded-files map.
#[pyclass(frozen, module = "nlab._conv")]
struct Converter {
    ctx: Arc<Context>,
}

#[pymethods]
impl Converter {
    #[new]
    #[pyo3(signature = (page_index=None, files_map=None))]
    fn new(page_index: Option<PathBuf>, files_map: Option<PathBuf>) -> PyResult<Self> {
        let ctx = Context::load(page_index.as_deref(), files_map.as_deref())
            .map_err(|e| PyOSError::new_err(e.to_string()))?;
        Ok(Converter { ctx: Arc::new(ctx) })
    }

    /// A whole page (Instiki Markdown + itex) as a typst document.
    #[pyo3(signature = (source, title=None))]
    fn page(&self, py: Python<'_>, source: &str, title: Option<&str>) -> String {
        let (ctx, source, title) = (self.ctx.clone(), source.to_owned(), title.map(str::to_owned));
        py.detach(move || with_context(&ctx, true, || crate::page_to_typst(&source, title.as_deref())))
    }

    /// A tikz-cd diagram as (fletcher code, ignored options).
    fn tikzcd(&self, py: Python<'_>, source: &str) -> PyResult<(String, Vec<String>)> {
        let (ctx, source) = (self.ctx.clone(), source.to_owned());
        py.detach(move || with_context(&ctx, true, || crate::tikzcd_to_fletcher(&source)))
            .map_err(PyValueError::new_err)
    }

    /// Page id for a page name or redirect alias (case-insensitive).
    fn resolve(&self, name: &str) -> Option<String> {
        self.ctx.page_index.get(&name.trim().to_lowercase()).cloned()
    }

    #[getter]
    fn page_index_size(&self) -> usize {
        self.ctx.page_index.len()
    }

    #[getter]
    fn files_size(&self) -> usize {
        self.ctx.files.len()
    }
}

/// One itex formula: (class, status, code or None).
#[pyfunction]
#[pyo3(signature = (tex, local_mitex=false))]
fn formula(py: Python<'_>, tex: &str, local_mitex: bool) -> (String, String, Option<String>) {
    let tex = tex.to_owned();
    let ctx = Arc::new(Context::default());
    py.detach(move || with_context(&ctx, local_mitex, || crate::formula_to_typst(&tex)))
}

/// The cell grid of an array diagram as JSON; ValueError carries the status.
#[pyfunction]
fn grid(py: Python<'_>, tex: &str) -> PyResult<String> {
    let tex = tex.to_owned();
    py.detach(move || crate::formula_grid_json(&tex)).map_err(PyValueError::new_err)
}

/// Key of a file URL in the downloads map (build/files.tsv).
#[pyfunction]
fn canonical_file_url(url: &str) -> String {
    crate::canonical_file_url(url)
}

/// The mitex syntax tree of a formula, one node per line.
#[pyfunction]
fn dump_tree(tex: &str) -> String {
    crate::dump_tree(tex)
}

#[pymodule]
#[pyo3(name = "_conv")]
fn conv_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Converter>()?;
    m.add_function(wrap_pyfunction!(formula, m)?)?;
    m.add_function(wrap_pyfunction!(grid, m)?)?;
    m.add_function(wrap_pyfunction!(canonical_file_url, m)?)?;
    m.add_function(wrap_pyfunction!(dump_tree, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
