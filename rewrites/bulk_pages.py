#!/usr/bin/env python3
"""Bulk-convert the whole nLab content mirror to typst pages.

Every pages/**/content.md becomes build/typ-pages/<id>.typ (+ .html with
--html, + .pdf unless --no-pdf). Content output is never committed, like
the mirrors. A failure summary lands in out/bulk-report.txt.

Conversion cache (table `conversions` in build/state.db): each page is
keyed on a hash of everything its output depends on - its source after
edits/ overlays, its title, how each of its [[wiki link]] targets
resolves in build/page-index.tsv, the build/files.tsv entries of the
images it references, and a fingerprint of the tools (nlab-typ binary,
installed mitex package, typst version, this script, the svgo config).
A page is re-converted only when its key changes, so e.g. a new redirect
alias re-converts just the pages whose links it changes. Failures are
cached too (--retry-failed to try them again). Resumable: results are
committed as they come in.

Usage: bulk_pages.py [--html] [--no-pdf] [--jobs J] [--limit N]
                     [--pages ID,ID] [--force] [--retry-failed]
"""

import argparse
import concurrent.futures
import hashlib
import os
import pathlib
import platform
import re
import sqlite3
import subprocess
import sys
import threading
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from downloads import read_map, urls_in  # noqa: E402

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
BUILD = ROOT / "build/nlab-content/pages"
OUT = ROOT / "build/typ-pages"
BIN = ROOT / "crates/nlab-typ/target/release/nlab-typ"
REPORT = HERE / "out/bulk-report.txt"
FILES_ROOT = ROOT / "build/files"  # content-addressed store (rewrites/downloads.py)
FILES_MAP = ROOT / "build/files.tsv"  # URL -> store path, read by nlab-typ
PAGEIDX = ROOT / "build/page-index.tsv"  # rewrites/page_index.py
STATE_DB = ROOT / "build/state.db"  # shared with rewrites/downloads.py
TYPST_CSS = ROOT / "build/typ-assets/typst.css"


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


_css_lock = threading.Lock()
_css_saved = False


def save_typst_css(css: str):
    global _css_saved
    if _css_saved:
        return
    with _css_lock:
        if not _css_saved:
            if not TYPST_CSS.exists() or TYPST_CSS.read_text() != css:
                TYPST_CSS.parent.mkdir(parents=True, exist_ok=True)
                TYPST_CSS.write_text(css)
            _css_saved = True


def shellify(html: str, name: str) -> str:
    """Docset post-pass on typst's HTML export: shared stylesheets
    instead of inline <style>, a <title>, cached-minified inline SVGs,
    and webp re-encoded images."""
    probe = '<meta name="viewport" content="width=device-width, initial-scale=1">'
    title = name.replace("&", "&amp;").replace("<", "&lt;")
    # typst's inline stylesheet is identical on every page: extract once
    # per run (rewritten if a new typst version changed it)
    m = STYLE_RE.search(html)
    if m:
        save_typst_css(m.group(0)[len("<style>"):-len("</style>")])
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


# ------------------------------------------------------ conversion cache

CONVERSIONS_SCHEMA = """
CREATE TABLE IF NOT EXISTS conversions(
  page TEXT PRIMARY KEY,
  name TEXT,
  mode TEXT NOT NULL,       -- typ | pdf | html
  key TEXT NOT NULL,        -- sha256 over every input (see module doc)
  status TEXT NOT NULL,     -- ok | fail
  error TEXT,
  seconds REAL,
  converted REAL
)"""

WIKI_RE = re.compile(r"\[\[([^\[\]]+)\]\]")


def file_digest(p: pathlib.Path) -> bytes:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.digest()


def mitex_package() -> pathlib.Path:
    base = (pathlib.Path.home() / "Library/Application Support"
            if platform.system() == "Darwin"
            else pathlib.Path(os.environ.get("XDG_DATA_HOME",
                                             pathlib.Path.home() / ".local/share")))
    return base / "typst/packages/local/mitex/0.2.7"


