"""Fetch the nLab's file-based mirrors and the live site's CSS/JS.

Sources (never committed):
  github.com/ncatlab/nlab-content       page sources (Markdown + itex)
  github.com/ncatlab/nlab-content-html  rendered pages (docset only)
  ncatlab.org/stylesheets, /javascripts page styling and theorem numbering
                                        (docset only)

Downloads are complete before anything is replaced: a mirror tarball is
fetched to a temp file (with retries), extracted next to the old copy and
swapped in only on success, so a failed refresh keeps the previous mirror.

Usage: nlab fetch [--refresh] [--html] [--assets]
"""

import argparse
import shutil
import tarfile
import time
import urllib.error
import urllib.request
from pathlib import Path

from .paths import HTML_MIRROR, MIRROR, WEB_ASSETS

USER_AGENT = "ncatlab-dash/1.0 (+https://github.com/taliesinb/ncatlab-dash)"
TARBALL = "https://github.com/ncatlab/{repo}/archive/refs/heads/master.tar.gz"
WEB_ASSET_PATHS = (
    "stylesheets/instiki.css",
    "stylesheets/mathematics.css",
    "stylesheets/syntax.css",
    "stylesheets/nlab.css",
    "javascripts/prototype.js",
    "javascripts/page_helper.js",
    "javascripts/thm_numbering.js",
)


def download(url: str, dest: Path, attempts: int = 3) -> None:
    """Stream url to dest atomically (dest.part, then rename)."""
    part = dest.with_name(dest.name + ".part")
    dest.parent.mkdir(parents=True, exist_ok=True)
    for attempt in range(1, attempts + 1):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(req, timeout=120) as r, open(part, "wb") as f:
                shutil.copyfileobj(r, f, 1 << 20)
            part.replace(dest)
            return
        except (urllib.error.URLError, OSError) as e:
            part.unlink(missing_ok=True)
            if attempt == attempts:
                raise SystemExit(f"error: downloading {url}: {e}")
            print(f"  retrying after {e}", flush=True)
            time.sleep(5 * attempt)


def fetch_mirror(repo: str, dest: Path, refresh: bool = False) -> None:
    if dest.is_dir() and not refresh:
        print(f"using cached {dest} (pass --refresh to re-download)")
        return
    tarball = dest.with_name(dest.name + ".tar.gz")
    tmp = dest.with_name(dest.name + ".tmp")
    shutil.rmtree(tmp, ignore_errors=True)
    print(f"downloading {repo} ...", flush=True)
    download(TARBALL.format(repo=repo), tarball)
    print(f"extracting {tarball.name} ...", flush=True)
    with tarfile.open(tarball) as tf:
        for m in tf:
            # strip the archive's top-level `<repo>-master/` directory
            parts = m.name.split("/", 1)
            if len(parts) < 2 or not parts[1]:
                continue
            m.name = parts[1]
            tf.extract(m, tmp, filter="data")
    tarball.unlink()
    shutil.rmtree(dest, ignore_errors=True)
    tmp.rename(dest)
    print(f"{repo} -> {dest}")


def fetch_web_assets(refresh: bool = False) -> None:
    for path in WEB_ASSET_PATHS:
        dest = WEB_ASSETS / Path(path).name
        if dest.exists() and not refresh:
            continue
        download(f"https://ncatlab.org/{path}", dest)
        print(f"fetched {path}")


def main(argv=None):
    ap = argparse.ArgumentParser(prog="nlab fetch", description=__doc__.split("\n\n")[0])
    ap.add_argument("--refresh", action="store_true", help="re-download even if cached")
    ap.add_argument("--html", action="store_true",
                    help="also the rendered-HTML mirror (for the docset)")
    ap.add_argument("--assets", action="store_true",
                    help="also the live site's CSS/JS (for the docset)")
    args = ap.parse_args(argv)
    fetch_mirror("nlab-content", MIRROR, args.refresh)
    if args.html:
        fetch_mirror("nlab-content-html", HTML_MIRROR, args.refresh)
    if args.assets:
        fetch_web_assets(args.refresh)


if __name__ == "__main__":
    main()
