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
FILES_ROOT = ROOT / "build/files"  # content-addressed store (rewrites/downloads.py)
FILES_MAP = ROOT / "build/files.tsv"  # URL -> store path, read by nlab-typ
PAGEIDX = ROOT / "build/page-index.tsv"  # rewrites/page_index.py


CSS = (HERE / "docset_shell.css").read_text()
EDITS = ROOT / "edits"


def apply_edits(pid: str, text: str) -> tuple[str, str | None]:
    """Overlay pointwise corrections from edits/<pid>.json onto the
    source before conversion. Each `find` must occur exactly once;
    misses and ambiguities are reported, not applied."""
    ef = EDITS / f"{pid}.json"
    if not ef.exists():
        return text, None
    import json

    try:
        spec = json.loads(ef.read_text())
    except json.JSONDecodeError as e:
        return text, f"edits/{pid}.json unparseable: {e}"
    problems = []
    for e in spec.get("edits", []):
        find = e.get("find", "")
        n = text.count(find) if find else 0
        if n == 1:
            text = text.replace(find, e.get("replace", ""), 1)
        elif n == 0:
            problems.append(f"stale edit (no match): {find[:50]!r}")
        else:
            problems.append(f"ambiguous edit ({n} matches): {find[:50]!r}")
    return text, "; ".join(problems) or None

SVG_CACHE = ROOT / "build/svgmin"
WEBP_CACHE = ROOT / "build/nlab-files-web"  # published as files-web/
CLASS_CACHE = ROOT / "build/nlab-files-class"  # line-art verdicts (not published)
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
    # keyed by the full name: X.png and X.jpg both exist in the store
    out = WEBP_CACHE / f"{name}.webp"
    if out.exists():
        return out.name
    WEBP_CACHE.mkdir(parents=True, exist_ok=True)
    tmp = WEBP_CACHE.parent / f".webp.{name}.{os.getpid()}.part"
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


def is_lineart(name: str) -> bool:
    """Line-art-on-white heuristic (near-white border + few distinct
    colors): such images invert cleanly in dark mode, photos must not.
    Cached as a .lineart/.photo sidecar in CLASS_CACHE. Without Pillow
    nothing is cached, so installing it later takes effect."""
    yes, no = CLASS_CACHE / f"{name}.lineart", CLASS_CACHE / f"{name}.photo"
    if yes.exists():
        return True
    if no.exists():
        return False
    try:
        from PIL import Image
    except ImportError:
        return False
    src = FILES_ROOT / name
    if not src.exists():
        return False
    verdict = False
    try:
        im = Image.open(src).convert("RGB")
        im.thumbnail((64, 64))
        px = list(im.getdata())
        w, h = im.size
        border = (px[:w] + px[-w:]
                  + [px[r * w] for r in range(h)]
                  + [px[r * w + w - 1] for r in range(h)])
        white = sum(1 for c in border if min(c) > 225) / max(len(border), 1)
        colors = len({(r >> 4, g >> 4, b >> 4) for r, g, b in px})
        verdict = white > 0.85 and colors < 400
    except Exception:
        pass
    CLASS_CACHE.mkdir(parents=True, exist_ok=True)
    (yes if verdict else no).touch()
    return verdict


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
        tag = m.group(0)
        w = webp_name(m.group(2))
        if w:
            tag = f"{m.group(1)}files-web/{w}{m.group(3)}"
        if is_lineart(m.group(2)):
            tag = tag.replace("<img ", '<img class="nlab-invert" ', 1)
        return tag

    return IMG_RE.sub(img, html)


def one(md: pathlib.Path, pdf: bool, html: bool = False):
    pid = md.parent.name
    name = (md.parent / "name").read_text().strip() if (md.parent / "name").exists() else pid
    typ = OUT / f"{pid}.typ"
    target = typ.with_suffix(".html") if html else (typ.with_suffix(".pdf") if pdf else typ)
    edit_file = EDITS / f"{pid}.json"
    try:
        stamp = max(md.stat().st_mtime, BIN.stat().st_mtime)
        if PAGEIDX.exists():  # rewritten only when its content changes
            stamp = max(stamp, PAGEIDX.stat().st_mtime)
        if edit_file.exists():
            stamp = max(stamp, edit_file.stat().st_mtime)
        if target.exists() and target.stat().st_mtime > stamp:
            return (pid, name, "ok", True)
    except OSError:
        pass
    env = dict(
        os.environ,
        NLAB_FILES_MAP=str(FILES_MAP),
        NLAB_PAGEIDX=str(PAGEIDX),
    )
    source, edit_problems = apply_edits(pid, md.read_text(errors="replace"))
    if edit_problems:
        print(f"  [edits] {pid} {name}: {edit_problems}", flush=True)
    try:
        r = subprocess.run(
            [str(BIN), "page", name],
            input=source,
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

    if not BIN.exists():
        raise SystemExit(f"error: {BIN} missing (cargo build --release in crates/nlab-typ)")
    if not PAGEIDX.exists():
        print(f"warning: {PAGEIDX} missing: every wiki link will point at "
              "ncatlab.org (run rewrites/page_index.py)", flush=True)
    if not FILES_MAP.exists():
        print(f"warning: {FILES_MAP} missing: no images will be embedded "
              "(run rewrites/downloads.py)", flush=True)
    OUT.mkdir(parents=True, exist_ok=True)
    files_link = OUT / "files"  # pages reference files/<hash>.<ext>
    if files_link.is_symlink() and files_link.resolve() != FILES_ROOT.resolve():
        files_link.unlink()  # pre-store builds pointed at build/nlab-files
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
    else:
        # pages deleted upstream (after a mirror refresh) must not linger:
        # html_site.py/typst_site.py publish whatever is in OUT
        ids = {p.parent.name for p in pages}
        gone = [f for f in OUT.iterdir()
                if (m := re.fullmatch(r"(\d+)\.(typ|html|pdf)", f.name))
                and m.group(1) not in ids]
        for f in gone:
            f.unlink()
        if gone:
            print(f"pruned {len(gone)} outputs of pages no longer in the mirror")

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