def tool_fingerprint(mode: str) -> str:
    """Changes whenever a tool that shapes every page changes."""
    h = hashlib.sha256(mode.encode())
    for p in (BIN, pathlib.Path(__file__).resolve(), HERE / "svgo.config.mjs"):
        h.update(file_digest(p) if p.exists() else b"-")
    pkg = mitex_package()
    if pkg.is_dir():
        for f in sorted(p for p in pkg.rglob("*") if p.is_file()):
            h.update(str(f.relative_to(pkg)).encode() + file_digest(f))
    else:
        h.update(b"no-mitex-package")
    try:
        h.update(subprocess.run(["typst", "--version"], capture_output=True,
                                text=True, timeout=30).stdout.encode())
    except OSError:
        h.update(b"no-typst")
    return h.hexdigest()


def load_page_index() -> dict[str, str]:
    """As nlab-typ reads it: lowercased, first entry wins."""
    idx: dict[str, str] = {}
    if PAGEIDX.exists():
        for line in PAGEIDX.read_text(encoding="utf-8").splitlines():
            name, tab, pid = line.partition("\t")
            if tab:
                idx.setdefault(name.strip().lower(), pid.strip())
    return idx


def page_key(tool: str, name: str, source: str, pidx: dict, fmap: dict) -> str:
    """Inputs are over-approximated (every [[...]] and every URL in the
    raw source), so the key can change needlessly but never miss a
    change nlab-typ would see."""
    links = set()
    for inner in WIKI_RE.findall(source):
        if inner.startswith("!"):
            continue
        target = inner.split("|", 1)[0].strip().lower()
        if not target.endswith(":file"):
            links.add((target, pidx.get(target, "")))
    images = sorted((u, fmap[u][0]) for u in urls_in(source)
                    if u in fmap and fmap[u][1] != "pdf")
    h = hashlib.sha256()
    for part in (tool, name, source, repr(sorted(links)), repr(images)):
        h.update(part.encode("utf-8", "surrogatepass"))
        h.update(b"\0")
    return h.hexdigest()


def target_of(pid: str, mode: str) -> pathlib.Path:
    return OUT / f"{pid}.{mode}"


# ------------------------------------------------------------ conversion

def one(pid: str, name: str, source: str, pdf: bool, html: bool = False):
    """Convert one page; returns (pid, name, status) with status 'ok' or
    an error description."""
    typ = OUT / f"{pid}.typ"
    env = dict(
        os.environ,
        NLAB_FILES_MAP=str(FILES_MAP),
        NLAB_PAGEIDX=str(PAGEIDX),
    )
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
        return (pid, name, "convert-timeout")
    if r.returncode != 0:
        return (pid, name, "convert-fail: " + r.stderr[:120])
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
            return (pid, name, "compile-timeout")
        if c.returncode != 0:
            return (pid, name, "compile-fail: " + first_line(c.stderr)[:140])
        out = typ.with_suffix(".html")
        out.write_text(shellify(out.read_text(errors="replace"), name))
        return (pid, name, "ok")
    if not pdf:
        return (pid, name, "ok")
    try:
        c = subprocess.run(
            ["typst", "compile", "--format", "pdf", str(typ), str(typ.with_suffix(".pdf"))],
            capture_output=True,
            text=True,
            timeout=300,
            cwd=str(OUT),
        )
    except subprocess.TimeoutExpired:
        return (pid, name, "compile-timeout")
    if c.returncode != 0:
        return (pid, name, "compile-fail: " + first_line(c.stderr)[:140])
    return (pid, name, "ok")


def timed(*args):
    t = time.time()
    return one(*args) + (time.time() - t,)


