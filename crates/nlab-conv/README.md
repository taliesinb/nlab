# nlab-conv

Converts nLab wiki source to typst. The nLab's pages are Instiki Markdown
(Maruku dialect) with itex math, whose diagrams come in several dialects;
this crate turns a whole page, or one formula, into typst, with diagrams as
native fletcher / cetz code and math through our mitex fork.

| module | converts |
|---|---|
| `prose` | whole pages: Markdown with Maruku quirks, theorem/proof environments, wiki links, images, footnotes |
| `emit` | itex formulas, including arrays drawn as commutative diagrams, to typst math / fletcher |
| `grid` | array diagrams into cell grids (for `emit`) |
| `tikzcd` | tikz-cd to fletcher |
| `tikzpic` | tikzpicture to cetz |
| `xy` | xymatrix to tikz-cd |

Wiki links and images resolve through a per-call `Context` (the page index
and the downloaded-files map, `build/page-index.tsv` and `build/files.tsv`)
installed with `with_context`; nothing is process-global, so a long-lived
caller converts pages from many threads at once.

The array-diagram parser and emitter are a port of the Python reference in
`../../rewrites/`, verified byte-identical across the full 8,959-diagram
corpus (`rewrites/diff_rust.py [--emit]`).

## Three faces

- **Python**: `nlab.conv` (wrapping the PyO3 module `nlab._conv`, built
  with the `python` feature by maturin when `uv run` syncs the project):
  `Converter(page_index, files_map).page(source, title)`, `formula`,
  `grid`, `Converter.tikzcd`, `canonical_file_url`. Calls release the GIL.
- **CLI**: `cargo build --release` gives `target/release/nlab-conv`, one
  conversion per process with the source on stdin: `page [TITLE]` (with
  `NLAB_PAGEIDX` / `NLAB_FILES_MAP` naming the TSVs), `grid`/`grids`
  (cell-grid JSON), `typsts` (formula emission, matching the `typst` DB
  table), `tikzcd(s)`, `dump` (mitex CST). The research scripts in
  `rewrites/` use it.
- **Rust**: the `nlab_conv` library.

## Local mitex (fork branch `nlab`)

The mitex fork carries fixes that make several `fix_tex` workarounds
unnecessary (branches: `fix/mathsf-upright`, `fix/xarrow-optional-labels`,
`feat/underoverset`, `feat/converter-api`; integration branch `nlab`).
It is pinned as the git submodule `vendor/mitex` (this crate links its
crates by path). `uv run nlab setup` builds its wasm plugin and installs it
as the local typst package `@local/mitex:0.2.7` (on Linux under
`${XDG_DATA_HOME:-~/.local/share}/typst/packages/local/`); by hand:

```sh
cd vendor/mitex    # from the repository root
cargo build --release --target wasm32-unknown-unknown \
    --manifest-path crates/mitex-wasm/Cargo.toml --features typst-plugin
PKG="$HOME/Library/Application Support/typst/packages/local/mitex/0.2.7"
rm -rf "$PKG" && mkdir -p "$PKG" && cp -r packages/mitex/* "$PKG/"
cp target/wasm32-unknown-unknown/release/mitex_wasm.wasm "$PKG/mitex.wasm"
```

To change the fork: work in `vendor/mitex` (a full clone; `git switch
nlab` first, as submodules check out a detached commit), commit and push
to the fork, then commit the new submodule pointer here (`git add
vendor/mitex`). Push the fork *before* sharing a pointer to a commit,
or `git submodule update` fails for everyone else.

Pages and tikz-cd always emit against `@local/mitex:0.2.7`; for formulas
it is opt-in (`formula(tex, local_mitex=True)`, or `NLAB_LOCAL_MITEX=1
nlab-conv typsts`), with the redundant workarounds disabled
(circled/set-operator unicode, `\mathscr`, `\mathsf`, plain
`\underoverset`, and the itex word-grouping regex — the fork's
`feat/itex-word-identifiers` implements itex's letter-run tokenization
natively, emitted as `mi-itex(...)`). On the 1,840 corpus formulas
using the affected commands: 96.5% compile in local mode vs 81.2% with
stock 0.2.6 + workarounds; a random 800-formula corpus sample compiles
at 98.2% in full local mode.
