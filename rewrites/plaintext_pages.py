#!/usr/bin/env python3
"""Plaintext dump of the generated HTML pages, for leak-scanning.

build/typ-pages/<id>.html -> build/typ-text/<id>.txt with <math>, <svg>,
<style>, <script> and the TOC <nav> dropped entirely, tags stripped,
entities resolved. Incremental: a page is re-extracted only when its
.html is newer than the existing .txt, so corpus-wide re-runs after a
partial regen are cheap.

Usage: plaintext_pages.py [--jobs N] [--force]
"""

import argparse
import concurrent.futures
import pathlib
import re
import sys
import time
from html.parser import HTMLParser

ROOT = pathlib.Path(__file__).resolve().parent.parent
PAGES = ROOT / "build/typ-pages"
OUT = ROOT / "build/typ-text"

DROP = {"math", "svg", "style", "script", "nav", "head", "title"}


class Extractor(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.depth = 0  # inside a dropped subtree
        self.chunks: list[str] = []
        self.block = {"p", "div", "li", "h1", "h2", "h3", "h4", "h5",
                      "h6", "tr", "blockquote", "figcaption", "figure"}

    def handle_starttag(self, tag, attrs):
        if tag in DROP:
            self.depth += 1
        elif self.depth == 0 and tag in self.block:
            self.chunks.append("\n")

    def handle_endtag(self, tag):
        if tag in DROP and self.depth > 0:
            self.depth -= 1

    def handle_data(self, data):
        if self.depth == 0:
            self.chunks.append(data)

    def text(self) -> str:
        t = "".join(self.chunks)
        t = re.sub(r"[ \t]+", " ", t)
        t = re.sub(r" ?\n ?", "\n", t)
        return re.sub(r"\n{3,}", "\n\n", t).strip() + "\n"


def one(html: pathlib.Path, force: bool) -> bool:
    txt = OUT / (html.stem + ".txt")
    if not force and txt.exists() and txt.stat().st_mtime > html.stat().st_mtime:
        return False
    ex = Extractor()
    try:
        ex.feed(html.read_text(errors="replace"))
    except Exception:
        pass
    txt.write_text(ex.text())
    return True


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--jobs", type=int, default=8)
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    htmls = sorted(PAGES.glob("*.html"))
    t0 = time.time()
    done = fresh = 0
    with concurrent.futures.ProcessPoolExecutor(args.jobs) as ex:
        for changed in ex.map(one, htmls, [args.force] * len(htmls),
                              chunksize=64):
            done += 1
            fresh += changed
            if done % 5000 == 0:
                print(f"  {done}/{len(htmls)} ({fresh} extracted)",
                      file=sys.stderr)
    print(f"{done} pages, {fresh} (re)extracted, {time.time()-t0:.0f}s "
          f"-> {OUT}")


if __name__ == "__main__":
    main()
