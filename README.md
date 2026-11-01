# ncatlab-dash

A recipe for turning the [nLab](https://ncatlab.org) — the collaborative wiki
on category theory, higher category theory, homotopy theory, type theory, and
mathematical physics — into an offline, instantly-searchable **Dash docset**,
and (the main output nowadays) a fully independent **static HTML site**.

The static site re-renders every page from its Markdown+itex source through
[typst](https://typst.app) (`crates/nlab-typ` converts the wiki dialect,
including tikzcd / xymatrix / tikzpicture / array diagrams, to typst; typst's
HTML export emits MathML math and inline-SVG diagrams), with in-site
cross-links, dark mode, and client-side full-text search via
[Pagefind](https://pagefind.app). See [Building the static HTML
site](#building-the-static-html-site); the rest of this section describes the
docset.

[Dash](https://kapeli.com/dash) is a macOS documentation browser: it stores
"docsets" (self-contained bundles of HTML plus a SQLite search index) fully
offline and provides as-you-type fuzzy search across everything you have
installed. Docsets are an open format, so they work beyond Dash itself — in
particular [Bolt](https://github.com/BoltDocs), a free open-source docset
browser for iOS/iPadOS, can load the same bundle, giving you the entire nLab
on an iPhone or iPad with no network connection.

This repository contains **only the recipe**. The nLab's content is fetched at
build time from the official file-based mirrors that the nLab project itself
publishes:

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

## What the recipe adds

Beyond mechanically repackaging ~20,700 pages, `make_docset.py` does a fair
amount of curation:

- **Native math, no MathJax.** nLab pages ship MathML, which the WebKit view
  inside Dash renders natively. The nLab's own theorem/section numbering
  scripts are vendored so propositions read "Proposition 3.4." with proof
  tombstones, exactly like the live site.
- **Working offline hyperlinks.** Wiki links (`/nlab/show/<name>`) are
  resolved through a name→page-id map to relative links inside the docset;
  anything unresolvable (uploads, missing pages, wiki actions) points at the
  live site instead. Editing chrome (nav bars, search form, edit/history
  links) is stripped.
- **A typed index from the nLab's own ontology.** `category:` tags and
  page-name conventions classify pages as `Person` (~6,000 biography pages),
  `Resource` (books/papers), `Category` (floating tables of contents),
  `Guide` (expository series like *geometry of physics*), with concepts as
  `Entry`. Archived "… > history" subpages, blanked "empty NNN" placeholders,
  and Sandbox pages stay out of the index.
- **Wikipedia-style redirects.** Every nLab redirect stays searchable under
  its own name but displays as "alias → canonical name" (via Dash's
  `dash_entry_menuDescription` path metadata) and opens the canonical page —
  aliases are index rows, never duplicated pages.
- **Aggressive alias de-noising.** Of ~55,000 raw redirect names, more than
  half are spelling variants, and the build collapses them: case, dash-style
  (`--` vs en/em-dash), hyphen-vs-space, `∞` vs "infinity", plural
  inflections (toposes *and* topoi, spectra, simplices), escaped-apostrophe
  residue, people initials ("A. A. Markov Jr"), non-Latin-script names, and
  one-letter transliteration wobbles (Andrei/Andrej/Andrey). Genuinely
  different names survive ("Bill Lawvere", "Souslin", unaccented
  "Poincare"). Pages that declare combinatorial alias walls (>20 aliases)
  additionally drop names that only add one qualifier word to an
  already-indexed name.
- **Slim pages.** The per-page boilerplate the wiki ships 20,000× over —
  identical inline styles/scripts, the inlined logo SVG, hidden TeX
  `<annotation>` sources — is deduplicated into shared assets or dropped,
  cutting the docset from 730 MB to ~530 MB with pixel-identical rendering.
- **Dash niceties.** In-page table-of-contents anchors at every section
  heading, commutative diagrams scaled 1.5× for comfortable reading, docset
  icon derived from the nLab logo, and a fallback URL so entries can be
  opened on ncatlab.org.

The result: ~46,000 searchable names over 20,146 indexed pages.

## Building the static HTML site

```sh
./build_site.sh             # everything, from a clean checkout -> dist/html
./build_site.sh --refresh   # re-download the wiki mirror first
./build_site.sh --downloads downloads/common.json   # also fetch arXiv etc.
```

Every step is cached, so reruns only redo what changed. Preview with
`python3 -m http.server -d dist/html 8000` and open
<http://localhost:8000/search.html>. Pages live at `dist/html/<slug>/`
(lowercased, hyphenated page names), ~20,000 of them, ~1.2 GB with the search
index.

### Requirements

macOS or Linux, network access (GitHub, ncatlab.org, the typst package
registry, npm), and:

| tool | used for | known-good version |
|---|---|---|
| Rust toolchain + `wasm32-unknown-unknown` target | `nlab-typ`, the mitex typst plugin | rustc 1.94 |
| [typst](https://github.com/typst/typst) CLI | page compilation (HTML export) | **0.15.1** |
| `svgo` | minifying inline diagram SVGs | 4.0.2 |
| `cwebp` (libwebp) | re-encoding uploaded images | 1.6.0 |
| Node.js / `npx` | Pagefind search index (fetched by `npx`) | node 26, pagefind 1.5.2 |
| git, curl, tar, Python 3 | everything else | Python 3.15 |
| Pillow (optional) | line-art detection so diagrams invert in dark mode | |

On macOS: `brew install rustup typst svgo webp node && rustup-init -y &&
rustup target add wasm32-unknown-unknown` (and optionally
`pip3 install pillow`). typst packages `@preview/fletcher:0.5.8` and
`@preview/cetz:0.3.4` download automatically on first compile.

The typst HTML export is still experimental (`--features html`) and its
output changes between typst releases; other versions than 0.15.x may work but
are untested.

### What `build_site.sh` does

In order (each step can also be run by hand):

1. **mitex fork.** Math goes through a fork of
   [mitex](https://github.com/taliesinb/mitex) (branch `nlab`, carrying
   itex-compatibility fixes), checked out as a *sibling of this repository*
   (`../mitex`, which `crates/nlab-typ/Cargo.toml` links by path). It is cloned
   if absent, its wasm plugin is built, and the package is installed as the
   local typst package `@local/mitex:0.2.7` (see
   [crates/nlab-typ/README.md](crates/nlab-typ/README.md)).
2. **Converter.** `cargo build --release --manifest-path
   crates/nlab-typ/Cargo.toml`.
3. **Mirror.** Downloads [ncatlab/nlab-content](https://github.com/ncatlab/nlab-content)
   (the Markdown+itex sources) to `build/nlab-content`. The HTML mirror
   `build.sh` also fetches is only needed for the docset.
4. **Page index.** `rewrites/page_index.py` writes `build/page-index.tsv`
   (every page name and `[[!redirects]]` alias → page id). Without it the
   converter cannot resolve wiki links and *silently* points every one of them
   at ncatlab.org.
5. **Downloads.** `rewrites/downloads.py --config downloads/ncatlab.json`
   fetches the files pages reference (see [Downloads](#downloads) below) into
   `build/files/` and writes `build/files.tsv`. Must run *before* step 6: the
   converter embeds only images listed there and turns the rest into links;
   step 6 re-converts exactly the pages whose images changed.
6. **Pages.** `rewrites/bulk_pages.py --html --jobs N` converts each
   `content.md` (after applying the corrections in `edits/`) with `nlab-typ`,
   compiles it with typst's HTML export, and post-processes it into
   `build/typ-pages/<id>.html` (shared stylesheets, svgo-minified SVGs, webp
   images). Results are cached in `build/state.db`: each page is keyed on a
   hash of its source (after `edits/`), its title, how each of its
   `[[links]]` resolves in the page index, the `files.tsv` entries of its
   images, and the tools (converter binary, mitex package, typst version,
   this script). So a rerun converts only pages whose inputs changed; a new
   redirect alias, say, re-converts just the pages whose links it changes.
   Failures are cached too; `--retry-failed` tries them again, `--force`
   ignores the cache, and `--pages ID,ID` converts specific pages.
   Interrupting is safe. Failures are listed in `rewrites/out/bulk-report.txt`; on the
   August 2026 mirror the run was 20,723/20,723 clean, but newer wiki content
   may introduce new failures (failed pages are simply absent from the site).
   `nlab-typ` itself also invokes `typst query` (with the mitex package) to
   measure tikzcd node sizes, falling back to estimates if that fails.
7. **Site.** `rewrites/html_site.py` lays out `dist/html/<slug>/index.html`,
   rewrites cross-links to slugs, points links to downloaded PDFs at the local
   copy (keeping the original as `data-original-url`), publishes exactly the
   files `build/files.tsv` lists, writes `search.html`, and runs Pagefind
   (configured by `pagefind.yml`). History subpages, placeholders and
   sandboxes are left out.

To work on a single page: `NLAB_PAGEIDX=build/page-index.tsv
NLAB_FILES_MAP=build/files.tsv crates/nlab-typ/target/release/nlab-typ page
"<page name>" < build/nlab-content/pages/…/<id>/content.md > page.typ` — the
source is read from **stdin** (a path argument silently yields an empty page).

### Downloads

Pages reference ~65,000 distinct URLs. `rewrites/downloads.py` scans them all
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
`nlab-typ` (`NLAB_FILES_MAP`) and `html_site.py` read.

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

### Deploying

- Netlify: `netlify deploy --prod --dir dist/html` (`_redirects` sends `/` to
  `search.html`).
- Any plain static host works, as all links are relative; give it a root
  `index.html` that redirects to `search.html` (as
  `rewrites/symbol_deploy.py` does for its host).

`rewrites/typst_site.py` emits the same pages as slug-named `.typ` sources
(`dist/typst`) for browsing typst directly.

## Building the Dash docset

Requirements: macOS or Linux with `curl`, `tar`, and Python 3 (stdlib only).

```sh
./build.sh              # first run downloads the mirrors (~150 MB archives,
                        # ~1.5 GB unpacked into build/), then packages
./build.sh --refresh    # re-download mirrors and assets to pick up new content
```

This produces `dist/nLab.docset` (~530 MB). If `build/typ-pages` exists (step 6
above), its typst-rendered pages replace the transformed mirror pages. Install it by double-clicking, or
via Dash → Settings → Docsets → `+` → Add Local Docset. Dash spends a few
minutes full-text-indexing on first install. Search with the `nlab:` keyword
(e.g. `nlab:yoneda`).

For iterating on the pipeline there is a fast path that builds a small docset
from named pages only:

```sh
python3 make_docset.py \
    --html build/nlab-content-html --source build/nlab-content \
    --assets build/assets --out dist-mini \
    --only "pullback,Yoneda lemma,adjoint functor"
```

## Caveats

- Uploaded files (images/PDFs under `/nlab/files/...`) are not part of the
  mirrors; those links point at the live site.
- The mirrors track the wiki with some lag; rerun with `--refresh` to update.
- A couple of oddities are upstream wiki content, not build artifacts (e.g.
  empty theorem cross-references on a few pages, reproducible on
  ncatlab.org).
