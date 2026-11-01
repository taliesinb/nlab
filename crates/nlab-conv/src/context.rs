//! Per-call conversion context.
//!
//! The page index (wiki links), the downloaded-files map (images) and the
//! local-mitex flag used to be process-wide globals read once from
//! environment variables. A long-lived caller (the Python module) converts
//! many pages, possibly with different contexts and from several threads,
//! so every entry point runs under a context installed for the current
//! thread by [`with_context`]. Without one, lookups find nothing and
//! local-mitex mode is off.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Default, Debug)]
pub struct Context {
    /// Lowercased page name or redirect alias -> page id (first entry wins,
    /// as in build/page-index.tsv).
    pub page_index: HashMap<String, String>,
    /// Canonical URL -> (store path `files/<hash>.<ext>`, kind), as in
    /// build/files.tsv.
    pub files: HashMap<String, (String, String)>,
}

impl Context {
    /// Load the two TSVs; a `None` path leaves that table empty.
    pub fn load(page_index: Option<&Path>, files_map: Option<&Path>) -> std::io::Result<Self> {
        let mut ctx = Context::default();
        if let Some(p) = page_index {
            ctx.page_index = parse_page_index(&std::fs::read_to_string(p)?);
        }
        if let Some(p) = files_map {
            ctx.files = parse_files_map(&std::fs::read_to_string(p)?);
        }
        Ok(ctx)
    }

    /// The CLI's context: NLAB_PAGEIDX / NLAB_FILES_MAP name the TSVs; an
    /// unset variable or unreadable file leaves that table empty.
    pub fn from_env() -> Self {
        let read = |var: &str| {
            std::env::var(var).ok().and_then(|p| std::fs::read_to_string(p).ok())
        };
        Context {
            page_index: read("NLAB_PAGEIDX").map(|t| parse_page_index(&t)).unwrap_or_default(),
            files: read("NLAB_FILES_MAP").map(|t| parse_files_map(&t)).unwrap_or_default(),
        }
    }
}

/// `name<TAB>id` lines; case-insensitive keys, first entry wins.
pub fn parse_page_index(txt: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for line in txt.lines() {
        if let Some((name, id)) = line.split_once('\t') {
            m.entry(name.trim().to_lowercase()).or_insert_with(|| id.trim().to_string());
        }
    }
    m
}

/// `url<TAB>store path<TAB>kind` lines.
pub fn parse_files_map(txt: &str) -> HashMap<String, (String, String)> {
    let mut m = HashMap::new();
    for line in txt.lines() {
        let mut cols = line.split('\t');
        if let (Some(url), Some(stored), Some(kind)) = (cols.next(), cols.next(), cols.next()) {
            m.insert(url.to_string(), (stored.to_string(), kind.to_string()));
        }
    }
    m
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<Context>>> = const { RefCell::new(None) };
    static LOCAL_MITEX: Cell<bool> = const { Cell::new(false) };
}

/// Run `f` with `ctx` and the local-mitex flag installed for this thread,
/// restoring the previous state afterwards (also on panic).
pub fn with_context<R>(ctx: &Arc<Context>, local_mitex: bool, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Arc<Context>>, bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|c| *c.borrow_mut() = self.0.take());
            LOCAL_MITEX.with(|m| m.set(self.1));
        }
    }
    let _restore = Restore(
        CURRENT.with(|c| c.replace(Some(ctx.clone()))),
        LOCAL_MITEX.with(|m| m.replace(local_mitex)),
    );
    f()
}

/// Page id for a (case-insensitive) page name or alias.
pub(crate) fn lookup_page(name: &str) -> Option<String> {
    CURRENT.with(|c| {
        c.borrow().as_ref().and_then(|ctx| ctx.page_index.get(&name.trim().to_lowercase()).cloned())
    })
}

/// (store path, kind) of a downloaded file, by canonical URL.
pub(crate) fn lookup_file(canonical_url: &str) -> Option<(String, String)> {
    CURRENT.with(|c| c.borrow().as_ref().and_then(|ctx| ctx.files.get(canonical_url).cloned()))
}

/// Emit against the locally built mitex fork (`@local/mitex:0.2.7`).
pub(crate) fn local_mitex() -> bool {
    LOCAL_MITEX.with(|m| m.get())
}
