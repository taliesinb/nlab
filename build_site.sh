#!/usr/bin/env bash
# Build the static HTML nLab site (dist/html) from scratch.
#
# Usage: ./build_site.sh [--refresh] [--jobs N] [--no-search] [--downloads FILE]
#   --refresh         re-download the nlab-content mirror (picks up new wiki
#                     content; every page is then re-converted)
#   --jobs N          parallel page conversions (default: CPU count)
#   --no-search       skip the Pagefind index (faster; search.html won't work)
#   --downloads FILE  download rules (default downloads/ncatlab.json: nLab
#                     images + PDFs; common.json adds arXiv etc. and takes
#                     days; everything.json adds any .pdf link) - the two
#                     broader presets are for private builds only
#
# Steps (each one is cached/resumable, so rerunning is cheap):
#   1. mitex fork   ../mitex @ nlab -> typst package @local/mitex:0.2.7
#   2. converter    crates/nlab-typ (links the same mitex checkout)
#   3. mirror       github.com/ncatlab/nlab-content -> build/nlab-content
#   4. page index   build/page-index.tsv (name/redirect -> id, for links)
#   5. downloads    referenced files per the rules -> build/files (content-
#                   addressed) + build/files.tsv; state in build/state.db
#   6. pages        build/typ-pages/<id>.{typ,html} via typst HTML export
#   7. site         dist/html/<slug>/index.html + assets + Pagefind index
#
# Requires: git curl tar python3 cargo (+ wasm32-unknown-unknown target)
# typst (0.15.x) svgo cwebp npx; Pillow is optional (dark-mode inversion
# of line-art images). See README.md "Building the static HTML site".
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
BUILD="$ROOT/build"
MITEX="$(cd "$ROOT/.." && pwd)/mitex"   # Cargo.toml path deps expect a sibling
MITEX_URL="https://github.com/taliesinb/mitex"
MITEX_BRANCH="nlab"
MITEX_KNOWN_GOOD="d13cf5fc3280f2f2ee32ddfb20e7a8b369f5a30d"  # last full-corpus build
TYPST_WANT="0.15"

REFRESH=0
NO_SEARCH=""
DOWNLOADS="$ROOT/downloads/ncatlab.json"
JOBS="$( (sysctl -n hw.ncpu || nproc) 2>/dev/null || echo 8)"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --refresh) REFRESH=1 ;;
        --jobs) JOBS="$2"; shift ;;
        --no-search) NO_SEARCH="--no-search" ;;
        --downloads) DOWNLOADS="$2"; shift ;;
        -h|--help) sed -n '2,28p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

step() { printf '\n==> %s\n' "$*"; }

# ---------------------------------------------------------------- tools
step "checking tools"
missing=()
for t in git curl tar python3 cargo typst svgo cwebp npx; do
    command -v "$t" >/dev/null || missing+=("$t")
done
if (( ${#missing[@]} )); then
    echo "missing: ${missing[*]}" >&2
    echo "macOS: brew install rustup typst svgo webp node && rustup-init -y" >&2
    exit 1
fi
if command -v rustup >/dev/null &&
        ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
    echo "adding rust target wasm32-unknown-unknown"
    rustup target add wasm32-unknown-unknown
fi
tv="$(typst --version | awk '{print $2}')"
if [[ "$tv" != "$TYPST_WANT"* ]]; then
    echo "warning: typst $tv; the pipeline was developed on $TYPST_WANT.x" >&2
fi
python3 -c 'import PIL' 2>/dev/null ||
    echo "note: Pillow not installed; line-art images won't invert in dark mode" \
         "(pip3 install pillow)"

# ----------------------------------------------------------- 1. mitex
step "mitex fork ($MITEX, branch $MITEX_BRANCH)"
if [[ ! -d "$MITEX/.git" ]]; then
    git clone -b "$MITEX_BRANCH" "$MITEX_URL" "$MITEX"
fi
branch="$(git -C "$MITEX" rev-parse --abbrev-ref HEAD)"
if [[ "$branch" != "$MITEX_BRANCH" ]]; then
    echo "warning: $MITEX is on branch '$branch', not '$MITEX_BRANCH'" >&2
fi
if [[ "$(git -C "$MITEX" rev-parse HEAD)" != "$MITEX_KNOWN_GOOD" ]]; then
    echo "note: mitex at $(git -C "$MITEX" rev-parse --short HEAD), not the" \
         "known-good ${MITEX_KNOWN_GOOD:0:7} (fine if you updated it on purpose)"
fi
case "$(uname)" in
    Darwin) PKG_ROOT="$HOME/Library/Application Support/typst/packages" ;;
    *) PKG_ROOT="${XDG_DATA_HOME:-$HOME/.local/share}/typst/packages" ;;
esac
PKG="$PKG_ROOT/local/mitex/0.2.7"
cargo build --release --target wasm32-unknown-unknown \
    --manifest-path "$MITEX/crates/mitex-wasm/Cargo.toml" --features typst-plugin
WASM="$(cd "$MITEX" && cargo metadata --format-version 1 --no-deps |
    python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
WASM="$WASM/wasm32-unknown-unknown/release/mitex_wasm.wasm"
rm -rf "$PKG" && mkdir -p "$PKG"
cp -R "$MITEX/packages/mitex/." "$PKG/"
cp "$WASM" "$PKG/mitex.wasm"
echo "installed @local/mitex:0.2.7 -> $PKG"

# ------------------------------------------------------- 2. converter
step "nlab-typ converter"
cargo build --release --manifest-path "$ROOT/crates/nlab-typ/Cargo.toml"

# ---------------------------------------------------------- 3. mirror
step "nlab-content mirror"
DEST="$BUILD/nlab-content"
if [[ -d "$DEST" && $REFRESH == 0 ]]; then
    echo "using cached $DEST (pass --refresh to re-download)"
else
    # download fully, then extract, then swap: a failed or interrupted
    # fetch leaves the previous mirror in place. tar -m stamps files with
    # the extraction time (GitHub archives carry the upstream commit time,
    # which can predate our last build and hide changed pages from
    # bulk_pages.py's mtime cache).
    rm -rf "$DEST.tmp" "$DEST.tar.gz"
    mkdir -p "$DEST.tmp"
    echo "downloading ncatlab/nlab-content (a few hundred MB) ..."
    curl -fL --retry 3 -o "$DEST.tar.gz" \
        "https://github.com/ncatlab/nlab-content/archive/refs/heads/master.tar.gz"
    tar xzmf "$DEST.tar.gz" -C "$DEST.tmp" --strip-components=1
    rm -f "$DEST.tar.gz"
    rm -rf "$DEST"
    mv "$DEST.tmp" "$DEST"
fi

# ------------------------------------------------- 4-7. pages and site
step "page index"
python3 "$ROOT/rewrites/page_index.py"

step "downloads ($(basename "$DOWNLOADS"); rate-limited and resumable, Ctrl-C is safe)"
python3 "$ROOT/rewrites/downloads.py" --config "$DOWNLOADS"

step "converting pages (--jobs $JOBS; ~10 min on a recent Mac for a full run)"
python3 "$ROOT/rewrites/bulk_pages.py" --html --jobs "$JOBS"
head -1 "$ROOT/rewrites/out/bulk-report.txt"

step "static site"
python3 "$ROOT/rewrites/html_site.py" $NO_SEARCH

echo
echo "Site written to $ROOT/dist/html"
echo "Preview: python3 -m http.server -d dist/html 8000  ->  http://localhost:8000/search.html"
