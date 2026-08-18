#!/usr/bin/env python3
"""Emit the rendered HTML pages as a deployable static site.

dist/html/<slug>/index.html per page (directory URLs), with cross-links
rewritten from <id>.html to ../<slug>/, shared assets/ + files/ +
files-web/ at the root (hardlinked from the build tree), and a root
index.html listing every page.

Usage: html_site.py [--out DIR]
Deploy: netlify deploy --prod --dir dist/html
"""

import argparse
import os
import pathlib
import re
import shutil
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from typst_site import load_names, slugify  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "build/typ-pages"

LINK_RE = re.compile(r'href="(\d+)\.html(#[^"]*)?"')
IMG_RE = re.compile(r'(src=")(files(?:-web)?/)')

# pagefind search UI, injected into every page (rel is "../" on page
# dirs, "" on the root index)
SEARCH_VARS = (
    "<style>#search{margin:0 0 1.5em}"
    ":root{--pagefind-ui-scale:.85;--pagefind-ui-primary:var(--link);"
    "--pagefind-ui-text:var(--fg);--pagefind-ui-background:var(--bg);"
    "--pagefind-ui-border:var(--rule);--pagefind-ui-tag:var(--rule);"
    "--hl-bg:#f6c47a}"
    "@media (prefers-color-scheme: dark){:root{--hl-bg:#8a4a00}}"
    # result titles follow the link color
    ".pagefind-ui__result-link{color:var(--link)}"
    # empty page-level excerpts (processResult) still render a <p>
    ".pagefind-ui__result-excerpt:empty{display:none}"
    # match highlight: normal text on dark orange, not yellow-on-black
    ".pagefind-ui__result-excerpt mark{background:var(--hl-bg);"
    "color:var(--fg);border-radius:2px}"
    "</style>"
)


def search_head(rel: str) -> str:
    return (
        f'<link rel="stylesheet" href="{rel}pagefind/pagefind-ui.css">'
        f'<script src="{rel}pagefind/pagefind-ui.js"></script>'
        + SEARCH_VARS
    )


def search_body(rel: str) -> str:
    return (
        '<div id="search"></div><script>'
        "window.addEventListener('DOMContentLoaded',()=>new PagefindUI("
        f"{{element:'#search',baseUrl:'{rel}',showSubResults:true,showImages:false,"
        # the page-level excerpt duplicates the per-section sub-results
        "processResult:(r)=>{r.excerpt='';return r}}));"
        "</script>"
    )


def hardlink_tree(src: pathlib.Path, dst: pathlib.Path):
    for f in src.rglob("*"):
        if not f.is_file():
            continue
        t = dst / f.relative_to(src)
        t.parent.mkdir(parents=True, exist_ok=True)
        if not t.exists():
            try:
                os.link(f, t)
            except OSError:
                shutil.copy(f, t)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(ROOT / "dist/html"))
    ap.add_argument("--no-search", action="store_true",
                    help="skip the pagefind index build")
    args = ap.parse_args()
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    names = load_names()
    pages = sorted(SRC.glob("*.html"), key=lambda p: int(p.stem))

    # slug assignment identical to the typst site (exact names first)
    taken: dict[str, str] = {}
    slug_of: dict[str, str] = {}

    def exactness(pid: str) -> int:
        name = names.get(pid, pid)
        return 0 if slugify(name) == name.lower().replace(" ", "-") else 1

    for p in sorted(pages, key=lambda p: (exactness(p.stem), int(p.stem))):
        pid = p.stem
        slug = slugify(names.get(pid, pid))
        if slug in taken:
            slug = f"{slug}-{pid}"
        taken[slug] = pid
        slug_of[pid] = slug

    def relink(m: re.Match) -> str:
        slug = slug_of.get(m.group(1))
        frag = m.group(2) or ""
        if slug is None:
            return m.group(0)
        return f'href="../{slug}/{frag}"'

    for p in pages:
        pid = p.stem
        d = out / slug_of[pid]
        d.mkdir(exist_ok=True)
        text = p.read_text(errors="replace")
        text = LINK_RE.sub(relink, text)
        text = IMG_RE.sub(r"\1../\2", text)
        text = text.replace("</head>", search_head("../") + "</head>", 1)
        text = text.replace("<body>", "<body>" + search_body("../"), 1)
        (d / "index.html").write_text(text)

    # prune slugs from earlier runs
    keep = set(taken) | {"assets", "files", "files-web", "pagefind"}
    for old in out.iterdir():
        if old.is_dir() and old.name not in keep:
            shutil.rmtree(old)

    # shared assets and image stores (hardlinks: same content, no copy)
    (out / "assets").mkdir(exist_ok=True)
    shutil.copy(
        pathlib.Path(__file__).parent / "docset_shell.css",
        out / "assets" / "nlab-typst.css",
    )
    typst_css = ROOT / "build/typ-assets/typst.css"
    if typst_css.exists():
        shutil.copy(typst_css, out / "assets" / "typst.css")
    hardlink_tree(ROOT / "build/nlab-files", out / "files")
    hardlink_tree(ROOT / "build/nlab-files-web", out / "files-web")

    (out / "search.html").write_text(
        '<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width, initial-scale=1">'
        "<title>nLab</title>"
        '<link rel="stylesheet" href="assets/typst.css">'
        '<link rel="stylesheet" href="assets/nlab-typst.css">'
        + search_head("")
        + "</head><body><h1>nLab</h1>"
        + search_body("")
        + "</body></html>"
    )
    (out / "index.html").unlink(missing_ok=True)
    (out / "_redirects").write_text("/  /search.html  302\n")
    print(f"{len(pages)} pages -> {out}")

    if not args.no_search:
        import subprocess
        subprocess.run(
            ["npx", "-y", "pagefind", "--site", str(out)],
            check=True,
            cwd=str(ROOT),
        )


if __name__ == "__main__":
    main()
