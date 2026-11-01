"""`nlab COMMAND [ARGS]`: the single entry point (`uv run nlab ...`)."""

import difflib
import importlib
import sys

# command -> (module:function, summary); each function takes argv
COMMANDS = {
    "build-site": ("nlab.build:build_site",
                   "everything, from a clean checkout to dist/html"),
    "build-docset": ("nlab.build:build_docset", "dist/nLab.docset for Dash"),
    "build-typst-site": ("nlab.typst_site:main",
                         "dist/typst: the pages as slug-named .typ sources"),
    "setup": ("nlab.toolchain:main",
              "check tools; build + install the mitex typst package"),
    "fetch": ("nlab.mirror:main", "download the wiki mirror(s)"),
    "page-index": ("nlab.page_index:main", "build/page-index.tsv (wiki links)"),
    "download": ("nlab.downloads:main",
                 "fetch referenced files per a rules config (build/files)"),
    "convert": ("nlab.convert:main",
                "pages -> typst -> HTML/PDF (build/typ-pages), cached"),
    "assemble-site": ("nlab.site:main",
                      "lay out dist/html from build/typ-pages, + search index"),
    "convert-page": ("nlab.build:convert_page",
                     "one page -> typst on stdout (debugging)"),
    "status": ("nlab.build:status", "downloads and conversions in build/state.db"),
    "deploy-netlify": ("nlab.deploy:netlify_main", "deploy dist/html to Netlify"),
    "deploy-symbol": ("nlab.deploy:main",
                      "deploy dist/html to a symbol static host"),
}


def usage() -> str:
    w = max(map(len, COMMANDS))
    lines = ["usage: nlab COMMAND [ARGS]   (nlab COMMAND --help for its options)", "",
             "commands:"]
    lines += [f"  {c:{w}}  {summary}" for c, (_, summary) in COMMANDS.items()]
    return "\n".join(lines)


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if not argv or argv[0] in ("-h", "--help", "help"):
        print(usage())
        return
    cmd, rest = argv[0], argv[1:]
    if cmd not in COMMANDS:
        close = difflib.get_close_matches(cmd, COMMANDS, n=1)
        sys.exit(f"nlab: unknown command {cmd!r}"
                 + (f"; did you mean {close[0]!r}?" if close else "")
                 + "\n\n" + usage())
    target, _ = COMMANDS[cmd]
    module, func = target.split(":")
    getattr(importlib.import_module(module), func)(rest)


if __name__ == "__main__":
    main()
