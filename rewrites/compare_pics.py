#!/usr/bin/env python3
"""Hand-verification galleries for xymatrix / standalone-tikzpicture
conversion, in the style of compare_tikzcd.py.

The site renders tikzcd, xymatrix and tikzpicture blocks server-side
into SVGs, in document order. We collect ALL such blocks per page (so
the nth block pairs with the nth non-logo SVG when the counts agree),
then build one gallery per requested kind, rendering our version by
pushing the raw block through the full `nlab-typ page` pipeline.

Usage: compare_pics.py --kind xymatrix|tikzpicture [--rows i,j] [--all]
"""

import argparse
import hashlib
import pathlib
import re
import subprocess
import concurrent.futures

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
BUILD = ROOT / "build"
BINARY = ROOT / "crates/nlab-typ/target/release/nlab-typ"
CACHE = HERE / "out/piccache"

BLOCK_RE = re.compile(
    r"(?s)\\begin\{tikzcd\}.*?\\end\{tikzcd\}"
    r"|\\begin\{xymatrix[^}]*\}.*?\\end\{xymatrix[^}]*\}"
    r"|\\begin\{tikzpicture\}.*?\\end\{tikzpicture\}",
)
SVG_RE = re.compile(r"<svg[^>]*>.*?</svg>", re.S)


def kind_of(block: str) -> str:
    if block.startswith("\\begin{tikzcd}"):
        return "tikzcd"
    if block.startswith("\\begin{xymatrix"):
        return "xymatrix"
    return "tikzpicture"


def corpus():
    """All diagram blocks, page order; tikzpictures inside tikzcd are
    part of their tikzcd block (BLOCK_RE's tikzcd alternative wins)."""
    blocks = []
    pages = sorted((BUILD / "nlab-content/pages").rglob("content.md"))
    for p in pages:
        src = p.read_text(errors="replace")
        for j, m in enumerate(BLOCK_RE.finditer(src)):
            blocks.append((p, j, kind_of(m.group(0)), m.group(0)))
    return blocks


def original_svgs(md_page: pathlib.Path):
    rel = md_page.relative_to(BUILD / "nlab-content")
    html = BUILD / "nlab-content-html" / rel.with_name("content.html")
    if not html.exists():
        return []
    src = html.read_text(errors="replace")
    svgs = []
    for m in SVG_RE.finditer(src):
        before = src[max(0, m.start() - 300):m.start()]
        if "pageName" in before or "float: left" in before:
            continue  # site logo
        if "<math" in before[-40:]:
            continue  # embedded in mathml
        svgs.append(m.group(0))
    return svgs


def render(block: str) -> pathlib.Path | None:
    CACHE.mkdir(parents=True, exist_ok=True)
    h = hashlib.sha1(block.encode()).hexdigest()[:16]
    svg = CACHE / f"{h}.svg"
    if svg.exists():
        return svg
    r = subprocess.run(
        [str(BINARY), "page"],
        input=block + "\n",
        capture_output=True,
        text=True,
        timeout=300,
    )
    if r.returncode != 0:
        (CACHE / f"{h}.err").write_text(r.stderr)
        return None
    typ = CACHE / f"{h}.typ"
    # auto-size the probe page instead of the 17cm article layout
    typ.write_text(r.stdout.replace(
        '#set page(width: 17cm, height: 25cm, margin: 1.6cm, fill: white, numbering: "1")',
        "#set page(width: auto, height: auto, margin: 10pt, fill: white)",
    ))
    c = subprocess.run(
        ["typst", "compile", "--format", "svg", str(typ), str(svg)],
        capture_output=True,
        text=True,
        timeout=120,
    )
    if c.returncode != 0:
        (CACHE / f"{h}.err").write_text(c.stderr)
        return None
    return svg


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kind", required=True,
                    choices=("xymatrix", "tikzpicture"))
    ap.add_argument("--rows", help="comma-separated indices in this kind")
    ap.add_argument("--all", action="store_true")
    args = ap.parse_args()
    out = HERE / "out" / f"compare-{args.kind}.html"

    blocks = corpus()
    page_counts: dict[pathlib.Path, int] = {}
    for p, _, _, _ in blocks:
        page_counts[p] = page_counts.get(p, 0) + 1
    mine = [(i, p, j, blk) for i, (p, j, k, blk) in enumerate(blocks)
            if k == args.kind]
    pick = list(range(len(mine)))
    if args.rows:
        pick = [int(x) for x in args.rows.split(",")]

    with concurrent.futures.ThreadPoolExecutor(8) as ex:
        ours = list(ex.map(lambda n: render(mine[n][3]), pick))

    page_svgs: dict[pathlib.Path, list[str]] = {}
    rows, n_ok = [], 0
    for n, svg in zip(pick, ours):
        _, page, ordinal, blk = mine[n]
        name = ((page.parent / "name").read_text().strip()
                if (page.parent / "name").exists() else str(page))
        if page not in page_svgs:
            page_svgs[page] = original_svgs(page)
        origs = page_svgs[page]
        orig = (origs[ordinal] if len(origs) == page_counts[page]
                else None)
        orig_html = orig if orig else "<em>no matched original</em>"
        if svg is not None:
            n_ok += 1
            m = f'<img src="piccache/{svg.name}" loading="lazy">'
        else:
            m = "<em>FAILED</em>"
        rows.append(
            f'<tr id="row{n}"><td class="n">{n}</td>'
            f'<td class="page">{name}</td>'
            f"<td>{orig_html}</td><td>{m}</td></tr>"
        )

    out.write_text(
        "<!doctype html><meta charset=utf-8>"
        f"<title>{args.kind} compare</title>"
        "<style>body{font:14px sans-serif}table{border-collapse:collapse}"
        "td{border:1px solid #ccc;padding:6px;vertical-align:middle}"
        "td.n{font-weight:bold;color:#c00}td.page{max-width:9em;font-size:11px}"
        "svg,img{max-width:34em;height:auto}</style>"
        f"<h2>{args.kind} &rarr; typst: {n_ok}/{len(pick)} rendered"
        f" ({len(mine)} blocks in corpus)</h2>"
        "<table><tr><th>#</th><th>page</th><th>nLab server render</th>"
        "<th>ours</th></tr>" + "\n".join(rows) + "</table>"
    )
    print(f"{out}: {len(pick)} rows, {n_ok} rendered ok")


if __name__ == "__main__":
    main()
