"""Offline builds of the nLab.

The pipeline, one module per stage (all driven by the `nlab` CLI):

  mirror      fetch the nlab-content mirror (and, for the docset, the
              rendered-HTML mirror and the live site's CSS/JS)
  page_index  build/page-index.tsv: page name / redirect -> page id
  downloads   files pages reference, per a JSON rules config, into the
              content-addressed store build/files (+ build/files.tsv)
  convert     every page -> typst (nlab.conv, in-process) -> HTML (typst)
  site        dist/html: the static site with Pagefind search
  typst_site  dist/typst: slug-named .typ sources
  docset      dist/nLab.docset for Dash
  deploy      deploy dist/html (Netlify, or a symbol static host)

State shared between stages lives in build/state.db (SQLite).
"""
