#!/usr/bin/env python3
"""Deploy dist/html: to Netlify, or to a symbol static host.

Netlify (`nlab deploy-netlify`) wraps `netlify deploy --dir dist/html`
(production unless --draft); Netlify's own diffing uploads only changed
files, and dist/html/_redirects sends / to search.html. The site ID is not
kept in this repository: it comes from --site, $NLAB_NETLIFY_SITE, or the
gitignored .netlify/state.json that a one-time `netlify link --id SITE_ID`
writes.

Symbol (`nlab deploy-symbol`): the initial upload streams the tree as a
series of tar.gz chunks (the server unpacks archives up to 50 MiB
compressed / 80 MiB extracted and merges them into the site). Subsequent
runs diff local blake3 hashes against GET /<site>/FILES and PUT only
changed or new files. Symbol has no _redirects, so a root index.html
redirecting to search.html is generated.

Usage:
  nlab deploy-netlify [--site ID] [--dir dist/html] [--draft] [--dry-run]
  nlab deploy-symbol [--site nlab] [--dir dist/html] [--full] [--dry-run]
"""

import argparse
import io
import json
import os
import pathlib
import shutil
import subprocess
import tarfile
import time
import urllib.error
import urllib.parse
import urllib.request

import blake3

from .paths import DIST, ROOT


def netlify_site(explicit: str | None) -> tuple[str | None, str]:
    """(site id, where it came from); None means the CLI's own link."""
    if explicit:
        return explicit, "--site"
    if os.environ.get("NLAB_NETLIFY_SITE"):
        return os.environ["NLAB_NETLIFY_SITE"], "$NLAB_NETLIFY_SITE"
    state = ROOT / ".netlify/state.json"
    if state.exists():
        try:
            site = json.loads(state.read_text()).get("siteId")
        except json.JSONDecodeError:
            site = None
        if site:
            return None, f"netlify link ({state.relative_to(ROOT)}: {site})"
    raise SystemExit(
        "error: which Netlify site? Pass --site ID, set NLAB_NETLIFY_SITE, or "
        "link this checkout once with `netlify link --id SITE_ID` (stored in the "
        "gitignored .netlify/state.json)")


def netlify_main(argv=None):
    ap = argparse.ArgumentParser(prog="nlab deploy-netlify",
                                 description="Deploy dist/html to Netlify")
    ap.add_argument("--site", help="site ID or name (default: $NLAB_NETLIFY_SITE, "
                                   "else the checkout's `netlify link`)")
    ap.add_argument("--dir", default=str(DIST / "html"))
    ap.add_argument("--draft", action="store_true",
                    help="deploy to a preview URL instead of production")
    ap.add_argument("--dry-run", action="store_true", help="print the command only")
    args = ap.parse_args(argv)

    base = pathlib.Path(args.dir)
    if not (base / "search.html").exists():
        raise SystemExit(f"error: {base} has no search.html; run `nlab build-site` "
                         "(or `nlab assemble-site`) first")
    if not shutil.which("netlify"):
        raise SystemExit("error: the Netlify CLI is not installed "
                         "(`npm install -g netlify-cli`)")
    site, source = netlify_site(args.site)
    cmd = ["netlify", "deploy", "--dir", str(base)]
    if not args.draft:
        cmd.append("--prod")
    if site:
        cmd += ["--site", site]
    print(f"site from {source}\n$ {' '.join(cmd)}", flush=True)
    if not args.dry_run:
        subprocess.run(cmd, cwd=ROOT, check=True)

SERVER = "http://SYMBOL-HOST"
CHUNK_EXTRACTED = 16 * 1024 * 1024

ROOT_INDEX = (
    '<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">'
    '<meta http-equiv="refresh" content="0; url=search.html">'
    '<title>nLab</title></head><body>'
    '<a href="search.html">search</a></body></html>'
)


def local_manifest(base: pathlib.Path) -> dict[str, pathlib.Path]:
    files = {}
    for f in base.rglob("*"):
        if f.is_file() and not f.is_symlink() or (f.is_symlink() and f.is_file()):
            rel = f.relative_to(base).as_posix()
            if rel in ("_redirects", "symbol.toml"):
                continue
            files[rel] = f
    return files


def remote_manifest(site: str) -> dict[str, str] | None:
    req = urllib.request.Request(
        f"{SERVER}/{site}/FILES", headers={"Accept": "application/json"}
    )
    try:
        with urllib.request.urlopen(req) as r:
            data = json.load(r)
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise
    return {f["path"]: f["hash"] for f in data.get("files", [])}


def local_hashes(paths: list[pathlib.Path], base: pathlib.Path) -> dict[str, str]:
    """blake3 of every file, in the server's `blake3:<hex>` form."""
    hashes: dict[str, str] = {}
    for p in paths:
        h = blake3.blake3(max_threads=blake3.blake3.AUTO)
        h.update_mmap(p)
        hashes[p.relative_to(base).as_posix()] = "blake3:" + h.hexdigest()
    return hashes


