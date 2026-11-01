"""The end-to-end builds, and small inspection commands.

  nlab build-site    clean checkout -> dist/html (every step cached)
  nlab build-docset  dist/nLab.docset for Dash
  nlab convert-page  one page -> typst on stdout (debugging)
  nlab status        what build/state.db knows
"""

import argparse
import os
import sqlite3
import sys

from .paths import (DIST, DOWNLOAD_PRESETS, FILES_MAP, PAGE_INDEX, PAGES, ROOT,
                    STATE_DB, TYP_PAGES)


def step(title: str) -> None:
    print(f"\n==> {title}", flush=True)


def build_site(argv=None):
    ap = argparse.ArgumentParser(
        prog="nlab build-site",
        description="Build the static HTML nLab (dist/html) from a clean checkout.")
    ap.add_argument("--refresh", action="store_true",
                    help="re-download the nlab-content mirror (new wiki content)")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 8,
                    help="parallel page conversions (default: CPU count)")
    ap.add_argument("--no-search", action="store_true",
                    help="skip the Pagefind index (search.html won't work)")
    ap.add_argument("--downloads", default=str(DOWNLOAD_PRESETS / "ncatlab.json"),
                    help="download rules (default downloads/ncatlab.json: nLab images "
                         "+ PDFs; common.json adds arXiv etc. and takes days; "
                         "everything.json adds any .pdf link; the broader two are "
                         "for private builds only)")
    ap.add_argument("--skip-setup", action="store_true",
                    help="skip the tool check and mitex package install")
    args = ap.parse_args(argv)

    from . import convert, downloads, mirror, page_index, site, toolchain
    if not args.skip_setup:
        step("tools and the mitex package (vendor/mitex)")
        toolchain.setup()
    step("nlab-content mirror")
    mirror.fetch_mirror("nlab-content", PAGES.parent, args.refresh)
    step("page index")
    page_index.main([])
    step(f"downloads ({os.path.basename(args.downloads)}; resumable, Ctrl-C is safe)")
    downloads.main(["--config", args.downloads])
    step(f"converting pages (--jobs {args.jobs}; a cold full run is ~1 h on an M4, "
         "mostly svgo; reruns convert only what changed)")
    convert.main(["--html", "--jobs", str(args.jobs)])
    step("static site")
    site.main(["--no-search"] if args.no_search else [])
    print(f"\nSite written to {DIST / 'html'}\nPreview: python3 -m http.server -d "
          f"{(DIST / 'html').relative_to(ROOT)} 8000  ->  http://localhost:8000/search.html")


def build_docset(argv=None):
    ap = argparse.ArgumentParser(
        prog="nlab build-docset",
        description="Build dist/nLab.docset for Dash from the mirrors; pages already "
                    "rendered by `nlab convert --html` (build/typ-pages) are used "
                    "in place of the mirror's HTML.")
    ap.add_argument("--refresh", action="store_true",
                    help="re-download the mirrors and the live site's CSS/JS")
    ap.add_argument("--only", help="comma-separated page names: a small test docset")
    ap.add_argument("--out", default=str(DIST))
    ap.add_argument("--no-typst-pages", action="store_true",
                    help="use the mirror's HTML for every page")
    args = ap.parse_args(argv)

    from . import docset, mirror
    from .paths import HTML_MIRROR, MIRROR, WEB_ASSETS
    step("mirrors and the live site's CSS/JS")
    mirror.fetch_mirror("nlab-content-html", HTML_MIRROR, args.refresh)
    mirror.fetch_mirror("nlab-content", MIRROR, args.refresh)
    mirror.fetch_web_assets(args.refresh)
    step("docset")
    dargs = ["--html", str(HTML_MIRROR), "--source", str(MIRROR),
             "--assets", str(WEB_ASSETS), "--out", args.out]
    diagrams = ROOT / "rewrites/out/svg"  # regenerated diagrams, if built
    if diagrams.is_dir():
        dargs += ["--diagrams", str(diagrams)]
    if TYP_PAGES.is_dir() and not args.no_typst_pages:
        dargs += ["--typst-pages", str(TYP_PAGES)]
    if args.only:
        dargs += ["--only", args.only]
    docset.main(dargs)


def convert_page(argv=None):
    ap = argparse.ArgumentParser(
        prog="nlab convert-page",
        description="Convert one page to typst on stdout, with the current page "
                    "index and downloads map (what `nlab convert` would produce).")
    ap.add_argument("page", help="page id or name in the mirror, or - for stdin")
    ap.add_argument("--title", help="title for stdin input")
    args = ap.parse_args(argv)

    from . import conv
    from .convert import apply_edits
    c = conv.Converter(PAGE_INDEX if PAGE_INDEX.exists() else None,
                       FILES_MAP if FILES_MAP.exists() else None)
    if args.page == "-":
        source, title = sys.stdin.read(), args.title
    else:
        pid = args.page if args.page.isdigit() else c.resolve(args.page)
        md = next(PAGES.glob(f"*/*/*/*/{pid}/content.md"), None) if pid else None
        if md is None:
            raise SystemExit(f"error: no page {args.page!r} in {PAGES}")
        source, problems = apply_edits(pid, md.read_text(encoding="utf-8",
                                                         errors="replace"))
        if problems:
            print(f"[edits] {problems}", file=sys.stderr)
        title = args.title or (md.parent / "name").read_text(encoding="utf-8").strip()
    sys.stdout.write(c.page(source, title))


def status(argv=None):
    argparse.ArgumentParser(prog="nlab status",
                            description="Summarize build/state.db").parse_args(argv)
    if not STATE_DB.exists():
        raise SystemExit(f"no {STATE_DB} yet")
    con = sqlite3.connect(STATE_DB)
    tables = {r[0] for r in con.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    if "downloads" in tables:
        print("downloads (referenced URLs), by rule / status:")
        for rule, st, n, mb in con.execute(
                "SELECT COALESCE(rule, '-'), status, count(*), "
                "sum(CASE WHEN status = 'done' THEN size ELSE 0 END) / 1e6 "
                "FROM downloads d WHERE EXISTS (SELECT 1 FROM download_refs r "
                "WHERE r.url = d.url) GROUP BY 1, 2 ORDER BY 1, 3 DESC"):
            print(f"  {rule:24} {st:12} {n:7}" + (f"  {mb:.1f} MB" if mb else ""))
    if "conversions" in tables:
        print("conversions, by mode / status:")
        for mode, st, n, secs in con.execute(
                "SELECT mode, status, count(*), sum(seconds) FROM conversions "
                "GROUP BY 1, 2 ORDER BY 1, 2"):
            print(f"  {mode:6} {st:6} {n:7}  ({secs or 0:.0f} s of conversion)")
        for page, name, err in con.execute(
                "SELECT page, name, error FROM conversions WHERE status != 'ok' "
                "ORDER BY page LIMIT 20"):
            print(f"    {page} {name}: {err}")
