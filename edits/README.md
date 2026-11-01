# Source overlay edits

Pointwise corrections applied in-memory to mirror sources before the
typst pipeline runs (`nlab convert`), for typos and malformed markup
in upstream pages. One JSON file per page id:

```json
{
  "page": "24183",
  "note": "what is wrong on this page",
  "edits": [
    {"find": "exact source text", "replace": "corrected text",
     "why": "optional rationale"}
  ]
}
```

Each `find` must match the source exactly once; a zero-match edit is
reported as stale (usually meaning the wiki fixed it upstream — delete
the edit), an ambiguous one is skipped. Editing a file here re-emits
just that page on the next bulk run.
