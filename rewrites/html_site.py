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
from typst_site import load_names, skip_page, slugify  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "build/typ-pages"

LINK_RE = re.compile(r'href="(\d+)\.html(#[^"]*)?"')
IMG_RE = re.compile(r'(src=")(files(?:-web)?/)')

# custom search UI over pagefind.js (no pagefind-ui): the exact
# markup/css iterated in search-sample.html. rel is "../" on page
# dirs, "" on the root search page.
SEARCH_CSS = """<style>
#search { margin: 0 0 1.5em; }
#search-input {
  width: 100%; box-sizing: border-box; font: inherit;
  color: var(--fg); background: var(--bg);
  border: 1px solid var(--rule); border-radius: 6px;
  padding: 0.45em 0.7em; outline: none;
}
#search-input:focus { border-color: var(--link); }
:root { --hl-fg: #b35c00; --chip: rgba(0,0,0,0.06); }
@media (prefers-color-scheme: dark) {
  :root { --hl-fg: #e8963f; --chip: rgba(255,255,255,0.07); }
}
.sr { border-bottom: 1px solid var(--rule); padding: 0 0 10px; margin: 0 0 10px; }
.sr-page { font-weight: 700; font-size: 1.1rem; display: block; margin-bottom: 0.15em; }
.sr-match {
  display: flex; flex-wrap: wrap; align-items: baseline;
  justify-content: space-between; column-gap: 1em;
  margin: 0.1em 0 0 1.25em;
}
.sr-sec { white-space: nowrap; }
.sr-sec::before { content: "# "; color: var(--muted); margin-left: -1.25em; }
.sr-snip {
  margin: 0 -0.45em; font-size: 0.85rem;
  color: color-mix(in srgb, var(--fg) 72%, var(--bg));
  background: var(--chip); padding: 0.1em 0.45em; border-radius: 4px;
}
.sr-snip mark { background: none; color: var(--hl-fg); }
</style>"""


def search_head(rel: str, wide: bool = False) -> str:
    extra = "<style>body{max-width:940px}</style>" if wide else ""
    return SEARCH_CSS + extra


def search_body(rel: str, autofocus: bool = False) -> str:
    af = " autofocus" if autofocus else ""
    return (
        f'<div id="search"><input id="search-input" type="search" '
        f'placeholder="Search the nLab"{af}><div id="search-results"></div></div>'
        '<script type="module">'
        f'const pf = await import("{rel or "./"}pagefind/pagefind.js");'
        "await pf.options({excerptLength: 20});"
        "pf.init();"
        "const inp = document.getElementById('search-input');"
        "const out = document.getElementById('search-results');"
        "const esc = t => t.replace(/[&<>\"]/g, c => "
        "({'&':'&amp;','<':'&lt;','>':'&gt;','\"':'&quot;'})[c]);"
        "const secname = t => esc(t.replace(/^[\\d.\u00a0 ]+/, ''));"
        "function render(d) {"
        "  const subs = (d.sub_results || [])"
        "    .filter(s => s.excerpt && s.url.includes('#'))"
        "    .slice(0, 4)"
        "    .map(s => `<div class=\"sr-match\"><a class=\"sr-sec\" "
        "href=\"${s.url}\">${secname(s.title)}</a>"
        "<span class=\"sr-snip\">${s.excerpt}</span></div>`)"
        "    .join('');"
        "  return `<div class=\"sr\"><a class=\"sr-page\" "
        "href=\"${d.url}\">${esc(d.meta.title)}</a>${subs}</div>`;"
        "}"
        "let seq = 0;"
        "async function run(q) {"
        "  const my = ++seq;"
        "  if (!q) { out.innerHTML = ''; return; }"
        "  const search = await pf.debouncedSearch(q, {}, 200);"
        "  if (!search || my !== seq) return;"
        "  const datas = await Promise.all("
        "    search.results.slice(0, 15).map(r => r.data()));"
        "  if (my !== seq) return;"
        "  out.innerHTML = datas.map(render).join('') ||"
        "    '<p class=\"sr-none\">no results</p>';"
        "}"
        "inp.addEventListener('input', () => run(inp.value.trim()));"
        "const q0 = new URLSearchParams(location.search).get('q');"
        "if (q0) { inp.value = q0; run(q0); }"
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
    pages = sorted(
        (p for p in SRC.glob("*.html")
         if not skip_page(names.get(p.stem, ""))),
        key=lambda p: int(p.stem),
    )

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
        + search_head("", wide=True)
        + "</head><body><h1>nLab</h1>"
        + search_body("", autofocus=True)
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
