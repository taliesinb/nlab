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
import html as html_mod
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
        (d / "index.html").write_text(text)

    # prune slugs from earlier runs
    keep = set(taken) | {"assets", "files", "files-web"}
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

    rows = sorted(
        ((names.get(pid, pid), slug) for slug, pid in taken.items()),
        key=lambda r: r[0].lower(),
    )
    items = "\n".join(
        f'<li><a href="{slug}/">{html_mod.escape(name)}</a></li>'
        for name, slug in rows
    )
    (out / "index.html").write_text(
        '<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width, initial-scale=1">'
        "<title>nLab</title>"
        '<link rel="stylesheet" href="assets/typst.css">'
        '<link rel="stylesheet" href="assets/nlab-typst.css">'
        f"</head><body><h1>nLab</h1><ul>{items}</ul></body></html>"
    )
    print(f"{len(pages)} pages -> {out}")


if __name__ == "__main__":
    main()
