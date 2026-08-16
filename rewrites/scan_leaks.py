#!/usr/bin/env python3
"""Scan the plaintext page dump (plaintext_pages.py) for markup that
leaked through the pipeline as literal text: raw HTML, markdown, latex,
typst calls, unresolved placeholders.

Usage: scan_leaks.py [--samples N] [--class NAME]
"""

import argparse
import collections
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
TXT = ROOT / "build/typ-text"

CLASSES = {
    # raw html leaked as text
    "html-tag": re.compile(r"</?(?:a|div|span|img|em|strong|table|tr|td|ul|ol|li|p|br|h[1-6])\b[^>]*>"),
    "html-href": re.compile(r"\bhref\s*="),
    "html-entity": re.compile(r"&(?:lbrack|rbrack|amp|lt|gt|nbsp|#\d+);"),
    # markdown leftovers
    "md-link": re.compile(r"\]\(https?://"),
    "md-wikilink": re.compile(r"\[\[[^\]]{1,60}\]\]"),
    "md-emph": re.compile(r"(?<![*\w])\*\*[A-Za-z][^*]{0,60}\*\*"),
    "md-header": re.compile(r"(?m)^#{1,4} [A-Za-z]"),
    # latex leftovers
    "latex-env": re.compile(r"\\(?:begin|end)\{[a-z*]+\}"),
    "latex-cmd": re.compile(r"\\(?:label|ref|cite|includegraphics|hspace|vspace|newcommand)\b"),
    # typst leakage
    "typst-call": re.compile(r"#(?:link|nlab-page|nlab-image|nlab-env|metadata|label|mi)\("),
    "typst-mi": re.compile(r"\bmi\(\""),
    # unresolved placeholders
    "pua-placeholder": re.compile(r"[\ue000-\ue031]"),
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--samples", type=int, default=3)
    ap.add_argument("--class", dest="only")
    ap.add_argument("--list", action="store_true",
                    help="print every matching page id for the class")
    args = ap.parse_args()
    classes = {args.only: CLASSES[args.only]} if args.only else CLASSES

    hits = collections.defaultdict(list)  # class -> [(pid, example line)]
    for txt in sorted(TXT.glob("*.txt")):
        t = txt.read_text(errors="replace")
        for name, rx in classes.items():
            m = rx.search(t)
            if m:
                line = t[max(0, m.start() - 60):m.end() + 60].replace("\n", " ")
                hits[name].append((txt.stem, line.strip()))

    for name in classes:
        pages = hits.get(name, [])
        print(f"{name}: {len(pages)} pages")
        if args.list:
            print("  " + ",".join(p for p, _ in pages))
        else:
            for pid, line in pages[:args.samples]:
                print(f"  [{pid}] ...{line}...")


if __name__ == "__main__":
    main()
