"""Where everything lives.

The working tree is the repository root: found from this file (an editable
install keeps it in python/nlab/), else by searching upward from the
current directory for this project's pyproject.toml. NLAB_ROOT overrides.
"""

import os
import pathlib


def _find_root() -> pathlib.Path:
    if os.environ.get("NLAB_ROOT"):
        return pathlib.Path(os.environ["NLAB_ROOT"]).resolve()
    candidates = [pathlib.Path(__file__).resolve().parents[2]]
    cwd = pathlib.Path.cwd().resolve()
    candidates += [cwd, *cwd.parents]
    for c in candidates:
        pp = c / "pyproject.toml"
        if pp.is_file() and 'name = "nlab"' in pp.read_text(encoding="utf-8"):
            return c
    return cwd


ROOT = _find_root()
ASSETS = pathlib.Path(__file__).resolve().parent / "assets"

BUILD = ROOT / "build"
DIST = ROOT / "dist"
MIRROR = BUILD / "nlab-content"  # Markdown+itex sources
PAGES = MIRROR / "pages"
HTML_MIRROR = BUILD / "nlab-content-html"  # rendered pages (docset only)
WEB_ASSETS = BUILD / "assets"  # live-site CSS/JS (docset only)
STATE_DB = BUILD / "state.db"
PAGE_INDEX = BUILD / "page-index.tsv"
FILES = BUILD / "files"  # content-addressed download store
FILES_MAP = BUILD / "files.tsv"
TYP_PAGES = BUILD / "typ-pages"
TYPST_CSS = BUILD / "typ-assets/typst.css"
WEBP_CACHE = BUILD / "nlab-files-web"
CONVERT_REPORT = BUILD / "convert-report.txt"

DOWNLOAD_PRESETS = ROOT / "downloads"
EDITS = ROOT / "edits"
MITEX = ROOT / "vendor/mitex"
