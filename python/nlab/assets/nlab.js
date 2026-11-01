// Floating page tools (static asset: edit + redeploy assets/nlab.js
// without rebuilding the page HTML).
(() => {
  const bar = document.createElement("div");
  bar.style.cssText =
    "position:fixed;top:10px;left:10px;z-index:50;display:flex;gap:6px;" +
    "opacity:0.25;transition:opacity 0.15s";
  bar.addEventListener("mouseenter", () => (bar.style.opacity = "1"));
  bar.addEventListener("mouseleave", () => (bar.style.opacity = "0.25"));

  function button(label, onclick) {
    const b = document.createElement("button");
    b.textContent = label;
    b.style.cssText =
      "font:0.75rem system-ui,sans-serif;color:var(--muted,#888);" +
      "background:var(--bg,#fff);border:1px solid var(--rule,#ccc);" +
      "border-radius:6px;padding:0.3em 0.7em;cursor:pointer";
    b.addEventListener("mouseenter", () => (b.style.color = "var(--fg,#222)"));
    b.addEventListener("mouseleave", () => (b.style.color = "var(--muted,#888)"));
    b.addEventListener("click", onclick);
    bar.appendChild(b);
    return b;
  }

  // a text snippet from the center of the viewport, punctuation-bounded,
  // for a #:~:text= fragment on the original page
  function centerSnippet() {
    const el = document.elementFromPoint(
      window.innerWidth / 2,
      window.innerHeight / 2
    );
    let node = el;
    while (node && (node.textContent || "").trim().length < 20) {
      node = node.parentElement;
    }
    if (!node) return null;
    const text = node.textContent.replace(/\s+/g, " ").trim();
    if (!text) return null;
    // first punctuation-free run of a few words
    const run = text.split(/[.,;:!?()[\]"\u{201c}\u{201d}\u{2026}|]/u)
      .map(s => s.trim())
      .find(s => s.split(/\s+/).length >= 3 && s.length >= 12);
    const pick = run || text.slice(0, 60);
    return pick.split(/\s+/).slice(0, 8).join(" ");
  }

  button("open original", () => {
    const name = document.title.trim();
    let url =
      "https://ncatlab.org/nlab/show/" +
      encodeURIComponent(name).replace(/%20/g, "+");
    const snip = centerSnippet();
    if (snip) url += "#:~:text=" + encodeURIComponent(snip);
    window.open(url, "_blank");
  });

  document.body.appendChild(bar);
})();
