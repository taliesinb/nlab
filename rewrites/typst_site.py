#!/usr/bin/env python3
"""Emit the generated .typ pages as a browsable typst site.

Copies build/typ-pages/<id>.typ to dist/typst/<slug>.typ, where <slug>
is the page's canonical name lowercased and hyphenated. Duplicate slugs
get -<id> appended. Image references resolve through a per-file symlink
tree (with extensions corrected to match magic bytes, since typst picks
its decoder by extension), and _index.typ lists every page.

Usage: typst_site.py [--out DIR]
"""

import argparse
import pathlib
import re
import shutil
import unicodedata

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "build/typ-pages"
FILES_ROOT = ROOT / "build/nlab-files"


def slugify(name: str) -> str:
    s = unicodedata.normalize("NFKC", name).lower()
    # keep negative numbers distinct: "(-2)-category" vs "2-category"
    s = re.sub(r"\((-|−)(\d)", r"(minus-\2", s)
    s = re.sub(r"['’\"]", "", s)
    s = re.sub(r"[^\w]+", "-", s, flags=re.UNICODE)
    s = s.strip("-")
    return s or "page"


def load_names() -> dict[str, str]:
    """Canonical page names (page-index.tsv also lists redirect
    aliases, which must not steal a page's slug)."""
    names = {}
    for f in (ROOT / "build/nlab-content/pages").glob("*/*/*/*/*/name"):
        names[f.parent.name] = f.read_text(encoding="utf-8").strip()
    return names


def markup_escape(t: str) -> str:
    return re.sub(r"([\\#$\[\]*_`<>@])", r"\\\1", t)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(ROOT / "dist/typst"))
    ap.add_argument(
        "--link-base", default="../",
        help="prefix for cross-page links (default '../' for slug-dir routing)")
    ap.add_argument(
        "--link-ext", default="/",
        help="suffix for cross-page links (default '/'; use '.html' for flat files)")
    args = ap.parse_args()
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    names = load_names()
    files_dir = out / "files"
    if files_dir.is_symlink():
        files_dir.unlink()
    files_dir.mkdir(exist_ok=True)
    fixed: dict[str, str] = {}

    def resolve_image(name: str) -> str:
        """Uploads sometimes carry the wrong extension (PNG bytes in a
        .jpg); link under an extension matching the magic bytes."""
        if name in fixed:
            return fixed[name]
        src = FILES_ROOT / name
        target = name
        try:
            magic = src.open("rb").read(12)
            actual = None
            if magic.startswith(b"\x89PNG"):
                actual = "png"
            elif magic.startswith(b"\xff\xd8"):
                actual = "jpg"
            elif magic[:6] in (b"GIF87a", b"GIF89a"):
                actual = "gif"
            elif magic[:4] == b"RIFF" and magic[8:12] == b"WEBP":
                actual = "webp"
            ext = name.rsplit(".", 1)[-1].lower()
            norm = {"jpeg": "jpg"}.get(ext, ext)
            if actual and norm != actual:
                target = f"{name}.{actual}"
        except OSError:
            pass
        link = files_dir / target
        if not (link.exists() or link.is_symlink()):
            link.symlink_to(src)
        fixed[name] = target
        return target

    img_re = re.compile(r'nlab-image\("files/([^"]+)"')
    page_re = re.compile(r'nlab-page\("(\d+)"')
    base_re = re.compile(
        r'(#let nlab-page-base = sys\.inputs\.at\("nlab-base", default: )""\)'
    )
    ext_re = re.compile(
        r'(#let nlab-page-ext = sys\.inputs\.at\("nlab-ext", default: )"\.html"\)'
    )

    # assign every slug first so cross-links can resolve forward
    taken: dict[str, str] = {}
    slug_of: dict[str, str] = {}
    pages = sorted(SRC.glob("*.typ"), key=lambda p: int(p.stem))
    # exact-name pages claim their slug first, so a mangled name
    # ("(-2)-category") can never steal "2-category" from the real one
    def exactness(pid: str) -> int:
        name = names.get(pid, pid)
        return 0 if slugify(name) == name.lower().replace(" ", "-") else 1
    for typ in sorted(pages, key=lambda p: (exactness(p.stem), int(p.stem))):
        pid = typ.stem
        slug = slugify(names.get(pid, pid))
        if slug in taken:
            slug = f"{slug}-{pid}"
        taken[slug] = pid
        slug_of[pid] = slug

    rows = []
    for typ in pages:
        pid = typ.stem
        slug = slug_of[pid]
        text = typ.read_text(errors="replace")
        text = img_re.sub(
            lambda m: f'nlab-image("files/{resolve_image(m.group(1))}"',
            text,
        )
        # cross-links by canonical slug instead of page id
        text = page_re.sub(
            lambda m: f'nlab-page("{slug_of.get(m.group(1), m.group(1))}"',
            text,
        )
        text = base_re.sub(f'\\g<1>"{args.link_base}")', text, count=1)
        text = ext_re.sub(f'\\g<1>"{args.link_ext}")', text, count=1)
        (out / f"{slug}.typ").write_text(text)
        rows.append((names.get(pid, pid), slug, pid))

    # prune pages whose slug changed since the last run
    keep = set(taken) | {"_index"}
    for old in out.glob("*.typ"):
        if old.stem not in keep:
            old.unlink()

    index = ["#set page(margin: 2cm)", "= nLab typst pages", ""]
    for name, slug, pid in sorted(rows, key=lambda r: r[0].lower()):
        index.append(
            f'#link("{slug}.typ")[{markup_escape(name)}]'
            f' #h(1fr) #text(gray, "{pid}") \\'
        )
    (out / "_index.typ").write_text("\n".join(index) + "\n")

    # static stylesheets for HTML rendering of these pages
    assets = out / "assets"
    assets.mkdir(exist_ok=True)
    shutil.copy(
        pathlib.Path(__file__).parent / "docset_shell.css",
        assets / "nlab-typst.css",
    )
    typst_css = ROOT / "build/typ-assets/typst.css"
    if typst_css.exists():
        shutil.copy(typst_css, assets / "typst.css")
    print(f"{len(rows)} pages -> {out}")


if __name__ == "__main__":
    main()
