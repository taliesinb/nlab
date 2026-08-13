#!/usr/bin/env python3
"""Bulk-convert the whole nLab content mirror to typst pages.

Every pages/**/content.md becomes build/typ-pages/<id>.typ (+ .pdf).
Content output is never committed, like the mirrors. Resumable: pages
whose .pdf is newer than both the source and the converter binary are
skipped. A failure summary lands in out/bulk-report.txt.

Usage: bulk_pages.py [--limit N] [--no-pdf] [--jobs J]
"""

import argparse
import hashlib
import re
import concurrent.futures
import os
import pathlib
import subprocess
import time

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
BUILD = ROOT / "build/nlab-content/pages"
OUT = ROOT / "build/typ-pages"
BIN = ROOT / "crates/nlab-typ/target/release/nlab-typ"
REPORT = HERE / "out/bulk-report.txt"
FILES_ROOT = ROOT / "build/nlab-files"


CSS = (HERE / "docset_shell.css").read_text()
SVG_CACHE = ROOT / "build/svgmin"
WEBP_CACHE = ROOT / "build/nlab-files-web"
STYLE_RE = re.compile(r"<style>.*?</style>", re.S)
SVG_RE = re.compile(r"<svg.*?</svg>", re.S)
IMG_RE = re.compile(r'(<img[^>]*src=")files/([^"]+)(")')


def minified_svg(svg: str) -> str:
    """Cached svgo pass; ids get a content-hash prefix so the many SVGs
    inlined into one page can never cross-reference."""
    h = hashlib.sha1(svg.encode()).hexdigest()[:16]
    cached = SVG_CACHE / f"{h}.svg"
    if cached.exists():
        return cached.read_text()
    SVG_CACHE.mkdir(parents=True, exist_ok=True)
    tmp_in = SVG_CACHE / f".{h}.{os.getpid()}.in.svg"
    tmp_out = SVG_CACHE / f".{h}.{os.getpid()}.out.svg"
    tmp_in.write_text(svg)
    try:
        r = subprocess.run(
            ["svgo", "--config", str(HERE / "svgo.config.mjs"),
             "-i", str(tmp_in), "-o", str(tmp_out)],
            capture_output=True,
            timeout=60,
            env=dict(os.environ, SVGO_PREFIX=h[:8]),
        )
        if r.returncode == 0 and tmp_out.exists():
            tmp_out.replace(cached)  # atomic publish
            return cached.read_text()
    except (subprocess.TimeoutExpired, OSError):
        pass
    finally:
        for t in (tmp_in, tmp_out):
            t.unlink(missing_ok=True)
    return svg  # minify failed: keep the original


def webp_name(name: str) -> str | None:
    """Cached cwebp re-encode of a png/jpeg from the file store; returns
    the name under files-web/, or None to keep the original."""
    stem, dot, ext = name.rpartition(".")
    if ext.lower() not in ("png", "jpg", "jpeg"):
        return None
    src = FILES_ROOT / name
    if not src.exists():
        return None
    out = WEBP_CACHE / f"{stem}.webp"
    if out.exists():
        return out.name
    WEBP_CACHE.mkdir(parents=True, exist_ok=True)
    # pages link ../assets/*.css; mirror the shared sheets there so the
    # generated pages preview correctly straight from build/typ-pages
    assets = ROOT / "build/assets"
    if assets.is_dir():
        import shutil
        shutil.copy(HERE / "docset_shell.css", assets / "nlab-typst.css")
        tc = ROOT / "build/typ-assets/typst.css"
        if tc.exists():
            shutil.copy(tc, assets / "typst.css")
    tmp = WEBP_CACHE / f".{stem}.{os.getpid()}.webp"
    try:
        r = subprocess.run(
            ["cwebp", "-quiet", "-q", "85", str(src), "-o", str(tmp)],
            capture_output=True,
            timeout=120,
        )
        if r.returncode == 0 and tmp.exists() and                 tmp.stat().st_size < src.stat().st_size:
            tmp.replace(out)
            return out.name
    except (subprocess.TimeoutExpired, OSError):
        pass
    finally:
        tmp.unlink(missing_ok=True)
    return None


def shellify(html: str, name: str) -> str:
    """Docset post-pass on typst's HTML export: shared stylesheets
    instead of inline <style>, a <title>, cached-minified inline SVGs,
    and webp re-encoded images."""
    probe = '<meta name="viewport" content="width=device-width, initial-scale=1">'
    title = name.replace("&", "&amp;").replace("<", "&lt;")
    # typst's inline stylesheet is identical on every page: extract once
    m = STYLE_RE.search(html)
    if m:
        typst_css = ROOT / "build/typ-assets/typst.css"
        if not typst_css.exists():
            typst_css.parent.mkdir(parents=True, exist_ok=True)
            typst_css.write_text(m.group(0)[len("<style>"):-len("</style>")])
        html = STYLE_RE.sub("", html, count=1)
    links = ('<link rel="stylesheet" href="../assets/typst.css">'
             '<link rel="stylesheet" href="../assets/nlab-typst.css">')
    html = html.replace(probe, f"{probe}<title>{title}</title>{links}", 1)
    html = SVG_RE.sub(lambda m: minified_svg(m.group(0)), html)

    def img(m: re.Match) -> str:
        w = webp_name(m.group(2))
        if w:
            return f"{m.group(1)}files-web/{w}{m.group(3)}"
        return m.group(0)

    return IMG_RE.sub(img, html)


