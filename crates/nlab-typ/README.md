# nlab-typ

Rust port of the diagram pipeline (see `../../rewrites/`), built on
[mitex-parser](https://github.com/taliesinb/mitex)'s CST. Verified
byte-identical to the Python reference across the full 8,959-diagram
corpus (`rewrites/diff_rust.py [--emit]`).

Modes: `dump` (CST), `grid`/`grids` (cell-grid JSON), `typsts`
(full typst emission, matching the `typst` DB table).

## Local mitex (fork branch `nlab`)

The mitex fork carries fixes that make several `fix_tex` workarounds
unnecessary (branches: `fix/mathsf-upright`, `fix/xarrow-optional-labels`,
`feat/underoverset`, `feat/converter-api`; integration branch `nlab`).
It is pinned as the git submodule `vendor/mitex` (this crate links its
crates by path). Build and install it as a local typst package
(`../../build_site.sh` does this automatically; on Linux the package
directory is `${XDG_DATA_HOME:-~/.local/share}/typst/packages/local/mitex/0.2.7`):

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

Then `NLAB_LOCAL_MITEX=1 nlab-typ typsts` emits against
`@local/mitex:0.2.7` with the redundant workarounds disabled
(circled/set-operator unicode, `\mathscr`, `\mathsf`, plain
`\underoverset`, and the itex word-grouping regex — the fork's
`feat/itex-word-identifiers` implements itex's letter-run tokenization
natively, emitted as `mi-itex(...)`). On the 1,840 corpus formulas
using the affected commands: 96.5% compile in local mode vs 81.2% with
stock 0.2.6 + workarounds; a random 800-formula corpus sample compiles
at 98.2% in full local mode.
