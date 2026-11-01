"""nLab wiki source -> typst, in-process (the Rust crate crates/nlab-conv).

The nLab's pages are Instiki Markdown (Maruku dialect) with itex math, whose
diagrams come as itex arrays, tikz-cd, xymatrix or tikzpicture. This module
converts a whole page, or a single formula, into typst: diagrams become
native fletcher / cetz code, math goes through our mitex fork
(@local/mitex:0.2.7, installed by `nlab setup`).

Every call releases the GIL, so a thread pool converts pages in parallel.

    from nlab import conv
    c = conv.Converter(page_index="build/page-index.tsv",
                       files_map="build/files.tsv")
    typ = c.page(source, title="Yoneda lemma")
"""

from __future__ import annotations

import os

from . import _conv

__all__ = ["Converter", "formula", "grid", "canonical_file_url", "dump_tree",
           "module_path"]


class Converter:
    """Converts pages against a page index and a downloaded-files map.

    page_index: TSV of `name<TAB>id` (build/page-index.tsv); wiki links to
        names in it become in-site links, others point at ncatlab.org.
    files_map: TSV of `url<TAB>files/<hash>.<ext><TAB>kind`
        (build/files.tsv); images listed there are embedded.
    """

    def __init__(self, page_index: str | os.PathLike | None = None,
                 files_map: str | os.PathLike | None = None):
        self._c = _conv.Converter(page_index, files_map)

    def page(self, source: str, title: str | None = None) -> str:
        """A whole page as a typst document."""
        return self._c.page(source, title)

    def tikzcd(self, source: str) -> tuple[str, list[str]]:
        """A tikz-cd diagram as (fletcher code, ignored options);
        ValueError if it cannot be converted."""
        return self._c.tikzcd(source)

    def resolve(self, name: str) -> str | None:
        """Page id for a page name or redirect alias (case-insensitive)."""
        return self._c.resolve(name)

    def __repr__(self) -> str:
        return (f"Converter({self._c.page_index_size} names, "
                f"{self._c.files_size} files)")


def formula(tex: str, local_mitex: bool = False) -> tuple[str, str, str | None]:
    """One itex formula: (diagram class, status, typst code or None)."""
    return _conv.formula(tex, local_mitex)


def grid(tex: str) -> str:
    """The cell grid of an array diagram as JSON; ValueError(status) if
    the formula is not one."""
    return _conv.grid(tex)


def canonical_file_url(url: str) -> str:
    """Key of a file URL in build/files.tsv: nLab uploads (absolute,
    relative, http or https) become https://ncatlab.org/nlab/files/<name>;
    other URLs are returned as written."""
    return _conv.canonical_file_url(url)


def dump_tree(tex: str) -> str:
    """The mitex syntax tree of a formula, one node per line."""
    return _conv.dump_tree(tex)


def module_path() -> str:
    """The compiled extension (part of the conversion cache's tool key)."""
    return _conv.__file__
