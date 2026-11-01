#!/usr/bin/env python3
"""Write build/page-index.tsv: page name / redirect alias -> page id.

nlab-typ reads this (via NLAB_PAGEIDX, set by bulk_pages.py) to turn
[[wiki links]] into in-site links; a name missing from the index falls
back to an ncatlab.org URL, so without this file every cross-link
silently points at the live wiki.

One `name<TAB>id` line per entry. Lookups are case-insensitive and the
first entry wins, so all canonical page names are written before any
[[!redirects]] alias: an alias on one page can never shadow another
page's real name. An alias claimed by several pages (~190 of them,
e.g. "cocones" on both *cone* and *cocone*) goes to the claimant whose
name is most similar to it, ties to the lowest id. Only pages the HTML
site publishes are indexed: those with a content.md, minus history
subpages, placeholders and sandboxes (typst_site.skip_page), whose
links would otherwise dangle in dist/html; they fall back to ncatlab.org.

The file is rewritten only when its content changes. (bulk_pages.py's
conversion cache keys each page on how its own links resolve, so only
pages whose links change are re-converted.)

Usage: page_index.py
"""

import difflib
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from typst_site import skip_page  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
PAGES = ROOT / "build/nlab-content/pages"
OUT = ROOT / "build/page-index.tsv"

REDIRECT_RE = re.compile(r"\[\[!redirects\s+([^\]]+)\]\]")


def main():
    if not PAGES.is_dir():
        raise SystemExit(f"error: {PAGES} missing (fetch the nlab-content mirror first)")
    names: list[tuple[str, str]] = []
    claims: dict[str, list[tuple[str, str]]] = {}  # lower alias -> [(alias, pid)]
    dirs = sorted(
        (f.parent for f in PAGES.glob("*/*/*/*/*/content.md")),
        key=lambda d: int(d.name),
    )
    for d in dirs:
        pid = d.name
        name_file = d / "name"
        if not name_file.exists():
            continue
        name = name_file.read_text(encoding="utf-8", errors="replace").strip()
        if not name or skip_page(name):
            continue
        names.append((name, pid))
        src = (d / "content.md").read_text(encoding="utf-8", errors="replace")
        for m in REDIRECT_RE.finditer(src):
            alias = m.group(1).strip()
            if alias and "\n" not in alias and "\t" not in alias:
                claims.setdefault(alias.lower(), []).append((alias, pid))

    name_of = {pid: n for n, pid in names}
    taken = {n.lower() for n, _ in names}

    def closeness(alias: str, pid: str) -> tuple[float, float]:
        # case-insensitive first; exact case breaks ties (rings -> ring, not Ring)
        name = name_of[pid]
        return (
            difflib.SequenceMatcher(None, alias.lower(), name.lower()).ratio(),
            difflib.SequenceMatcher(None, alias, name).ratio(),
        )

    aliases: list[tuple[str, str]] = []
    contested = 0
    for key, cl in claims.items():
        if key in taken:
            continue  # a canonical name always wins
        pids = {pid for _, pid in cl}
        contested += len(pids) > 1
        best = max(pids, key=lambda p: (*closeness(cl[0][0], p), -int(p)))
        aliases.append((cl[0][0], best))
    aliases.sort(key=lambda a: (int(a[1]), a[0]))

    text = "".join(f"{n}\t{i}\n" for n, i in names + aliases)
    if OUT.exists() and OUT.read_text(encoding="utf-8") == text:
        print(f"{OUT} unchanged ({len(names)} names, {len(aliases)} aliases)")
        return
    OUT.write_text(text, encoding="utf-8")
    print(f"{len(names)} names + {len(aliases)} aliases "
          f"({contested} contested) -> {OUT}")


if __name__ == "__main__":
    main()
