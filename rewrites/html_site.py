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
  width: 100%; box-sizing: border-box;
  font: 1rem system-ui, sans-serif;
  color: var(--fg); background: var(--bg);
  border: 1px solid var(--rule); border-radius: 6px;
  padding: 0.45em 0.7em; outline: none;
}
#search-input:focus { border-color: var(--link); }
#search-box { position: relative; margin-bottom: 1.2em; }
.search-hdr {
  display: flex; align-items: center; gap: 1.2em; margin: 0.6em 0 1.2em;
}
.search-hdr h1 {
  margin: 0; padding: 0; border: none; font-size: 1.5rem; white-space: nowrap;
}
.search-hdr #search-box { flex: 1; margin-bottom: 0; }
#search-count {
  position: absolute; right: 3em; top: 50%; transform: translateY(-50%);
  color: var(--muted); font-size: 0.8rem; pointer-events: none;
  font-family: system-ui, sans-serif;
}
:root { --hl-fg: #b35c00; --chip: rgba(0,0,0,0.06); }
@media (prefers-color-scheme: dark) {
  :root { --hl-fg: #e8963f; --chip: rgba(255,255,255,0.07); }
}
.sr { border-bottom: 1px solid var(--rule); padding: 0 0 10px; margin: 0 0 10px; }
.sr-page { font-weight: 700; font-size: 1.25rem; display: block; margin-bottom: 0.4em; }
.sr table {
  border-collapse: separate; border-spacing: 0 0.3em;
  table-layout: fixed; width: 100%;
  margin: -0.15em 0 0 0;
}
.sr col.c-t { width: 32ch; }
.sr td {
  padding: 0.12em 0.5em 0.12em 0; vertical-align: baseline;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.sr td.t a { color: var(--fg); font-weight: 400; font-size: 0.95rem; }
.sr .sr-lvl {
  color: var(--muted); font-size: 0.7rem; font-family: ui-monospace, monospace;
  margin-right: 0.45em; text-transform: uppercase;
}
.sr td.s {
  font-size: 0.85rem;
  color: color-mix(in srgb, var(--fg) 72%, var(--bg));
  background: var(--chip);
  padding: 0.12em 0.45em; border-radius: 4px;
}
.sr mark { background: none; color: var(--hl-fg); }
.sr td.s a.sniplink {
  display: block; color: inherit; text-decoration: none; cursor: pointer;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.sr td.s a.sniplink:hover { text-decoration: none; }
.sr .sr-more td { text-align: center; }
.sr .sr-more a {
  color: var(--muted); font-size: 0.8rem;
  font-family: system-ui, sans-serif;
}
#load-more {
  display: block; margin: 0.5em auto 1.5em; padding: 0.35em 1em;
  font: 0.8rem system-ui, sans-serif; color: var(--muted);
  background: none; border: none; cursor: pointer;
}
#load-more:hover { color: var(--fg); }
</style>"""


def search_head(rel: str, wide: bool = False) -> str:
    extra = "<style>body{max-width:940px}</style>" if wide else ""
    return SEARCH_CSS + extra


def search_body(rel: str, autofocus: bool = False, header: bool = False) -> str:
    af = " autofocus" if autofocus else ""
    js = """
const pf = await import("IMPORTPATH");
await pf.options({excerptLength: 45});
pf.init();
const inp = document.getElementById('search-input');
const out = document.getElementById('search-results');
const cnt = document.getElementById('search-count');
const esc = t => t.replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'})[c]);
const secname = t => t.replace(/^[\\d.\\u00a0 ]+/, '');
let terms = [];
function hi(t) {
  let h = esc(t);
  if (terms.length) {
    const re = new RegExp('(' + terms.join('|') + ')', 'gi');
    h = h.replace(re, '<mark>$1</mark>');
  }
  return h;
}
function win(e) {
  const i = e.indexOf('<mark>');
  if (i > 60) {
    const cut = e.lastIndexOf(' ', i - 60);
    if (cut > 0) e = '\\u2026' + e.slice(cut + 1);
  }
  return e;
}
const unesc = t => t.replace(/&(amp|lt|gt|quot|#39);/g,
  (_, e) => ({amp:'&', lt:'<', gt:'>', quot:'"', '#39':"'"})[e]);
function fragurl(url, ex) {
  const mm = /<mark>(.*?)<\\/mark>/.exec(ex);
  if (!mm) return url;
  const target = unesc(mm[1]).trim();
  // walk outward from the match, stopping just inside the first
  // punctuation: the excerpt is stitched from separate blocks, so a
  // context that crosses a boundary would never match on the page
  const stops = /[.,;:!?()\\[\\]"\\u{201c}\\u{201d}\\u{2018}\\u{2019}\\u{2026}\\u{2014}|]/u;
  const before = unesc(ex.slice(0, mm.index).replace(/<[^>]+>/g, ''));
  const after = unesc(ex.slice(mm.index + mm[0].length).replace(/<[^>]+>/g, ''));
  let pre = '';
  for (let i = before.length - 1; i >= 0 && before.length - i < 40; i--) {
    if (stops.test(before[i])) break;
    pre = before[i] + pre;
  }
  pre = pre.trim();
  let suf = '';
  for (let i = 0; i < after.length && i < 40; i++) {
    if (stops.test(after[i])) break;
    suf += after[i];
  }
  suf = suf.trim();
  let frag = encodeURIComponent(target);
  if (pre) frag = encodeURIComponent(pre) + '-,' + frag;
  if (suf) frag = frag + ',-' + encodeURIComponent(suf);
  return url + (url.includes('#') ? ':~:text=' : '#:~:text=') + frag;
}
function render(d) {
  const subs = (d.sub_results || []).filter(s => s.excerpt && s.url.includes('#'));
  const items = subs.slice(0, 3).map(s => ({
    lvl: (s.anchor?.element) || '', title: secname(s.title),
    url: s.url, ex: s.excerpt,
  }));
  if (!items.length && d.excerpt) {
    items.push({ lvl: 'body', title: '', url: d.url, ex: d.excerpt });
  }
  let rows = items.map(r => {
    const ex = win(r.ex);
    return `<tr><td class="t"><span class="sr-lvl">${r.lvl}</span>` +
      (r.title ? `<a href="${r.url}">${hi(r.title)}</a>` : '') +
      `</td><td class="s"><a class="sniplink" href="${fragurl(r.url, ex)}">${ex}</a></td></tr>`;
  }).join('');
  if (subs.length > 3) {
    const n = subs.length - 3;
    rows += `<tr class="sr-more"><td colspan="2">` +
      `<a href="${d.url}">show ${n} more ${n === 1 ? 'match' : 'matches'}\u2026</a></td></tr>`;
  }
  const tbl = rows ? `<table><colgroup><col class="c-t"><col></colgroup>${rows}</table>` : '';
  return `<div class="sr"><a class="sr-page" href="${d.url}">${hi(d.meta.title)}</a>${tbl}</div>`;
}
let seq = 0, results = [], shown = 0;
async function draw(my) {
  const datas = await Promise.all(results.slice(0, shown).map(r => r.data()));
  if (my !== seq) return;
  let html = datas.map(render).join('') || '<p>no results</p>';
  if (shown < results.length) {
    const hi2 = Math.min(shown + 15, results.length);
    html += `<button id="load-more">show matches ${shown + 1}\\u2013${hi2} of ${results.length}</button>`;
  }
  out.innerHTML = html;
  cnt.textContent = `${Math.min(shown, results.length)} of ${results.length} result${results.length === 1 ? '' : 's'}`;
  document.getElementById('load-more')?.addEventListener('click', () => {
    shown += 15; draw(seq);
  });
}
async function run(q) {
  const my = ++seq;
  if (!q) { out.innerHTML = ''; cnt.textContent = ''; return; }
  const search = await pf.debouncedSearch(q, {}, 200);
  if (!search || my !== seq) return;
  terms = q.split(/\\s+/).filter(w => w.length > 1)
    .map(w => esc(w).replace(/[.*+?^${}()|[\\]\\\\]/g, '\\\\$&'));
  results = search.results;
  shown = 15;
  draw(my);
}
inp.addEventListener('input', () => run(inp.value.trim()));
const q0 = new URLSearchParams(location.search).get('q');
if (q0) { inp.value = q0; run(q0); }
"""
    js = js.replace("IMPORTPATH", (rel or "./") + "pagefind/pagefind.js")
    box = (
        f'<div id="search-box"><input id="search-input" '
        f'type="search" placeholder="Search the nLab" '
        f'spellcheck="false" autocorrect="off" autocapitalize="off"{af}>'
        f'<span id="search-count"></span></div>'
    )
    if header:
        box = f'<div class="search-hdr"><h1>nLab</h1>{box}</div>'
    return (
        f'<div id="search">{box}'
        f'<div id="search-results"></div></div>'
        f'<script type="module">{js}</script>'
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
        # navigation pages (tables of contents) are link lists that
        # would out-rank real articles for common terms
        if names.get(pid, "").endswith("contents"):
            text = text.replace(
                "<body>", '<body data-pagefind-weight="0.1">', 1)
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
        + "</head><body>"
        + search_body("", autofocus=True, header=True)
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
