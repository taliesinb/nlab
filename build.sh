#!/usr/bin/env bash
# Build the nLab Dash docset from the official file-based mirrors of the wiki.
#
# Usage: ./build.sh [--refresh]
#   --refresh   re-download the content mirrors and web assets
#
# Sources (never committed to this repo):
#   https://github.com/ncatlab/nlab-content-html  rendered pages (XHTML+MathML)
#   https://github.com/ncatlab/nlab-content      page sources, for [[!redirects]]
#   https://ncatlab.org/stylesheets/*.css         page styling
#   https://ncatlab.org/javascripts/*             theorem numbering
#
# Requires: curl, tar, Python 3.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
BUILD="$ROOT/build"
REFRESH="${1:-}"

fetch_mirror() { # fetch_mirror REPO DEST
    local repo="$1" dest="$BUILD/$2"
    if [[ -d "$dest" && "$REFRESH" != "--refresh" ]]; then
        echo "using cached $dest (pass --refresh to re-download)"
        return
    fi
    # download, extract, then swap: a failed fetch keeps the old mirror;
    # -m gives files fresh mtimes (see build_site.sh)
    rm -rf "$dest.tmp" "$dest.tar.gz"
    mkdir -p "$dest.tmp"
    echo "downloading $repo (this is large; a few hundred MB) ..."
    curl -fL --retry 3 -o "$dest.tar.gz" \
        "https://github.com/ncatlab/$repo/archive/refs/heads/master.tar.gz"
    tar xzmf "$dest.tar.gz" -C "$dest.tmp" --strip-components=1
    rm -f "$dest.tar.gz"
    rm -rf "$dest"
    mv "$dest.tmp" "$dest"
}

fetch_asset() { # fetch_asset URL-PATH
    local dest="$BUILD/assets/$(basename "$1")"
    if [[ -f "$dest" && "$REFRESH" != "--refresh" ]]; then return; fi
    mkdir -p "$BUILD/assets"
    curl -fsSL --retry 3 "https://ncatlab.org/$1" -o "$dest.part"
    mv "$dest.part" "$dest"
    echo "fetched $1"
}

fetch_mirror nlab-content-html nlab-content-html
fetch_mirror nlab-content nlab-content

fetch_asset stylesheets/instiki.css
fetch_asset stylesheets/mathematics.css
fetch_asset stylesheets/syntax.css
fetch_asset stylesheets/nlab.css
fetch_asset javascripts/prototype.js
fetch_asset javascripts/page_helper.js
fetch_asset javascripts/thm_numbering.js

# Regenerated diagrams (rewrites/ pipeline), if they have been built.
DIAGRAMS=""
if [[ -d "$ROOT/rewrites/out/svg" ]]; then
    DIAGRAMS="--diagrams $ROOT/rewrites/out/svg"
fi

# Typst-rendered pages (rewrites/bulk_pages.py --html), if built.
TYPST_PAGES=""
if [[ -d "$BUILD/typ-pages" ]]; then
    TYPST_PAGES="--typst-pages $BUILD/typ-pages"
fi

python3 "$ROOT/make_docset.py" \
    --html "$BUILD/nlab-content-html" \
    --source "$BUILD/nlab-content" \
    --assets "$BUILD/assets" \
    --out "$ROOT/dist" \
    $DIAGRAMS \
    $TYPST_PAGES

echo "Docset written to $ROOT/dist/nLab.docset"