def put_file(site: str, rel: str, path: pathlib.Path):
    url = f"{SERVER}/{site}/" + urllib.parse.quote(rel)
    subprocess.run(
        ["curl", "-fsS", "-o", "/dev/null", "-T", str(path), url],
        check=True,
    )


CHUNK_COMPRESSED = 12 * 1024 * 1024


def push_tar(site: str, members: list[tuple[str, pathlib.Path]], n, total):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        # the server strips a lone top-level directory from archives;
        # a root-level marker member defeats that heuristic
        marker = tarfile.TarInfo(".symbol-keep")
        marker.size = 0
        tf.addfile(marker, io.BytesIO(b""))
        for rel, path in members:
            tf.add(path, arcname=rel)
    data = buf.getvalue()
    if len(data) > CHUNK_COMPRESSED and len(members) > 1:
        # incompressible content (images, pagefind fragments): split
        mid = len(members) // 2
        push_tar(site, members[:mid], f"{n}a", total)
        push_tar(site, members[mid:], f"{n}b", total)
        return
    print(f"  chunk {n}/{total}: {len(members)} files, "
          f"{len(data) / 1e6:.1f} MB compressed", flush=True)
    req = urllib.request.Request(
        f"{SERVER}/{site}",
        data=data,
        method="PUT",
        headers={"Content-Type": "application/gzip", "unpack": "1"},
    )
    for attempt in range(3):
        try:
            with urllib.request.urlopen(req, timeout=600) as r:
                r.read()
            return
        except urllib.error.HTTPError as e:
            if e.code >= 500:
                raise SystemExit(
                    f"server error {e.code}; stopping (resume by rerunning)")
            if e.code == 413 and len(members) > 1:
                # the request cap in practice is lower than documented:
                # split until the server accepts
                mid = len(members) // 2
                push_tar(site, members[:mid], f"{n}a", total)
                push_tar(site, members[mid:], f"{n}b", total)
                return
            if attempt == 2:
                raise
            print(f"    retry after {e}", flush=True)
            time.sleep(3)
        except Exception as e:
            if attempt == 2:
                raise
            print(f"    retry after {e}", flush=True)
            time.sleep(3)


def main(argv=None):
    ap = argparse.ArgumentParser(prog="nlab deploy-symbol",
                                 description="Upload dist/html to http://SYMBOL-HOST/<site>/")
    ap.add_argument("--site", default="nlab")
    ap.add_argument("--dir", default=str(DIST / "html"))
    ap.add_argument("--full", action="store_true",
                    help="upload everything, skip the remote diff")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args(argv)
    base = pathlib.Path(args.dir)

    # symbol has no _redirects support: real root index instead
    idx = base / "index.html"
    if not idx.exists() or "url=search.html" in idx.read_text()[:200]:
        idx.write_text(ROOT_INDEX)

    files = local_manifest(base)
    print(f"{len(files)} local files", flush=True)

    remote = None if args.full else remote_manifest(args.site)
    if remote is None:
        todo = sorted(files)
        print(f"initial upload of {len(todo)} files", flush=True)
    else:
        print(f"{len(remote)} remote files; hashing local tree...", flush=True)
        hashes = local_hashes([files[r] for r in sorted(files)], base)
        todo = sorted(
            r for r in files
            if remote.get(r) != hashes[r]
        )
        stale = sorted(set(remote) - set(files) - {"symbol.toml"})
        print(f"{len(todo)} changed/new, {len(stale)} stale on server",
              flush=True)
        if stale and not args.dry_run:
            failed = 0
            for r in stale:
                url = f"{SERVER}/{args.site}/" + urllib.parse.quote(r)
                rc = subprocess.run(["curl", "-fsS", "-o", "/dev/null",
                                     "-X", "DELETE", url], check=False)
                failed += rc.returncode != 0
            print(f"deleted {len(stale) - failed} stale files"
                  + (f" ({failed} already gone)" if failed else ""),
                  flush=True)

    if args.dry_run:
        for r in todo[:20]:
            print("  would upload", r)
        if len(todo) > 20:
            print(f"  ... and {len(todo) - 20} more")
        return

    # small change sets go as individual PUTs; big ones as tar chunks
    if len(todo) <= 50:
        for r in todo:
            put_file(args.site, r, files[r])
            print("  put", r, flush=True)
    else:
        chunks: list[list[tuple[str, pathlib.Path]]] = [[]]
        size = 0
        for r in todo:
            fsize = files[r].stat().st_size
            if size + fsize > CHUNK_EXTRACTED and chunks[-1]:
                chunks.append([])
                size = 0
            chunks[-1].append((r, files[r]))
            size += fsize
        for i, members in enumerate(chunks, 1):
            push_tar(args.site, members, i, len(chunks))
            time.sleep(2)  # let the server breathe between unpacks

    subprocess.run(["curl", "-fsS", "-o", "/dev/null", "-X", "DELETE",
                    f"{SERVER}/{args.site}/.symbol-keep"], check=False)
    print(f"done: {SERVER}/{args.site}/", flush=True)


if __name__ == "__main__":
    main()
