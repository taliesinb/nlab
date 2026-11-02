# nlab

A pipeline for turning the [nLab](https://ncatlab.org) — the wonderful 
collaborative wiki on category theory, higher category theory, homotopy 
theory, type theory, and mathematical physics — into a single 
[typst](https://typst.app/) corpus that compiles to a static, modern 
website with various quality-of-life improvements over the original ncatlab:
* higher-quality, more accurate, SVG-rendered commutative diagrams
* accurate, modern MathML-based equation rendering
* modern, dark-mode CSS styling
* client-side full-text search of the entire corpus

You can [serve](#serving-the-static-html-site-locally) or 
[build](#building-the-static-html-site) your own offline copy, or you can browse a recent build that [I host here](https://nlab.tali.link).

<p float="center">
<img width="320" alt="search" src="https://github.com/user-attachments/assets/ab94d42f-fe98-4f3b-bed6-7384cadd8393" />
<img width="320" alt="page-top" src="https://github.com/user-attachments/assets/15d2a5bd-1bcb-40fd-a157-5cf3b66ecde5" />
<img width="320" alt="diagram" src="https://github.com/user-attachments/assets/dcefe965-d117-4e74-8677-55190ca3db04" />
</p>

## Motivation

[nLab](https://ncatlab.org) is an extraordinary resource built by many 
very talented people. 

But like any long-runnning community project, it has accumulated tech debt 
that is up to the newer generation to pay off: it lacks full-text search, 
has an archaic build process, uses an ancient dialect of TeX, resorts to inline 
images for mathematical typesetting, and employs a hodge-podge of commutative 
diagram markup formats that either look ugly (low-res, no anti-aliasing), 
or misrender (HTML tables).  

Luckily, the age of LLMs makes rejuvenation possible. We can kill 
multiple birds with one stone by moving the entire corpus to the 
modern TeX-replacement known as [Typst](https://typst.app/).

This is my (draft) attempt to do this programmatically. 

## Development

The *raw code* for this project was almost entirely written by Claude 
(various models, culminating in Opus 5.5), but supervised extremely 
closely by me, (Taliesin Beynon)[https://tali.link]. The motivation was 
to make an entirely offline version of nlab not only easy to create, 
but *better* than the original.

This project involved careful *human* technical design at every stage of 
the project; as of 2026, LLMs are amazingly productive coders, but lack 
far-sighted strategic design sense, the ability to manage technical debt, 
and most importantly, any real sense of software engineering *taste*. 
They are, effectively, very, very good interns.

The majority of the human work came from many hours of painful QA in 
which I reviewed a large fraction of the converted commutative diagrams
for errors, iterating in a loop with Claude to fix them, understand 
*how* it was fixing them and manage the accumulation of technical debt, 
and to build a test suite.

Errors in conversation definitely do remain, please report them as issues on this
repo. I will add a "report issue" button to the build in the near future to streamline
this process.

## Usage

### CLI tool

The repo defines an `nlab` python package and CLI tool `uv run nlab ...` 
that drives all stages of this pipeline:

1. downloading of the current nLab mirror and static assets (like custom diagrams)
2. parsing and converting the embedded TeX dialect, and various forms of commutative diagram, into near-identical [typst](https://typst.app) equivalents, using a custom Rust crate for speed
3. rendering the resulting typst corpus to a static HTLM site, using MathML and SVG
4. de-aliasing the various original nLab pages
4. building a full-text search index
5. publishing this site via [Netlify](https://www.netlify.com/)
6. exporting the static site to an offline [Dash](https://kapeli.com/dash) docset

The Rust crate `crates/nlab-conv` (exposed to Python as `nlab.conv`) 
converts the wiki dialect (markdown+itex source) into [typst](https://typst.app), 
including tikzcd / xymatrix / tikzpicture / array diagrams. 

My [custom fork](https://github.com/taliesinb/mitex) of Mitex is used
to convert TeX to equivalent `typst` at compile time; the fork adds many missing 
features that were required to get all nlab's itex to convert accurately.

Typst's HTML export emits MathML math and inline-SVG diagrams, with in-site 
cross-links, dark mode, and client-side full-text search via 
[Pagefind](https://pagefind.app).

The nLab's content is fetched at build time from the official file-based 
mirrors that the nLab project itself publishes:

- [ncatlab/nlab-content-html](https://github.com/ncatlab/nlab-content-html) —
  every wiki page as rendered XHTML+MathML, laid out as
  `pages/<d>/<d>/<d>/<d>/<id>/{content.html,name,revision_id}`, where the
  shard path is the page id's decimal digits, least significant first
  (page 10004 lives at `pages/4/0/0/0/10004/`).
- [ncatlab/nlab-content](https://github.com/ncatlab/nlab-content) — the same
  pages as Markdown+itex source, used to harvest `[[!redirects ...]]`
  directives (search aliases) and `category:` tags (page classification).
- CSS/JS assets fetched from ncatlab.org (page styles, `thm_numbering.js` and
  its `prototype.js` dependency, `page_helper.js`).

### Serving the static HTML site locally

`uv run nlab serve` will serve the site on your localhost, building it from
scratch if necessary (see below).

### Building the static HTML site

```sh
git clone --recursive https://github.com/taliesinb/nlab && cd nlab
uv run nlab build-site             # everything -> dist/html
uv run nlab build-site --refresh   # re-download the wiki mirror first
uv run nlab build-site --downloads downloads/common.json   # also fetch arXiv etc.
```

`uv run` creates the environment, installs the Python dependencies and
compiles the converter (`nlab._conv`, a PyO3 extension built by maturin)
first; it rebuilds the extension by itself whenever the Rust changes. Every
step is cached, so reruns only redo what changed. To look at it:

```sh
uv run nlab serve-site             # serves dist/html at http://localhost:8000/
uv run nlab serve-site --port 9000 --rebuild   # build-site first, then serve
```

`serve-site` builds the site first if none exists yet (or with `--rebuild`;
other options, e.g. `--downloads`, are passed on to `build-site`), and serves
only if the build succeeds. `/` redirects to the search page, as on Netlify. 

Pages live at `dist/html/<slug>/` (lowercased, hyphenated page names), 
~20,000 of them, ~3 GB with the downloaded files and the search index.

### Building the Dash docset

[Dash](https://kapeli.com/dash) is a macOS documentation browser: it stores
"docsets" (self-contained bundles of HTML plus a SQLite search index) fully
offline and provides as-you-type fuzzy search across everything you have
installed. Docsets are an open format, so they work beyond Dash itself — in
particular [Bolt](https://github.com/BoltDocs), a free open-source docset
browser for iOS/iPadOS, can load the same bundle, giving you the entire nLab
on an iPhone or iPad with no network connection.

To build the dash docset, use:

```sh
uv run nlab build-docset             # fetches both mirrors (~1.5 GB unpacked
                                     # into build/) and the live site's CSS/JS
uv run nlab build-docset --refresh   # re-download them to pick up new content
uv run nlab build-docset --only "pullback,Yoneda lemma" --out dist-mini  # quick
```

This produces `dist/nLab.docset` (~530 MB). If `build/typ-pages` exists (from
`nlab build-site`), its typst-rendered pages replace the transformed mirror
pages (`--no-typst-pages` to opt out). Install it by double-clicking, or via
Dash → Settings → Docsets → `+` → Add Local Docset. Dash spends a few minutes
full-text-indexing on first install. Search with the `nlab:` keyword (e.g.
`nlab:yoneda`).

### Converting a single page to typst

To output the typst for a single page during development or testing, 
you can use the CLI as follows:

```sh
uv run nlab convert-page "Yoneda lemma"`
```

You can also provide a page ID or specify `-` to read from stdin.

This uses the same page index and downloads map the build uses. 

In Python, this takes the form:

```python
from nlab import conv
c = conv.Converter(page_index="build/page-index.tsv", files_map="build/files.tsv")
typ = c.page(source, title="Yoneda lemma")
```

### Requirements

macOS or Linux, network access (GitHub, ncatlab.org, PyPI, the typst package
registry), and:

| tool | used for | known-good version |
|---|---|---|
| [uv](https://docs.astral.sh/uv/) | the environment; Python 3.15 and the Python dependencies (Pillow, Pagefind, blake3) come from `pyproject.toml` | 0.13 |
| Rust via rustup | the converter and the mitex typst plugin; `rust-toolchain.toml` selects **stable** and the wasm target whatever your default is | rustc 1.99 |
| [typst](https://github.com/typst/typst) CLI | page compilation (HTML export) | **0.15.1** |
| `svgo` | minifying inline diagram SVGs | 4.1 |
| git | the `vendor/mitex` submodule | |

On macOS: `brew install uv rustup typst svgo && rustup-init -y`. The typst
packages `@preview/fletcher:0.5.8` and `@preview/cetz:0.3.4` download
automatically on first compile. `uv run nlab setup` checks all of this.

The typst HTML export is still experimental (`--features html`) and its
output changes between typst releases; other versions than 0.15.x may work but
are untested.

## Code layout

| path | what |
|---|---|
| `python/nlab/` | the `nlab` package: one module per stage, `cli.py` the entry point |
| `crates/nlab-conv/` | the converter (Rust); `nlab.conv` in Python, also a CLI binary `nlab-conv` |
| `vendor/mitex/` | our mitex fork (submodule): math conversion, and the `@local/mitex` typst package |
| `downloads/` | download rule presets |
| `edits/` | pointwise corrections to upstream page sources |
| `tests/` | `uv run pytest` |
| `rewrites/` | research tools from the diagram-rewrite work (galleries, comparisons); not part of the build |
| `build/`, `dist/` | everything generated (gitignored); shared state in `build/state.db` |

## `nlab` subcommands

The top-level subcommands that will invoke other subcommands automatically are:

1. [`uv run nlab serve-site`](#serving-the-static-html-site-locally)
2. [`uv run nlab build-site`](#building-the-static-html-site)
3. [`uv run nlab build-docset`](#building-the-dash-docset)
4. [`uv run nlab convert-page`](#converting-a-single-page-to-typst)

The individual subcommands you may want to invoke during active development are:

1. **`nlab setup`.** Checks the tools, initializes the `vendor/mitex`
   submodule if needed (our [mitex fork](https://github.com/taliesinb/mitex),
   branch `nlab`, carrying itex-compatibility fixes; the converter links its
   crates by path), builds its wasm plugin and installs it as the local typst
   package `@local/mitex:0.2.7` (see [crates/nlab-conv/README.md](crates/nlab-conv/README.md)).
2. **`nlab fetch`.** Downloads [ncatlab/nlab-content](https://github.com/ncatlab/nlab-content)
   (the Markdown+itex sources) to `build/nlab-content`.
3. **`nlab page-index`.** Writes `build/page-index.tsv` (every page name and
   `[[!redirects]]` alias → page id). Without it the converter cannot resolve
   wiki links and *silently* points every one of them at ncatlab.org.
4. **`nlab download`.** Fetches the files pages reference (see
   [Downloads](#downloads) below) into `build/files/` and writes
   `build/files.tsv`. Must run *before* step 5: the converter embeds only
   images listed there and turns the rest into links; step 5 re-converts
   exactly the pages whose images changed.
5. **`nlab convert --html`.** Converts each `content.md` (after applying the
   corrections in `edits/`) in-process with `nlab.conv` (worker threads in
   parallel), compiles it with typst's HTML export, and post-processes it into
   `build/typ-pages/<id>.html` (shared stylesheets, svgo-minified SVGs, WebP
   images). A cold full run takes about an hour on a 10-core M4, most of it
   one `svgo` process per distinct diagram (~53,000, cached in
   `build/svgmin`); the conversion itself takes under two minutes. Results
   are cached in `build/state.db`: each page is keyed on a hash of its source
   (after `edits/`), its title, how each of its `[[links]]` resolves in the
   page index, the `files.tsv` entries of its images, and the tools (the
   converter extension, mitex package, typst version, `convert.py`). So a
   rerun converts only pages whose inputs changed; a new redirect alias, say,
   re-converts just the pages whose links it changes. Failures are cached
   too; `--retry-failed` tries them again, `--force` ignores the cache, and
   `--pages ID,ID` converts specific pages. Interrupting is safe. Failures
   are listed in `build/convert-report.txt` and by `nlab status`.
6. **`nlab assemble-site`.** Lays out `dist/html/<slug>/index.html`,
   rewrites cross-links to slugs, points links to downloaded PDFs at the local
   copy (keeping the original as `data-original-url`), publishes exactly the
   files `build/files.tsv` lists, writes `search.html`, and runs Pagefind
   (configured by `pagefind.yml`). History subpages, placeholders and
   sandboxes are left out.

## Downloading assets

Pages reference ~65,000 distinct URLs. `nlab download` scans them all
into `build/state.db` (SQLite) on every run, decides per URL from a JSON rules
file what may be downloaded, and fetches the queue with a thread pool that is
rate-limited **per host**. Three presets in `downloads/`, each extending the
previous:

| preset | adds | size of the queue | use |
|---|---|---|---|
| `ncatlab.json` (default) | nLab-hosted uploads: images and PDFs | ~2,800 files | public site |
| `common.json` | arXiv (~15,500 papers; ~2.7 days at arXiv's 15 s crawl delay), open-access journals (TAC, NUMDAM, EMIS, CORE), web.archive.org; blocks publisher/paywall hosts | ~20,000 | private builds |
| `everything.json` | any direct `.pdf` link on any host (~7,200 more on ~2,200 hosts), 20 GB budget | ~27,000 | private builds |

Third-party papers are not ours to redistribute: deploy publicly only what
`ncatlab.json` fetches. Switching to a narrower preset is enough: the export,
and so the published site, only ever contains files the *current* config
allows, whatever was downloaded before.

Runs are resumable: Ctrl-C or `kill` stops cleanly, and a rerun continues
where it left off (nothing counts as done until it is on disk). Useful flags:
`--status` (counts per rule and status; no network), `--limit N`,
`--rule NAME` (only that rule's downloads), `--retry` (requeue
missing/refused/rejected/failed URLs), `--recheck` (revalidate downloaded
files with `If-None-Match` / `If-Modified-Since`).

**How files are stored.** As `build/files/<sha256[:32]>.<kind>`, where the kind
(png, jpg, gif, webp, svg, pdf) is sniffed from the bytes, never taken from
the URL or `Content-Type`. ncatlab.org answers a missing upload with `200`
and an HTML page, and some uploads have the wrong extension. Identical files
are stored once. `build/files.tsv` (canonical URL, store path, kind) is what
the converter and `nlab assemble-site` read; URLs are canonicalized by one
function, in Rust (`nlab.conv.canonical_file_url`).

**Rules.** A config file is an object with optional `extends` (a path or a
list of paths, relative to the file, loaded first), `global` and `rules`.
Rules are tried in order and the **first match wins**; a URL matching no rule
is not downloaded. Rules from extended files come first, so a broader preset
cannot undo a parent's `disable`. Unknown keys are errors. `--config` may be
given several times; the files are concatenated in order.

| rule key | meaning | default |
|---|---|---|
| `name` | required; shown in logs and stored in the database | |
| `host`, `path` | glob or list of globs, case-insensitive (`*.arxiv.org`, `/nlab/files/*`, `*.pdf`) | `*` |
| `disable` | never download matching URLs; also blocks redirects *into* them | `false` |
| `accept` | kinds to keep: `image`, `pdf`, or `png`/`jpg`/`gif`/`webp`/`svg`; anything else is rejected after the first 4 KB | `["image", "pdf"]` |
| `rewrite` | `[regex, replacement]` applied to the URL before fetching (e.g. arXiv `abs/` → `pdf/`) | none |
| `max_concurrent` | simultaneous downloads per host | 2 |
| `min_interval` | seconds between request starts per host (raised to the host's `robots.txt` `Crawl-delay`) | 1 |
| `robots` | obey `robots.txt` | `true` |
| `max_bytes` | per-file cap | 50 MB |
| `max_total_bytes` | budget for everything this rule downloads | none |
| `timeout` | seconds per request | 60 |
| `backoff_initial`, `backoff_max` | per-host back-off after 429/5xx/timeouts (doubling; `Retry-After` wins) | 60, 3600 |
| `note` | free text (JSON has no comments) | |

`global` keys: `max_concurrent` (downloads in flight across all hosts, 8),
`user_agent`, `max_attempts` (transient failures before a URL is `failed`, 4),
`max_wait` (a host backed off longer than this many seconds is left for the
next run, 300).

Per-host limits apply to whichever host is actually contacted: redirects are
followed by hand, re-checked against the rules, and rate-limited for their
own host. Back-off state and `robots.txt` (cached for a week) persist in the
database across runs.

## Deploying the site

- Netlify: `uv run nlab deploy-netlify` (production; `--draft` for a preview
  URL, `--dry-run` to print the command). `_redirects` sends `/` to
  `search.html`. The site ID is not in the repository: link the checkout once
  with `netlify link --id SITE_ID` (kept in the gitignored
  `.netlify/state.json`), or pass `--site` / set `NLAB_NETLIFY_SITE`.
- A symbol static host: `uv run nlab deploy-symbol --server URL` (or set
  `NLAB_SYMBOL_URL`); hash-diffed incremental uploads.
- Any plain static host works, as all links are relative; give it a root
  `index.html` that redirects to `search.html` (as `deploy-symbol` does).

`uv run nlab build-typst-site` emits the same pages as slug-named `.typ`
sources (`dist/typst`) for browsing typst directly.

## Caveats

- Uploaded files (images/PDFs under `/nlab/files/...`) are not part of the
  mirrors; those links point at the live site.
- The mirrors track the wiki with some lag; rerun with `--refresh` to update.
- A couple of oddities are upstream wiki content, not build artifacts (e.g.
  empty theorem cross-references on a few pages, reproducible on
  ncatlab.org).