def one(md: pathlib.Path, pdf: bool, html: bool = False):
    pid = md.parent.name
    name = (md.parent / "name").read_text().strip() if (md.parent / "name").exists() else pid
    typ = OUT / f"{pid}.typ"
    target = typ.with_suffix(".html") if html else (typ.with_suffix(".pdf") if pdf else typ)
    try:
        stamp = max(md.stat().st_mtime, BIN.stat().st_mtime)
        if target.exists() and target.stat().st_mtime > stamp:
            return (pid, name, "ok", True)
    except OSError:
        pass
    env = dict(
        os.environ,
        NLAB_FILES_ROOT=str(FILES_ROOT),
        NLAB_PAGEIDX=str(ROOT / "build/page-index.tsv"),
    )
    try:
        r = subprocess.run(
            [str(BIN), "page", name],
            input=md.read_text(errors="replace"),
            capture_output=True,
            text=True,
            timeout=120,
            env=env,
        )
    except subprocess.TimeoutExpired:
        return (pid, name, "convert-timeout", False)
    if r.returncode != 0:
        return (pid, name, "convert-fail: " + r.stderr[:120], False)
    typ.write_text(r.stdout)
    if html:
        try:
            c = subprocess.run(
                [
                    "typst", "compile", "--format", "html", "--features", "html",
                    str(typ), str(typ.with_suffix(".html")),
                ],
                capture_output=True,
                text=True,
                timeout=300,
                cwd=str(OUT),
            )
        except subprocess.TimeoutExpired:
            return (pid, name, "compile-timeout", False)
        if c.returncode != 0:
            return (pid, name, "compile-fail: " + c.stderr.splitlines()[0][:140], False)
        out = typ.with_suffix(".html")
        out.write_text(shellify(out.read_text(errors="replace"), name))
        return (pid, name, "ok", False)
    if not pdf:
        return (pid, name, "ok", False)
    try:
        c = subprocess.run(
            ["typst", "compile", "--format", "pdf", str(typ), str(typ.with_suffix(".pdf"))],
            capture_output=True,
            text=True,
            timeout=300,
            cwd=str(OUT),
        )
    except subprocess.TimeoutExpired:
        return (pid, name, "compile-timeout", False)
    if c.returncode != 0:
        return (pid, name, "compile-fail: " + c.stderr.splitlines()[0][:140], False)
    return (pid, name, "ok", False)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--limit", type=int)
    ap.add_argument("--no-pdf", action="store_true")
    ap.add_argument("--html", action="store_true")
    ap.add_argument("--jobs", type=int, default=8)
    args = ap.parse_args()

    OUT.mkdir(parents=True, exist_ok=True)
    files_link = OUT / "files"
    if not files_link.exists():
        files_link.symlink_to(FILES_ROOT)
    web_link = OUT / "files-web"
    if not web_link.exists():
        web_link.symlink_to(WEBP_CACHE)
    WEBP_CACHE.mkdir(parents=True, exist_ok=True)
    # pages link ../assets/*.css; mirror the shared sheets there so the
    # generated pages preview correctly straight from build/typ-pages
    assets = ROOT / "build/assets"
    if assets.is_dir():
        import shutil
        shutil.copy(HERE / "docset_shell.css", assets / "nlab-typst.css")
        tc = ROOT / "build/typ-assets/typst.css"
        if tc.exists():
            shutil.copy(tc, assets / "typst.css")
    FILES_ROOT.mkdir(parents=True, exist_ok=True)

    pages = sorted(BUILD.rglob("content.md"))
    if args.limit:
        pages = pages[: args.limit]

    t0 = time.time()
    ok = cached = 0
    fails = []
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
        futs = [ex.submit(one, p, not args.no_pdf, args.html) for p in pages]
        for i, f in enumerate(concurrent.futures.as_completed(futs), 1):
            pid, name, status, was_cached = f.result()
            if status == "ok":
                ok += 1
                cached += was_cached
            else:
                fails.append((pid, name, status))
            if i % 500 == 0:
                el = time.time() - t0
                print(f"{i}/{len(pages)} ok={ok} fail={len(fails)}"
                      f" ({el:.0f}s, {i/el:.1f}/s)", flush=True)

    REPORT.parent.mkdir(exist_ok=True)
    with open(REPORT, "w") as f:
        f.write(f"{ok}/{len(pages)} ok ({cached} cached), {len(fails)} failed\n\n")
        for pid, name, status in sorted(fails):
            f.write(f"{pid}\t{name}\t{status}\n")
    print(f"done: {ok}/{len(pages)} ok, {len(fails)} failed -> {REPORT}")


if __name__ == "__main__":
    main()