def first_line(text: str) -> str:
    return next((l for l in text.splitlines() if l.strip()), "(no output)")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--limit", type=int)
    ap.add_argument("--no-pdf", action="store_true")
    ap.add_argument("--html", action="store_true")
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--pages", help="comma-separated page ids (implies no pruning)")
    ap.add_argument("--force", action="store_true", help="ignore the conversion cache")
    ap.add_argument("--retry-failed", action="store_true",
                    help="re-run pages whose cached result is a failure")
    args = ap.parse_args()
    mode = "html" if args.html else ("typ" if args.no_pdf else "pdf")

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
    if args.pages:
        want = set(args.pages.split(","))
        pages = [p for p in pages if p.parent.name in want]
    if args.limit:
        pages = pages[: args.limit]
    if not (args.limit or args.pages):
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

    con = sqlite3.connect(STATE_DB, timeout=60)
    con.execute("PRAGMA journal_mode=WAL")
    con.execute(CONVERSIONS_SCHEMA)
    if not (args.limit or args.pages):
        live = [(p.parent.name,) for p in pages]
        con.execute("CREATE TEMP TABLE live(page TEXT PRIMARY KEY)")
        con.executemany("INSERT INTO live VALUES (?)", live)
        con.execute("DELETE FROM conversions WHERE page NOT IN (SELECT page FROM live)")
    prev = {r[0]: r[1:] for r in con.execute(
        "SELECT page, mode, key, status, error FROM conversions")}

    print("computing page keys ...", flush=True)
    tool = tool_fingerprint(mode)
    pidx, fmap = load_page_index(), read_map(FILES_MAP)
    ok = cached = 0
    fails = []
    todo = []  # (pid, name, source, key)
    for md in pages:
        pid = md.parent.name
        nf = md.parent / "name"
        name = nf.read_text(encoding="utf-8").strip() if nf.exists() else pid
        source, edit_problems = apply_edits(
            pid, md.read_text(encoding="utf-8", errors="replace"))
        key = page_key(tool, name, source, pidx, fmap)
        p = prev.get(pid)
        if p and p[0] == mode and p[1] == key and not args.force:
            if p[2] == "ok" and target_of(pid, mode).exists():
                ok += 1
                cached += 1
                continue
            if p[2] != "ok" and not args.retry_failed:
                fails.append((pid, name, f"{p[3]} (cached)"))
                cached += 1
                continue
        if edit_problems:
            print(f"  [edits] {pid} {name}: {edit_problems}", flush=True)
        todo.append((pid, name, source, key))
    print(f"{len(pages)} pages: {cached} unchanged, {len(todo)} to convert", flush=True)

    t0 = last_commit = time.time()
    keys = {pid: key for pid, _, _, key in todo}
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
        futs = [ex.submit(timed, pid, name, source, not args.no_pdf, args.html)
                for pid, name, source, _ in todo]
        try:
            for i, f in enumerate(concurrent.futures.as_completed(futs), 1):
                pid, name, status, seconds = f.result()
                if status == "ok":
                    ok += 1
                else:
                    fails.append((pid, name, status))
                con.execute(
                    "INSERT OR REPLACE INTO conversions VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    (pid, name, mode, keys[pid], "ok" if status == "ok" else "fail",
                     None if status == "ok" else status, seconds, time.time()))
                if time.time() - last_commit > 5:
                    con.commit()
                    last_commit = time.time()
                if i % 500 == 0:
                    el = time.time() - t0
                    print(f"{i}/{len(todo)} converted, {len(fails)} failed"
                          f" ({el:.0f}s, {i/el:.1f}/s)", flush=True)
        except KeyboardInterrupt:
            print("\ninterrupted: finished pages are recorded; rerun to resume")
            ex.shutdown(wait=False, cancel_futures=True)
            con.commit()
            raise SystemExit(130)
    con.commit()

    REPORT.parent.mkdir(exist_ok=True)
    with open(REPORT, "w") as f:
        f.write(f"{ok}/{len(pages)} ok ({cached} cached), {len(fails)} failed\n\n")
        for pid, name, status in sorted(fails):
            f.write(f"{pid}\t{name}\t{status}\n")
    print(f"done: {ok}/{len(pages)} ok, {len(fails)} failed -> {REPORT}")


if __name__ == "__main__":
    main()
