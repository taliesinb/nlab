//! Instiki (nLab-flavored Markdown) -> typst markup.
//!
//! Order matters: itex math is extracted before Markdown parsing (its
//! underscores would read as emphasis), along with wiki links, includes,
//! and the `+--{: .class}` / `=--` environment fences, all stashed
//! behind private-use-area placeholders; pulldown-cmark handles the
//! remaining standard Markdown; a final pass resolves placeholders into
//! typst (math through mi-itex / the diagram emitters, links to
//! ncatlab.org, fences into nlab-env blocks).

use crate::emit;
use crate::tikzcd;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

const P0: char = '\u{E000}'; // placeholder delimiters
const P1: char = '\u{E001}';

pub(crate) struct Stash {
    items: Vec<String>, // resolved typst fragments
    srcs: Vec<String>,  // original source, restored inside code contexts
}

impl Stash {
    fn put(&mut self, typst: String) -> String {
        self.items.push(typst);
        self.srcs.push(String::new());
        format!("{}{}{}", P0, self.items.len() - 1, P1)
    }
    fn put_src(&mut self, typst: String, src: String) -> String {
        self.items.push(typst);
        self.srcs.push(src);
        format!("{}{}{}", P0, self.items.len() - 1, P1)
    }
    /// Placeholders back to their source text (code spans/blocks).
    fn unstash(&self, text: &str) -> String {
        let re = regex::Regex::new(&format!("{}(\\d+){}", P0, P1)).unwrap();
        re.replace_all(text, |c: &regex::Captures| {
            let i: usize = c[1].parse().unwrap();
            if self.srcs[i].is_empty() {
                self.items[i].clone()
            } else {
                self.srcs[i].clone()
            }
        })
        .to_string()
    }
}

// ------------------------------------------------------------ pre-pass

/// Pull `\label{Id}` out of a math body; returns (tex, typst anchors).
fn split_math_labels(tex: &str) -> (String, String) {
    let re = regex::Regex::new(r"\\label\{([\w:.-]+)\}").unwrap();
    let mut anchors = String::new();
    let tex = re
        .replace_all(tex, |c: &regex::Captures| {
            anchors.push_str(&format!("#metadata(none)#label(\"{}\")", &c[1]));
            String::new()
        })
        .to_string();
    (tex, anchors)
}

fn extract_math(src: &str, stash: &mut Stash) -> String {
    // \[ ... \] display math first (chars, so the $-scanner never sees it)
    let bracket_re = regex::Regex::new(r"(?s)\\\[(.*?)\\\]").unwrap();
    let src = bracket_re
        .replace_all(src, |c: &regex::Captures| {
            // authors also use \[..\] as ESCAPED LITERAL brackets, most
            // often around reference links: leave those to markdown
            let body = &c[1];
            if body.contains("<a ") || body.contains("http") || body.contains("](") {
                return c[0].to_string();
            }
            // an unclosed \[ (author typo) can swallow env fences and
            // paragraphs; leave those spans alone
            if body.contains("=--") || body.contains("+--") || body.len() > 4000 {
                return c[0].to_string();
            }
            let (tex, anchors) = split_math_labels(body);
            stash.put(format!("{}{}", math_to_typst(&tex, true), anchors))
        })
        .to_string();
    let mut out = String::new();
    let b: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == '$' {
            let display = i + 1 < b.len() && b[i + 1] == '$';
            let open = if display { 2 } else { 1 };
            // find closing delimiter
            let mut j = i + open;
            let mut found = None;
            while j < b.len() {
                if b[j] == '$' && b[j - 1] != '\\' {
                    if display {
                        if j + 1 < b.len() && b[j + 1] == '$' {
                            found = Some(j);
                            break;
                        }
                    } else {
                        found = Some(j);
                        break;
                    }
                }
                j += 1;
            }
            if let Some(j) = found {
                let tex: String = b[i + open..j].iter().collect();
                // an unclosed $$ (author typo) pairs with the NEXT
                // block's opener and swallows prose and env fences;
                // emit the whole span literally (delimiters included)
                // so pairing parity downstream is unchanged
                if tex.contains("=--") || tex.contains("+--") || tex.len() > 4000 {
                    for &ch in &b[i..j + open] {
                        out.push(ch);
                    }
                    i = j + open;
                    continue;
                }
                let delim = if display { "$$" } else { "$" };
                let src_frag = format!("{}{}{}", delim, tex, delim);
                let (tex, anchors) = split_math_labels(&tex);
                out.push_str(&stash.put_src(
                    format!("{}{}", math_to_typst(&tex, display), anchors),
                    src_frag,
                ));
                i = j + open;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

fn math_to_typst(tex: &str, display: bool) -> String {
    // \begin{svg}[[!include SVG rightarrow]]\end{svg} idiom: an inline
    // arrow image used as a stackrel base
    let tex = regex::Regex::new(
        r"(?s)\\begin\{svg\}.*?\\end\{svg\}(?:\\includegraphics\[[^\]]*\]\{[^}]*\})?",
    )
    .unwrap()
    .replace_all(tex, |c: &regex::Captures| {
        if c[0].contains("left") {
            "\\longleftarrow"
        } else if c[0].contains("vert") {
            "\\downarrow"
        } else {
            "\\longrightarrow"
        }
        .to_string()
    })
    .to_string();
    let tex = tex.as_str();
    // numeric character references inside math ($C &#8696; D$)
    let tex = regex::Regex::new(r"&#(\d+);")
        .unwrap()
        .replace_all(tex, |c: &regex::Captures| {
            c[1].parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .map(String::from)
                .unwrap_or_else(|| c[0].to_string())
        })
        .to_string();
    let tex = &emit::fix_itex_builtins(&tex);
    if display {
        if let Some(body) = emit::emit_formula_body(tex) {
            return format!("#nlab-diagram[\n{}]", body.trim_end());
        }
        // plain display math (arrays inside become matrices)
        // emit_equation already includes the leading `#`
        emit::emit_equation_pub(tex).trim_end().to_string()
    } else {
        format!("#mi({})", emit::ts(tex))
    }
}

/// Inline `<img>`/`<figure>` HTML referencing the nLab file store.
/// Images already present in the local cache (NLAB_FILES_ROOT, default
/// build/nlab-files next to the content mirror) embed as #figure/#image
/// with a `files/<name>` path; otherwise they degrade to a link.
fn extract_images(src: &str, stash: &mut Stash) -> String {
    // presentation wrappers around figures would make pulldown-cmark
    // swallow the whole block (with our placeholder) as raw HTML
    let src = regex::Regex::new(r"(?m)^\s*</?(center|div)[^>]*>\s*$")
        .unwrap()
        .replace_all(src, "")
        .to_string();
    let src = src.as_str();
    let fig_re = regex::Regex::new(
        r#"(?s)<figure[^>]*>\s*.*?<img[^>]*src="([^"]+)"[^>]*>.*?(?:<figcaption[^>]*>(.*?)</figcaption>)?\s*</figure>"#,
    )
    .unwrap();
    let src = fig_re
        .replace_all(src, |c: &regex::Captures| {
            let cap = c.get(2).map(|m| m.as_str()).unwrap_or("");
            stash.put(image_typst(&c[1], cap))
        })
        .to_string();
    let img_re = regex::Regex::new(r#"<img[^>]*src="([^"]+)"[^>]*/?>"#).unwrap();
    img_re
        .replace_all(&src, |c: &regex::Captures| stash.put(image_typst(&c[1], "")))
        .to_string()
}

fn files_root() -> std::path::PathBuf {
    std::env::var("NLAB_FILES_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("build/nlab-files"))
}

/// Escape caption text for typst, but keep `[text](url)` links live.
fn caption_typst(text: &str) -> String {
    let link_re = regex::Regex::new(r"\[([^\[\]]+)\]\((https?://[^)\s]+)\)").unwrap();
    let mut out = String::new();
    let mut pos = 0usize;
    for m in link_re.captures_iter(text) {
        let whole = m.get(0).unwrap();
        out.push_str(&escape_typst(&text[pos..whole.start()]));
        out.push_str(&format!(
            "#link(\"{}\")[{}];",
            &m[2],
            escape_typst(&m[1])
        ));
        pos = whole.end();
    }
    out.push_str(&escape_typst(&text[pos..]));
    out
}

fn image_typst(url: &str, caption_html: &str) -> String {
    let caption = caption_typst(
        &regex::Regex::new(r"<[^>]+>")
            .unwrap()
            .replace_all(caption_html, "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    );
    // typst's #figure supplies its own "Figure N:" prefix
    let caption = regex::Regex::new(r"^Figure \d+\s*[:.]\s*")
        .unwrap()
        .replace(&caption, "")
        .to_string();
    let name = url.rsplit('/').next().unwrap_or(url);
    let cached = url.contains("/nlab/files/") && files_root().join(name).exists();
    if cached {
        let img = format!("nlab-image(\"files/{}\")", name);
        if caption.is_empty() {
            format!("#{}", img)
        } else {
            format!("#figure({}, caption: [{}])", img, caption)
        }
    } else {
        let text = if caption.is_empty() {
            "(figure)".to_string()
        } else {
            caption
        };
        format!("#link(\"{}\")[{}];", url, text)
    }
}

/// Page-name -> id lookup for in-docset links, loaded from the TSV
/// named by NLAB_PAGEIDX (name or redirect, tab, page id). Lookups are
/// case-insensitive like the wiki's.
fn page_index() -> &'static std::collections::HashMap<String, String> {
    static IDX: std::sync::OnceLock<std::collections::HashMap<String, String>> =
        std::sync::OnceLock::new();
    IDX.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        if let Ok(path) = std::env::var("NLAB_PAGEIDX") {
            if let Ok(txt) = std::fs::read_to_string(path) {
                for line in txt.lines() {
                    if let Some((name, id)) = line.split_once('\t') {
                        m.entry(name.trim().to_lowercase())
                            .or_insert_with(|| id.trim().to_string());
                    }
                }
            }
        }
        m
    })
}

fn wiki_target_url(name: &str) -> String {
    let mut enc = String::new();
    for c in name.trim().chars() {
        match c {
            ' ' => enc.push('+'),
            c if c.is_ascii_alphanumeric() || "+-_.".contains(c) => enc.push(c),
            c => {
                let mut buf = [0u8; 4];
                for byte in c.encode_utf8(&mut buf).bytes() {
                    enc.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    format!("https://ncatlab.org/nlab/show/{}", enc)
}

fn extract_wiki(src: &str, stash: &mut Stash) -> String {
    // [text]([[Name:file]]) — a file wiki-link in destination position
    // must become a bare URL, not a stashed #link call
    let src = regex::Regex::new(r"\]\(\[\[([^\[\]|]+?):file\]\]\)")
        .unwrap()
        .replace_all(src, |c: &regex::Captures| {
            format!(
                "](https://ncatlab.org/nlab/files/{})",
                c[1].trim().replace(' ', "+")
            )
        })
        .to_string();
    // a wiki-link inside a URL destination (`](http://..#[[X]])`) is
    // just part of the URL text; drop the brackets
    let src = regex::Regex::new(r"\]\(([^()\s\[\]]+)\[\[([^\[\]]+)\]\]\)")
        .unwrap()
        .replace_all(&src, |c: &regex::Captures| {
            format!("]({}{})", &c[1], c[2].replace(' ', "%20"))
        })
        .to_string();
    let re = regex::Regex::new(r"\[\[([^\[\]]+)\]\]").unwrap();
    re.replace_all(&src, |c: &regex::Captures| {
        let inner = &c[1];
        let whole = c[0].to_string();
        if let Some(rest) = inner.strip_prefix('!') {
            // directives: skip includes/redirects in the page body
            let _ = rest;
            return String::new();
        }
        let (target, text) = match inner.split_once('|') {
            Some((t, x)) => (t, x),
            None => (inner, inner),
        };
        // [[Name.pdf:file]] links to an uploaded file, not a wiki page
        if let Some(file) = target.strip_suffix(":file") {
            let text = text.strip_suffix(":file").unwrap_or(text);
            return stash.put_src(format!(
                "#link(\"https://ncatlab.org/nlab/files/{}\")[{}];",
                file.trim().replace(' ', "+"),
                escape_typst(text)
            ), whole.clone());
        }
        if let Some(id) = page_index().get(&target.trim().to_lowercase()) {
            return stash.put_src(format!(
                "#nlab-page(\"{}\")[{}];",
                id,
                escape_typst(text)
            ), whole.clone());
        }
        stash.put_src(format!(
            "#link(\"{}\")[{}];",
            wiki_target_url(target),
            escape_typst(text)
        ), whole.clone())
    })
    .to_string()
}

/// `+-- {: .num_defn #id}` ... `=--` fences (nestable) into env markers.
fn extract_fences(src: &str, stash: &mut Stash) -> String {
    let open_re =
        regex::Regex::new(r"^\+--\s*\{:\s*([^}]*)\}\s*$").unwrap();
    let mut out: Vec<String> = Vec::new();
    let mut skip_depth = 0usize; // inside a dropped block (.rightHandSide)
    let mut open_stack: Vec<bool> = Vec::new(); // true = emitted env
    for line in src.lines() {
        let t = line.trim();
        if let Some(c) = open_re.captures(t) {
            let attrs = c[1].to_string();
            if skip_depth > 0 {
                skip_depth += 1;
                continue;
            }
            if attrs.contains(".rightHandSide") || attrs.contains(".toc") {
                skip_depth = 1;
                continue;
            }
            let id = attrs
                .split_whitespace()
                .find(|w| w.starts_with('#'))
                .map(|w| w[1..].to_string());
            let class = attrs
                .split_whitespace()
                .find(|w| w.starts_with('.'))
                .map(|w| w[1..].to_string())
                .unwrap_or_default();
            let (kind, style) = env_kind(&class);
            let id_arg = id
                .map(|i| format!(", id: \"{}\"", i))
                .unwrap_or_default();
            // blank lines around the markers: an unterminated raw-HTML
            // block would otherwise swallow the fence placeholder
            out.push(String::new());
            out.push(stash.put(format!("#nlab-env(\"{}\", \"{}\"{})[", kind, style, id_arg)));
            open_stack.push(true);
            continue;
        }
        if t.starts_with("=--") && t[1..].chars().all(|c| c == '-') {
            if skip_depth > 0 {
                skip_depth -= 1;
                continue;
            }
            if open_stack.pop().unwrap_or(false) {
                out.push(String::new());
                out.push(stash.put("]".to_string()));
            }
            continue;
        }
        if skip_depth > 0 {
            continue;
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

fn env_kind(class: &str) -> (&'static str, &'static str) {
    match class {
        "num_defn" | "un_defn" => ("Definition", "num"),
        "num_theorem" | "un_theorem" => ("Theorem", "num"),
        "num_lemma" | "un_lemma" => ("Lemma", "num"),
        "num_prop" | "un_prop" => ("Proposition", "num"),
        "num_cor" | "un_cor" => ("Corollary", "num"),
        "num_remark" | "un_remark" => ("Remark", "num"),
        "num_example" | "un_example" => ("Example", "num"),
        "num_note" | "un_note" => ("Note", "num"),
        "proof" => ("Proof", "proof"),
        _ => ("", "plain"),
    }
}

// -------------------------------------------------------- markdown pass

/// Repair `](url)` spans whose URL contains spaces or unbalanced
/// parens (both make CommonMark treat the link as literal text).
fn fix_md_link_urls(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < b.len() {
        let is_open = b[i] == ']'
            && b.get(i + 1) == Some(&'(')
            && b[i + 2..].iter().take(4).collect::<String>() == "http";
        if !is_open {
            out.push(b[i]);
            i += 1;
            continue;
        }
        // walk the URL: balanced close wins; otherwise the last ')'
        // before end-of-line
        let start = i + 2;
        let mut depth = 1i32;
        let mut end = None;
        let mut last_close = None;
        let mut j = start;
        // authors wrap long URLs across a line; walk past a single
        // newline but never a blank line or far afield
        while j < b.len() && j - start < 400 {
            if b[j] == '\n'
                && (j + 1 >= b.len() || b[j + 1] == '\n')
            {
                break;
            }
            match b[j] {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    last_close = Some(j);
                    if depth == 0 {
                        end = Some(j);
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let (end, balanced) = match (end, last_close) {
            (Some(e), _) => (e, true),
            (None, Some(e)) => (e, false),
            (None, None) => {
                out.push(b[i]);
                i += 1;
                continue;
            }
        };
        let url: String = b[start..end].iter().collect();
        // a line break inside the URL is wrapping, not content
        let mut fixed = url.replace('\n', "").replace(' ', "%20");
        if !balanced {
            fixed = fixed.replace('(', "%28").replace(')', "%29");
        }
        out.push_str("](");
        out.push_str(&fixed);
        out.push(')');
        i = end + 1;
    }
    out
}

/// Typst label identifiers: keep the safe charset, dash the rest.
fn sanitize_id(id: &str) -> String {
    id.trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || ":_.-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn escape_typst(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '#' | '*' | '_' | '`' | '$' | '[' | ']' | '<' | '>' | '@' | '\\' | '\'' | '"' => {
                out.push('\\');
                out.push(c);
            }
            // "//" starts a typst comment even in markup ("/*" is
            // already broken up by the '*' escape above)
            '/' if out.ends_with('/') => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

fn heading_eq(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 1,
        HeadingLevel::H3 => 2,
        HeadingLevel::H4 => 3,
        HeadingLevel::H5 => 4,
        HeadingLevel::H6 => 5,
    }
}

fn markdown_to_typst(src: &str, stash: &Stash) -> String {
    let parser = Parser::new_ext(src, Options::ENABLE_TABLES);
    let mut out = String::new();
    let mut skip_heading = false; // h6 titles inside envs are dropped
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut item_depth = 0usize;
    let mut link_semi: Vec<bool> = Vec::new();
    let mut in_code = false;
    for ev in parser {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                if level == HeadingLevel::H6 {
                    skip_heading = true;
                } else {
                    out.push_str(&format!("\n{} ", "=".repeat(heading_eq(level))));
                }
            }
            Event::End(TagEnd::Heading(level)) => {
                if level == HeadingLevel::H6 {
                    skip_heading = false;
                } else {
                    out.push('\n');
                }
            }
            Event::Start(Tag::Table(ref aligns)) => {
                out.push_str(&format!(
                    "\n#table(columns: {}, stroke: 0.5pt + luma(50%), inset: 5pt,\n",
                    aligns.len()
                ));
            }
            Event::End(TagEnd::Table) => out.push_str(")\n\n"),
            Event::Start(Tag::TableHead) => out.push_str("table.header("),
            Event::End(TagEnd::TableHead) => out.push_str("),\n"),
            Event::Start(Tag::TableRow) => {}
            Event::End(TagEnd::TableRow) => out.push('\n'),
            Event::Start(Tag::TableCell) => out.push('['),
            Event::End(TagEnd::TableCell) => out.push_str("],"),
            Event::Start(Tag::Paragraph) => {
                // paragraphs inside a list item stay in the item
                if item_depth == 0 {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if item_depth == 0 {
                    out.push('\n');
                }
            }
            // function forms: the marker forms (*bold*) require word
            // boundaries and fall apart in text like `**4**4 (2011)`
            Event::Start(Tag::Emphasis) => out.push_str("#emph["),
            Event::End(TagEnd::Emphasis) => out.push_str("];"),
            Event::Start(Tag::Strong) => out.push_str("#strong["),
            Event::End(TagEnd::Strong) => out.push_str("];"),
            Event::Start(Tag::List(start)) => list_stack.push(start),
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
                out.push('\n');
            }
            Event::Start(Tag::Item) => {
                item_depth += 1;
                let marker = if matches!(list_stack.last(), Some(Some(_))) {
                    "+"
                } else {
                    "-"
                };
                out.push_str(&format!("\n{} ", marker));
            }
            Event::End(TagEnd::Item) => {
                item_depth = item_depth.saturating_sub(1);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                // inside a list item the quote must stay in the item
                // (a column-0 block would terminate the typst list)
                if item_depth > 0 {
                    out.push_str(" #quote(block: true)[");
                } else {
                    out.push_str("\n#quote(block: true)[");
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                if item_depth > 0 {
                    out.push_str("] ");
                } else {
                    out.push_str("]\n");
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                // the trailing `;` (pushed at End) terminates the hash
                // expression, so following ".Word" or "(..)" is text
                let dest = dest_url.trim_matches(|c| c == '{' || c == '}');
                if dest.is_empty() {
                    out.push_str("#[");
                    link_semi.push(false);
                } else if let Some(anchor) = dest.strip_prefix('#') {
                    // authors sometimes write `](#{#Id})` or other junk
                    let anchor = anchor
                        .trim_matches(|c: char| c == '{' || c == '}' || c == '#');
                    out.push_str(&format!(
                        "#nlab-anchor(\"{}\")[",
                        sanitize_id(anchor)
                    ));
                    link_semi.push(true);
                } else {
                    let dest = dest.replace('\\', "%5C").replace('"', "%22");
                    out.push_str(&format!("#link(\"{}\")[", dest));
                    link_semi.push(true);
                }
            }
            Event::End(TagEnd::Link) => {
                out.push(']');
                if link_semi.pop().unwrap_or(false) {
                    out.push(';');
                }
            }
            Event::Start(Tag::CodeBlock(_)) => {
                in_code = true;
                out.push_str("\n```\n");
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                out.push_str("\n```\n");
            }
            Event::Code(c) => {
                out.push_str(&format!("`{}`", stash.unstash(&c)));
            }
            Event::Text(t) => {
                if in_code {
                    // fence content is verbatim: no typst escapes, and
                    // placeholders revert to their source text
                    out.push_str(&stash.unstash(&t));
                } else if !skip_heading {
                    out.push_str(&escape_typst(&t));
                }
            }
            Event::SoftBreak => out.push(' '),
            Event::HardBreak => out.push_str(" \\\n"),
            Event::Rule => out.push_str("\n#line(length: 100%)\n"),
            _ => {}
        }
    }
    out
}

// ------------------------------------------------------------ assembly

const PAGE_PREAMBLE: &str = r##"#import "@local/mitex:0.2.7": mi-itex, mitex-itex, mitex-scope
#import "@preview/fletcher:0.5.8": diagram, node, edge
#import "@preview/cetz:0.3.4"
// natively-emitted math calls the same helpers the mitex plugin's eval
// scope provides; bind the ones the converter can reference
#let mitexref(body) = [(#body)]
#let mitexdisplay = mitex-scope.at("mitexdisplay")
#let mitexinline = mitex-scope.at("mitexinline")
#let mitexmathbf = mitex-scope.at("mitexmathbf")
#let mitexupright = mitex-scope.at("mitexupright")
#let mitexitalic = mitex-scope.at("mitexitalic")
#let mitexsans = mitex-scope.at("mitexsans")
#let mitexfrak = mitex-scope.at("mitexfrak")
#let mitexmono = mitex-scope.at("mitexmono")
#let mitexsqrt = mitex-scope.at("mitexsqrt")
#let mitexset = mitex-scope.at("mitexset")
#let mitexnot = mitex-scope.at("mitexnot")
#let mitexscript = mitex-scope.at("mitexscript")
#let mitexsscript = mitex-scope.at("mitexsscript")
#let mitexarray = mitex-scope.at("mitexarray")
#let mitexlabel = mitex-scope.at("mitexlabel")
#let mitexoverbrace = mitex-scope.at("mitexoverbrace")
#let mitexunderbrace = mitex-scope.at("mitexunderbrace")
#let mitexoverbracket = mitex-scope.at("mitexoverbracket")
#let mitexunderbracket = mitex-scope.at("mitexunderbracket")
#let mitexcomment = mitex-scope.at("mitexcomment")
#let scope-or(name, fallback) = if name in mitex-scope { mitex-scope.at(name) } else { fallback }
#let xarrow = scope-or("xrightarrow", scope-or("xarrow", none))
#let xleftarrow = scope-or("xleftarrow", xarrow)
#let xrightarrow = scope-or("xrightarrow", xarrow)
#let xLeftarrow = scope-or("xLeftarrow", xarrow)
#let xRightarrow = scope-or("xRightarrow", xarrow)
#let xleftrightarrow = scope-or("xleftrightarrow", xarrow)
#let xLeftrightarrow = scope-or("xLeftrightarrow", xarrow)
#let xhookleftarrow = scope-or("xhookleftarrow", xarrow)
#let xhookrightarrow = scope-or("xhookrightarrow", xarrow)
#let xtwoheadleftarrow = scope-or("xtwoheadleftarrow", xarrow)
#let xtwoheadrightarrow = scope-or("xtwoheadrightarrow", xarrow)
#let xleftharpoonup = scope-or("xleftharpoonup", xarrow)
#let xrightharpoonup = scope-or("xrightharpoonup", xarrow)
#let xleftharpoondown = scope-or("xleftharpoondown", xarrow)
#let xrightharpoondown = scope-or("xrightharpoondown", xarrow)
#let xleftrightharpoons = scope-or("xleftrightharpoons", xarrow)
#let xrightleftharpoons = scope-or("xrightleftharpoons", xarrow)
#let xtofrom = scope-or("xtofrom", xarrow)
#let xmapsto = scope-or("xmapsto", xarrow)
#let xlongequal = scope-or("xlongequal", xarrow)
#let operatorname = scope-or("operatorname", none)
#let operatornamewithlimits = scope-or("operatornamewithlimits", none)
#let one-arg-ok(f) = (..args) => {
  let p = args.pos()
  if p.len() == 1 { p.at(0) } else { f(..args) }
}
#let underset = one-arg-ok(scope-or("underset", none))
#let overset = one-arg-ok(scope-or("overset", none))
#let underoverset = scope-or("underoverset", none)
#let stackrel = one-arg-ok(scope-or("stackrel", none))
#let textmath = scope-or("textmath", none)
#let big = scope-or("big", none)
#let Big = scope-or("Big", none)
#let bigg = scope-or("bigg", none)
#let Bigg = scope-or("Bigg", none)
#let aligned = scope-or("aligned", none)
#let matrix = scope-or("matrix", none)
#let pmatrix = scope-or("pmatrix", none)
#let bmatrix = scope-or("bmatrix", none)
#let Bmatrix = scope-or("Bmatrix", none)
#let vmatrix = scope-or("vmatrix", none)
#let Vmatrix = scope-or("Vmatrix", none)
#let smallmatrix = scope-or("smallmatrix", none)
#let substack = scope-or("substack", none)
#let cases = scope-or("cases", std.math.cases)
#let atop = scope-or("atop", none)
#let negthinspace = scope-or("negthinspace", none)
#let mitexcite = scope-or("mitexcite", none)
#let mitexcolor = scope-or("mitexcolor", none)
// LaTeX size switches are ABSOLUTE: \tiny inside a script-size
// attachment still gives the same glyph size as in body text, so these
// bind to fixed pt (body 10.5pt), overriding mitex's em-relative ones
#let mitexsizetiny = it => text(size: 6.8pt, math.display(it))
#let mitexsizescript = it => text(size: 7.9pt, math.display(it))
#let mitexsizefoot = it => text(size: 8.4pt, math.display(it))
#let mitexsizesmall = it => text(size: 9.5pt, math.display(it))
#let mitexsizelarge = it => text(size: 12.6pt, math.display(it))
#let mitexsizeLarge = it => text(size: 15.1pt, math.display(it))
#let mitexsizeLARGE = it => text(size: 18.2pt, math.display(it))
#let mitexsizehuge = it => text(size: 21.7pt, math.display(it))
#let mitexsizeHuge = it => text(size: 26.1pt, math.display(it))
#let mitexsim = scope-or("mitexsim", math.scripts(sym.tilde.op))
#let mitexsimeq = scope-or("mitexsimeq", math.scripts(sym.tilde.eq))
#let mitexbacksimeq = scope-or("mitexbacksimeq", math.scripts(sym.tilde.eq.rev))
#let mitexcong = scope-or("mitexcong", math.scripts(sym.tilde.equiv))
#let mitexapprox = scope-or("mitexapprox", math.scripts(sym.approx))
#let mitexequiv = scope-or("mitexequiv", math.scripts(sym.equiv))
#let phantom = scope-or("phantom", none)
#let mathop = scope-or("mathop", none)
#let Set = scope-or("Set", none)
#let mathclap = scope-or("mathclap", none)
#let rlap = scope-or("rlap", none)
#let llap = scope-or("llap", none)
#let tfrac = scope-or("tfrac", none)
#let dfrac = scope-or("dfrac", none)
#let mathsf = scope-or("mathsf", none)
#let mathscr = scope-or("mathscr", none)
#let mathrm = scope-or("mathrm", none)
#let mathit = scope-or("mathit", none)
#let scalebox = scope-or("scalebox", (f, it) => it)
#let rotatebox = scope-or("rotatebox", (deg, it) => it)
#let raisebox = scope-or("raisebox", (d, it) => it)
#let boxed = scope-or("boxed", it => box(stroke: 0.5pt, inset: 2.5pt, $it$))
// explicit size: a box inside an equation inside a fletcher node has
// its inset-derived height collapsed, cropping ascenders/descenders
#let fbox = it => context { let sz = measure(it); box(stroke: 0.5pt, width: sz.width + 12pt, height: sz.height + 10pt, align(center + horizon, it)) }
#let textrm = scope-or("textrm", it => math.upright(it))
#let textup = scope-or("textup", it => math.upright(it))
#let textit = scope-or("textit", it => math.italic(it))
#let mitexcolorbox = scope-or("mitexcolorbox", (c, it) => it)
#let textbf = scope-or("textbf", it => math.bold(it))
#let mitexbold = scope-or("mitexbold", it => math.bold(it))
#let mitexcal = scope-or("mitexcal", it => math.cal(it))
#let mathrel = scope-or("mathrel", it => math.class("relation", it))
#let mathbin = scope-or("mathbin", it => math.class("binary", it))
#let pmod = scope-or("pmod", it => $(#math.op("mod") #it)$)
#let lvert = scope-or("lvert", sym.bar.v)
#let rvert = scope-or("rvert", sym.bar.v)
#let argmin = scope-or("argmin", math.op("arg min", limits: true))
#let argmax = scope-or("argmax", math.op("arg max", limits: true))
#let arraystretch = none
// itex commands that take an argument but also appear bare: tolerate
// the zero-argument call the converter then emits
#let argless(name, bare) = {
  let f = scope-or(name, none)
  (..args) => if args.pos().len() == 0 { bare } else { f(..args) }
}
#let Set = argless("Set", math.upright("Set"))
#let mitexnot = argless("mitexnot", sym.slash)
#let hat = (..args) => if args.pos().len() == 0 { sym.hat } else { std.math.hat(..args) }
#let double = argless("double", none)
#set page(width: 17cm, height: 25cm, margin: 1.6cm, fill: white, numbering: "1")
#set text(10.5pt)
#set heading(numbering: (..n) => context {
  if target() == "html" {
    html.elem("span", attrs: (class: "secnum"), numbering("1.", ..n))
  } else {
    numbering("1.", ..n)
  }
})
#set list(indent: 1.2em)
#set enum(indent: 1.2em)
#show link: set text(fill: rgb("#1a6318"))
#show figure.caption: set text(0.85em)
// images embed as base64 data URIs under HTML export; reference the
// shared file store instead so the docset can dedupe and re-encode
#let nlab-image(path) = context {
  if target() == "html" {
    html.elem(
      "div",
      attrs: (class: "nlab-diagram"),
      html.elem("img", attrs: (src: path, style: "max-width: 74%")),
    )
  } else {
    align(center, image(path, width: 74%))
  }
}
// diagrams (fletcher/cetz) draw to frames, which the HTML export
// would silently drop: freeze them as inline SVG there
#let nlab-diagram(body) = context {
  // inline equations under-report tall ink (frac, stretched lr) via
  // font line metrics; bounds edges make node boxes hug the glyphs
  let b = block(breakable: false, {
    set text(top-edge: "bounds", bottom-edge: "bounds")
    body
  })
  if target() == "html" {
    html.elem("div", attrs: (class: "nlab-diagram"), html.frame(b))
  } else {
    align(center, b)
  }
}
// in-docset cross-page link; retarget at compile time via
//   typst compile --input nlab-base=... --input nlab-ext=.html
#let nlab-page-base = sys.inputs.at("nlab-base", default: "")
#let nlab-page-ext = sys.inputs.at("nlab-ext", default: ".html")
#let nlab-page(id, body) = link(nlab-page-base + id + nlab-page-ext, body)
#let nlab-count = counter("nlab-env")
#show heading.where(level: 1): it => { nlab-count.update(0); it }
#let nlab-title(body) = context {
  if target() == "html" {
    html.elem("h1", body)
  } else {
    align(center, text(1.7em, weight: 700, body))
    v(0.5em)
  }
}
#let nlab-env(kind, style, id: none, body) = context {
  let head = if style == "num" {
    nlab-count.step()
    context {
      let num = str(counter(heading).get().at(0, default: 0)) + "." + str(nlab-count.get().first())
      if id != none { [#metadata(num)#label(id)] }
      strong[#kind #num.]
    }
  } else if style == "proof" {
    emph[Proof.]
  } else { none }
  if target() == "html" {
    let qed = if style == "proof" {
      html.elem("span", attrs: (class: "qed"), $qed$)
    } else { none }
    html.elem(
      "div",
      attrs: (class: "nlab-env nlab-" + style + " nlab-kind-" + lower(kind)),
      [#head #body#qed],
    )
  } else if style == "num" or style == "proof" {
    let tail = if style == "proof" { [#h(1fr) $qed$] } else { none }
    block(inset: (left: 1.5em), above: 1em, below: 1em)[#head #body#tail]
  } else {
    block(above: 1em, below: 1em, body)
  }
}
#let nlab-cite(id) = context {
  if query(label(id)).len() > 0 { [\[#link(label(id))[#id]\]] } else { [\[#id\]] }
}
#let nlab-ref(id) = context {
  let ms = query(label(id))
  if ms.len() > 0 and ms.first().func() == metadata and ms.first().value != none {
    link(label(id), ms.first().value)
  } else if ms.len() > 0 {
    link(label(id))[(above)]
  } else {
    [(ref)]
  }
}
// itex the converter could not parse: show the source instead of
// failing the page
#let nlab-tex-fallback(src) = box(
  fill: luma(96%),
  inset: (x: 3pt, y: 2pt),
  radius: 2pt,
  raw(src),
)
// framed/painted text patch with real margins: inset-only boxes
// collapse around equations, so measure and size explicitly
#let nlab-tbox(strokev, fillv, it) = {
  set text(top-edge: "bounds", bottom-edge: "bounds")
  context {
    let sz = measure(it)
    box(stroke: strokev, fill: fillv, width: sz.width + 12pt, height: sz.height + 9pt, align(center + horizon, it))
  }
}
// xy \overset{\mathclap{lab}}{base}: the label takes no room; arrows
// aim at base. Float lab just past base's edge, measured with bounds
// edges (inline equations otherwise under-report tall ink).
#let nlab-satellite(base, lab, over) = {
  set text(top-edge: "bounds", bottom-edge: "bounds")
  context {
    let h = measure(lab).height
    if over {
      block(base + place(top + center, dy: -h - 3pt, lab))
    } else {
      block(base + place(bottom + center, dy: h + 3pt, lab))
    }
  }
}
#let nlab-anchor(id, body) = context {
  if id.len() == 0 or query(label(id)).len() == 0 { body } else { link(label(id), body) }
}
"##;

pub(crate) fn page_to_typst(src: &str, title: Option<&str>) -> String {
    let mut stash = Stash { items: Vec::new(), srcs: Vec::new() };
    // windows-1252 mojibake control chars can't encode in HTML
    let src: String = src
        .chars()
        .filter(|c| !('\u{80}'..='\u{9F}').contains(c))
        .collect();
    let src = src.as_str();
    let src = src.to_string();
    // Instiki <nowiki> spans are fully verbatim
    let src = regex::Regex::new(r"(?s)<nowiki>(.*?)</nowiki>")
        .unwrap()
        .replace_all(&src, |c: &regex::Captures| {
            stash.put_src(format!("#raw({:?})", &c[1]), c[1].to_string())
        })
        .to_string();
    // code spans and fences are verbatim: stash them BEFORE the wiki /
    // math extractors, which would otherwise convert `[[x]]` or `$y$`
    // written inside backticks as markup examples
    let src = regex::Regex::new(r"(?ms)^```([A-Za-z]*)[ \t]*\n(.*?)^```[ \t]*$")
        .unwrap()
        .replace_all(&src, |c: &regex::Captures| {
            let lang = if c[1].is_empty() {
                String::new()
            } else {
                format!(", lang: \"{}\"", &c[1])
            };
            stash.put_src(
                format!(
                    "#block(fill: luma(96%), inset: 8pt, radius: 4pt, width: 100%)[#raw({:?}{})]",
                    c[2].trim_end(),
                    lang
                ),
                c[0].to_string(),
            )
        })
        .to_string();
    let src = regex::Regex::new(r"`([^`\n]+)`")
        .unwrap()
        .replace_all(&src, |c: &regex::Captures| {
            stash.put_src(format!("#raw({:?})", &c[1]), c[0].to_string())
        })
        .to_string();
    let src = src.as_str();
    // strip trailing metadata lines
    let src: String = src
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.starts_with("category:") && !t.starts_with("[[!redirects")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let src = src.replace("\\tableofcontents", &stash.put("#outline(depth: 2)".into()));
    // xymatrix drawings translate to tikzcd vocabulary and ride the
    // same fletcher pipeline
    let xy_re = regex::Regex::new(
        r"(?s)\\begin\{xymatrix([^}]*)\}(.*?)\\end\{xymatrix[^}]*\}",
    )
    .unwrap();
    let src = xy_re
        .replace_all(&src, |c: &regex::Captures| {
            // documentation quoting xy syntax in code spans is not a
            // diagram
            if c[2].contains("<nowiki>") || c[2].contains('`') {
                return c[0].to_string();
            }
            // spacing riders: @C=50pt, @R12pt, @C+2pc, @=18pt ...
            let mut seps: Vec<String> = Vec::new();
            let rider = regex::Regex::new(
                r"@([CR]?)\s*(?:(=)\s*)?([+-])?\s*([0-9.]+)\s*(pt|pc|mm|cm|em|ex)",
            )
            .unwrap();
            for m in rider.captures_iter(c.get(1).map(|m| m.as_str()).unwrap_or("")) {
                let v: f64 = m[4].parse().unwrap_or(0.0);
                let unit = match &m[5] {
                    "pc" => 12.0,
                    "mm" => 2.845,
                    "cm" => 28.45,
                    "em" => 11.0,
                    "ex" => 4.4,
                    _ => 1.0,
                };
                let mut pt = v * unit;
                // @C+2pc adds to the default; @C=-5pt just sets
                if m.get(2).is_none() {
                    match m.get(3).map(|x| x.as_str()) {
                        Some("+") => pt += 24.0,
                        Some("-") => pt = (24.0 - pt).max(4.0),
                        _ => {}
                    }
                } else if m.get(3).map(|x| x.as_str()) == Some("-") {
                    pt = -pt;
                }
                match &m[1] {
                    "C" => seps.push(format!("column sep={}pt", pt)),
                    "R" => seps.push(format!("row sep={}pt", pt)),
                    _ => seps.push(format!("sep={}pt", pt)),
                }
            }
            // xy grids default to equal 2pc separations both ways
            if !seps.iter().any(|s| s.starts_with("column") || s.starts_with("sep")) {
                seps.push("column sep=40pt".into());
            }
            if !seps.iter().any(|s| s.starts_with("row") || s.starts_with("sep")) {
                seps.push("row sep=38pt".into());
            }
            seps.push("xymode".into());
            let opt = format!("[{}]", seps.join(", "));
            format!(
                "\\begin{{tikzcd}}{}{}\\end{{tikzcd}}",
                opt,
                crate::xy::xymatrix_to_tikzcd(&c[2])
            )
        })
        .to_string();
    // raw tikzcd blocks (the live site renders these server-side)
    let tikz_re =
        regex::Regex::new(r"(?s)\\begin\{tikzcd\}.*?\\end\{tikzcd\}").unwrap();
    let src = tikz_re
        .replace_all(&src, |c: &regex::Captures| {
            match tikzcd::tikzcd_to_fletcher(&c[0]) {
                Ok((code, _)) => stash.put(format!(
                    "#nlab-diagram[\n{}]",
                    code.trim_end()
                )),
                Err(_) => c[0].to_string(),
            }
        })
        .to_string();
    // raw LaTeX-style theorem environments in prose (as opposed to the
    // +-- fences), optionally carrying a \label{Id}
    const ENVS: &str = "proposition|theorem|lemma|corollary|definition|\
example|examples|remark|note|proof|prop|prpn|thm|lem|cor|defn|rmk|exmp|\
terminology|notn|notation|conjecture|assumption|exercise";
    // \begin{center}..\end{center} wrappers are transparent for us
    let src = regex::Regex::new(r"\\(begin|end)\{(?:center|centre)\}")
        .unwrap()
        .replace_all(&src, "")
        .to_string();
    // author typos pairing \[ with \end{equation} (or the reverse):
    // convert only the spans whose NEAREST terminator is the wrong one
    let src = {
        let mut out = String::new();
        let mut rest = src.as_str();
        loop {
            let i_br = rest.find("\\[");
            let i_eq = rest.find("\\begin{equation}");
            let (opener, i, olen) = match (i_br, i_eq) {
                (Some(a), Some(b)) if a < b => ("\\[", a, 2),
                (Some(a), None) => ("\\[", a, 2),
                (_, Some(b)) => ("\\begin{equation}", b, 16),
                (None, None) => break,
            };
            let after = &rest[i + olen..];
            let want = if opener == "\\[" { "\\]" } else { "\\end{equation}" };
            let wrong = if opener == "\\[" { "\\end{equation}" } else { "\\]" };
            let c_ok = after.find(want);
            let c_bad = after.find(wrong);
            let convert = match (c_ok, c_bad) {
                (Some(a), Some(b)) => b < a,
                (None, Some(_)) => true,
                _ => false,
            };
            if convert {
                let e = c_bad.unwrap();
                let body = &after[..e];
                if !(body.contains("=--") || body.contains("+--") || body.len() > 4000) {
                    out.push_str(&rest[..i]);
                    let (tex, anchors) = split_math_labels(body);
                    out.push_str(&stash.put(format!(
                        "{}{}",
                        math_to_typst(&tex, true),
                        anchors
                    )));
                    rest = &after[e + wrong.len()..];
                    continue;
                }
            }
            out.push_str(&rest[..i + olen]);
            rest = after;
        }
        out.push_str(rest);
        out
    };
    // \begin{equation} bodies are display math the earlier extractors
    // do not cover
    let eq_re = regex::Regex::new(
        r"(?s)\\begin\{equation\*?\}(.*?)\\end\{equation\*?\}",
    )
    .unwrap();
    let src = eq_re
        .replace_all(&src, |c: &regex::Captures| {
            let (tex, anchors) = split_math_labels(&c[1]);
            stash.put(format!("{}{}", math_to_typst(&tex, true), anchors))
        })
        .to_string();
    // standalone tikzpicture drawings go through the CeTZ converter
    let pic_re = regex::Regex::new(
        r"(?s)\\begin\{tikzpicture\}(\[[^\]]*\])?(.*?)\\end\{tikzpicture\}",
    )
    .unwrap();
    let src = pic_re
        .replace_all(&src, |c: &regex::Captures| {
            let opts = c.get(1).map(|m| m.as_str()).unwrap_or("");
            match crate::tikzpic::to_cetz(&c[2], opts) {
                Some((code, _)) => {
                    stash.put(format!("#nlab-diagram[#{}]", code))
                }
                // outside the vocabulary: show the source as code
                // rather than letting tikz markup leak into prose
                None => stash.put(format!(
                    "#block(fill: luma(96%), inset: 8pt, radius: 4pt)[#raw({:?}, lang: \"latex\")]",
                    c[0].to_string()
                )),
            }
        })
        .to_string();
    // \begin{imagefromfile} carries a JSON-ish body naming an uploaded
    // file; render like any other stored image
    let img_env_re = regex::Regex::new(
        r"(?s)\\begin\{imagefromfile\}(.*?)\\end\{imagefromfile\}",
    )
    .unwrap();
    let src = img_env_re
        .replace_all(&src, |c: &regex::Captures| {
            let body = &c[1];
            let name = regex::Regex::new(r#""file_name"\s*:\s*"([^"]+)""#)
                .unwrap()
                .captures(body)
                .map(|m| m[1].to_string());
            match name {
                Some(n) if files_root().join(&n).exists() => stash.put_src(
                    format!("#nlab-image(\"files/{}\")", n),
                    c[0].to_string(),
                ),
                Some(n) => stash.put_src(
                    format!(
                        "#link(\"https://ncatlab.org/nlab/files/{}\")[(figure)];",
                        n.replace(' ', "+")
                    ),
                    c[0].to_string(),
                ),
                None => String::new(),
            }
        })
        .to_string();
    // one sequential pass so a stray \end (author error) is the one
    // dropped, not the document's final legitimate close
    let env_re = regex::Regex::new(&format!(
        r"\\(begin|end)\{{({})\}}([ \t]*\[([^\]\n]*)\])?(\s*\\label\{{([\w:.-]+)\}})?",
        ENVS
    ))
    .unwrap();
    let mut env_depth = 0i32;
    let src = env_re
        .replace_all(&src, |c: &regex::Captures| {
            if &c[1] == "end" {
                return if env_depth > 0 {
                    env_depth -= 1;
                    stash.put("]".into())
                } else {
                    String::new()
                };
            }
            env_depth += 1;
            let (kind, style) = match &c[2] {
                "proof" => ("Proof", "proof"),
                "proposition" | "prop" | "prpn" => ("Proposition", "num"),
                "theorem" | "thm" => ("Theorem", "num"),
                "lemma" | "lem" => ("Lemma", "num"),
                "corollary" | "cor" => ("Corollary", "num"),
                "definition" | "defn" => ("Definition", "num"),
                "example" | "examples" | "exmp" => ("Example", "num"),
                "remark" | "rmk" => ("Remark", "num"),
                "terminology" => ("Terminology", "num"),
                "notn" | "notation" => ("Notation", "num"),
                "conjecture" => ("Conjecture", "num"),
                "assumption" => ("Assumption", "num"),
                "exercise" => ("Exercise", "num"),
                _ => ("Note", "num"),
            };
            let id_arg = c
                .get(6)
                .map(|m| format!(", id: \"{}\"", m.as_str()))
                .unwrap_or_default();
            let title = c
                .get(4)
                .map(|m| m.as_str().trim())
                .filter(|t| !t.is_empty())
                .map(|t| format!("#strong[({})] ", escape_typst(t)))
                .unwrap_or_default();
            stash.put(format!(
                "#nlab-env(\"{}\", \"{}\"{})[{}",
                kind, style, id_arg, title
            ))
        })
        .to_string();
    // any still-unclosed environments get their closes at document end
    let mut src = src;
    for _ in 0..env_depth.max(0) {
        src = format!("{}\n{}", src, stash.put("]".into()));
    }
    let src = src.replace("\\linebreak", " ");
    // a "# Contents" heading right before the toc directive would
    // double #outline()'s own title (and appear inside the outline)
    let src = regex::Regex::new(
        r"(?mi)^#+\s*Contents?\s*#*\s*\n+(\*[^\n{]*\n?\{:\s*toc)",
    )
    .unwrap()
    .replace_all(&src, "$1")
    .to_string();
    // maruku table-of-contents list item ("* table of contents\n{:toc}",
    // "* automatic toc {: toc}", ...)
    let src = regex::Regex::new(r"(?m)^\*[^\n{]*\n?\{:\s*toc[^}]*\}\s*$")
        .unwrap()
        .replace_all(&src, |_: &regex::Captures| {
            stash.put("#outline(depth: 2)".into())
        })
        .to_string();
    // closed ATX headings without spaces (`#Contents#`) aren't CommonMark
    let src = regex::Regex::new(r"(?m)^(#+)\s*(.*?)\s*#+\s*$")
        .unwrap()
        .replace_all(&src, "$1 $2")
        .to_string();
    // an unclosed ** inside a table cell (author typo) leaves literal
    // stars in the rendered cell; drop the lone marker
    let src = src
        .lines()
        .map(|l| {
            if !l.trim_start().starts_with('|') {
                return l.to_string();
            }
            l.split('|')
                .map(|seg| {
                    if seg.matches("**").count() % 2 == 1 {
                        seg.replacen("**", "", 1)
                    } else {
                        seg.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect::<Vec<_>>()
        .join("\n");
    // a non-breaking space after ATX hashes is not a heading to cmark
    let src = regex::Regex::new(r"(?m)^(#+)\u{A0}+")
        .unwrap()
        .replace_all(&src, "$1 ")
        .to_string();
    // Maruku also accepts headings with NO space after the hashes
    // ("##Idea"); cmark needs one
    let src = regex::Regex::new(r"(?m)^(#{1,6})([^#\s])")
        .unwrap()
        .replace_all(&src, "$1 $2")
        .to_string();
    // Maruku attribute-definition lines `{:name: attrs}`; expand their
    // references `{: name}` in fences to the stored attributes
    let mut ial_defs: Vec<(String, String)> = Vec::new();
    let src = regex::Regex::new(r"(?m)^\{:(\w+):([^}\n]*)\}\s*$")
        .unwrap()
        .replace_all(&src, |c: &regex::Captures| {
            ial_defs.push((c[1].to_string(), c[2].trim().to_string()));
            String::new()
        })
        .to_string();
    let mut src = src;
    for (name, attrs) in &ial_defs {
        src = regex::Regex::new(&format!(r"\{{:\s*{}\s*\}}", regex::escape(name)))
            .unwrap()
            .replace_all(&src, format!("{{: {}}}", attrs).as_str())
            .to_string();
    }
    // anchor ids containing spaces, in definition and reference position
    let src = regex::Regex::new(r"\{#([\w:-]+(?: [\w:-]+)+)\}")
        .unwrap()
        .replace_all(&src, |c: &regex::Captures| {
            format!("{{#{}}}", c[1].replace(' ', "-"))
        })
        .to_string();
    let src = regex::Regex::new(r"\]\(#([\w:-]+(?: [\w:-]+)+)\)")
        .unwrap()
        .replace_all(&src, |c: &regex::Captures| {
            format!("](#{})", c[1].replace(' ', "-"))
        })
        .to_string();
    // authors occasionally write LaTeX sectioning instead of markdown
    let src = regex::Regex::new(
        r"(?m)^\\(sub)?(sub)?section\*?\{([^}\n]*)\}[ \t]*$",
    )
    .unwrap()
    .replace_all(&src, |c: &regex::Captures| {
        let depth = 2
            + c.get(1).map_or(0, |_| 1)
            + c.get(2).map_or(0, |_| 1);
        format!("{} {}", "#".repeat(depth), &c[3])
    })
    .to_string();
    // Maruku tolerates a space between link text and destination
    // (`[text] (http://..)`); cmark does not
    let src = regex::Regex::new(r"\]\s+\((https?://[^()\s]+|#[^()\s]+)\)")
        .unwrap()
        .replace_all(&src, "]($1)")
        .to_string();
    // `&lt;url>` — an angle autolink whose < was entity-escaped
    let src = regex::Regex::new(r"&lt;(https?://[^\s>&]+)>")
        .unwrap()
        .replace_all(&src, "<$1>")
        .to_string();
    // `[text] [url]` — square-bracket destination
    let src = regex::Regex::new(r"\]\s*\[(https?://[^\s\]]+)\]")
        .unwrap()
        .replace_all(&src, "]($1)")
        .to_string();
    // inline inkscape/svg-edit drawings: cmark's inline-HTML handling
    // leaks their metadata as text; drop them wholesale
    let src = regex::Regex::new(r"(?s)<svg\b.*?</svg>")
        .unwrap()
        .replace_all(&src, "")
        .to_string();
    // an author who swapped text and destination ([url](description))
    let src = regex::Regex::new(r"\[(https?://[^\s\]]+)\]\(([^()]*[ ][^()]*)\)")
        .unwrap()
        .replace_all(&src, "[$2]($1)")
        .to_string();
    // Instiki autolinks bare URLs; cmark needs <..>. Preceding context
    // must not be a markdown destination `](`, an attribute `="`, or
    // an already-wrapped `<`
    let src = regex::Regex::new(r#"(^|[\s:]|[^\]<\[]\()(https?://[^\s<>"']+)"#)
        .unwrap()
        .replace_all(&src, |c: &regex::Captures| {
            let mut url = &c[2];
            // trailing punctuation belongs to the prose, and a closing
            // paren only to the URL if it has a matching opener inside
            loop {
                let t = url.trim_end_matches(['.', ',', ';', '!', '?']);
                let t = if t.ends_with(')')
                    && t.matches('(').count() < t.matches(')').count()
                {
                    &t[..t.len() - 1]
                } else {
                    t
                };
                if t.len() == url.len() {
                    break;
                }
                url = t;
            }
            let tail = &c[2][url.len()..];
            format!("{}<{}>{}", &c[1], url, tail)
        })
        .to_string();
    // inline <a> tags become markdown links BEFORE math extraction:
    // anchors inside \text{} would otherwise reach typst raw and
    // parse as unclosed <labels>
    let src = regex::Regex::new(r#"(?s)<a\b[^>]*?href="([^"]+?)"?[^>]*>(.*?)<[/\\]a>"#)
        .unwrap()
        .replace_all(&src, "[$2]($1)")
        .to_string();
    let src = extract_images(&src, &mut stash);
    let src = extract_math(&src, &mut stash);
    let src = extract_fences(&src, &mut stash);
    // stray \label in prose (outside envs and math) becomes an anchor
    let label_re = regex::Regex::new(r"\\label\{([^}\n]+)\}").unwrap();
    let src = label_re
        .replace_all(&src, |c: &regex::Captures| {
            stash.put(format!(
                "#metadata(none)#label(\"{}\")",
                sanitize_id(&c[1])
            ))
        })
        .to_string();
    // numeric character references leak through the math/stash routes
    let src = regex::Regex::new(r"&#(\d+);")
        .unwrap()
        .replace_all(&src, |c: &regex::Captures| {
            c[1].parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .map(String::from)
                .unwrap_or_default()
        })
        .to_string();
    // Maruku definition lists (`: definition` + 4-space continuation)
    // are not CommonMark; flatten them so continuations don't become
    // indented code blocks
    let src = {
        let mut out: Vec<String> = Vec::new();
        let mut in_dl = false;
        for line in src.lines() {
            if let Some(rest) = line.strip_prefix(": ") {
                in_dl = true;
                out.push(rest.to_string());
            } else if in_dl && line.starts_with("    ") {
                out.push(line[4..].to_string());
            } else {
                if !line.trim().is_empty() {
                    in_dl = false;
                }
                out.push(line.to_string());
            }
        }
        out.join("\n")
    };
    // citation lines wrap link groups in escaped brackets \[..\];
    // CommonMark treats the escapes as literal text and then refuses
    // the inner links. On lines that clearly carry links, turn escaped
    // brackets into character references so the links parse.
    let src = src
        .lines()
        .map(|l| {
            if l.contains("](http") || l.contains("<a ") || l.contains("](/nlab") {
                l.replace("\\[", "&#91;").replace("\\]", "&#93;")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    // named entities for <, >, & are author escapes; keep them literal
    // through markdown
    let src = src
        .replace("&amp;", "&")
        .replace("&lt;", "\\<")
        .replace("&gt;", "\\>");
    // markdown link URLs containing spaces or unbalanced parens make
    // CommonMark drop the whole link: percent-encode the offenders
    let src = fix_md_link_urls(&src);
    // "**bold **" (trailing space) is not CommonMark strong. The
    // opening ** must actually be an opener (start of line or after
    // whitespace/bracket), else this would pair a CLOSING ** with the
    // next opener ("**a**, or **b**" -> "**a**, or** b**")
    let src = regex::Regex::new(r"(?m)(^|[\s(\[{])\*\*(\S[^*\n]*?) +\*\*")
        .unwrap()
        .replace_all(&src, "$1**$2** ")
        .to_string();
    let src = extract_wiki(&src, &mut stash);
    // \ref{Id}: Maruku turns these into numbered links to environment
    // anchors (the live site fills the number with JavaScript; we query
    // the env's metadata statically).
    let ref_re = regex::Regex::new(r"\\ref\{([^}\n]+)\}").unwrap();
    let src = ref_re
        .replace_all(&src, |c: &regex::Captures| {
            stash.put(format!("#nlab-ref(\"{}\");", sanitize_id(&c[1])))
        })
        .to_string();
    // \cite{Key}: bracketed link to the reference-list anchor when it
    // exists, plain bracketed key otherwise
    let cite_re = regex::Regex::new(r"\\cite\{([^}\n]+)\}").unwrap();
    let src = cite_re
        .replace_all(&src, |c: &regex::Captures| {
            stash.put(format!("#nlab-cite(\"{}\");", sanitize_id(&c[1])))
        })
        .to_string();
    // a bare \ref with no target is an author slip; drop the command
    let src = regex::Regex::new(r"\\ref\b")
        .unwrap()
        .replace_all(&src, "")
        .to_string();
    // an anchor at the end of a heading would ride into the heading
    // body and be duplicated by #outline(); move it below the heading
    // Maruku's colon form `{: #Id}` is the same anchor IAL as `{#Id}`
    let src = regex::Regex::new(r"\{:\s*#([\w:-]+)\s*\}")
        .unwrap()
        .replace_all(&src, "{#$1}")
        .to_string();
    let heading_anchor_re =
        regex::Regex::new(r"(?m)^(#+[^\n]*?)\s*\{#([\w:-]+)\}([^\n]*)$").unwrap();
    let src = heading_anchor_re
        .replace_all(&src, "$1$3\n\n{#$2}")
        .to_string();
    // Maruku anchor IALs: {#Id} standalone or inline; a repeated id
    // would be a duplicate typst label, so only the first one anchors
    let anchor_re = regex::Regex::new(r"(^|[^(#])\{#([\w:-]+)\}").unwrap();
    let mut seen_anchors = std::collections::HashSet::new();
    let src = anchor_re
        .replace_all(&src, |c: &regex::Captures| {
            // `[text]({#Id})` is a LINK to the anchor, not the anchor
            if seen_anchors.insert(c[2].to_string()) {
                format!(
                    "{}{}",
                    &c[1],
                    stash.put_src(
                        format!("#metadata(none)#label(\"{}\")", &c[2]),
                        format!("{{#{}}}", &c[2]),
                    )
                )
            } else {
                c[1].to_string()
            }
        })
        .to_string();
    // a stashed block (diagram, math) whose source line was indented
    // must not become an indented code block under cmark. Dedent to 3
    // spaces — but ONLY outside list items: inside an item the
    // indentation is what KEEPS the block in the item.
    let src = {
        let ph_line = regex::Regex::new(r"^[ \t]{4,}(\u{E000}\d+\u{E001})[ \t\r]*$")
            .unwrap();
        let marker = regex::Regex::new(r"^([*+-]|\d+[.)])([ \t]+)").unwrap();
        let mut list_indent: Option<usize> = None;
        let mut out: Vec<String> = Vec::new();
        for line in src.lines() {
            let trimmed = line.trim_start_matches([' ', '\t']);
            let indent = line.len() - trimmed.len();
            if !trimmed.is_empty() {
                if let Some(m) = marker.captures(trimmed) {
                    list_indent =
                        Some(indent + m[1].len() + m[2].len());
                } else if indent == 0 {
                    list_indent = None;
                }
            }
            if let Some(c) = ph_line.captures(line) {
                let in_list = matches!(list_indent, Some(ci) if indent + 1 >= ci);
                if !in_list {
                    out.push(format!("   {}", &c[1]));
                    continue;
                }
            }
            out.push(line.to_string());
        }
        out.join("\n")
    };
    let body = markdown_to_typst(&src, &stash);
    // resolve placeholders (escape pass never touches PUA chars); a
    // literal ( or [ right after a #call() would chain as another
    // argument list, so terminate the expression with `;` first
    let re = regex::Regex::new(&format!("{}(\\d+){}([\\(\\[])?", P0, P1)).unwrap();
    // stash items can themselves contain placeholders (math inside a
    // wiki link's display text): resolve until fixpoint
    let mut body = body;
    for _ in 0..8 {
        if !body.contains(P0) {
            break;
        }
        body = re
            .replace_all(&body, |c: &regex::Captures| {
                let item = &stash.items[c[1].parse::<usize>().unwrap()];
                match c.get(2) {
                    Some(next) if item.ends_with(')') || item.ends_with(']') => {
                        format!("{};{}", item, next.as_str())
                    }
                    Some(next) => format!("{}{}", item, next.as_str()),
                    None => item.clone(),
                }
            })
            .to_string();
    }
    let title_block = title
        .map(|t| format!("#nlab-title[{}]\n", escape_typst(t)))
        .unwrap_or_default();
    let body = emit::nativize_calls(&format!("{}{}\n", title_block, body));
    // typst labels must be unique: only the first attachment of an id
    // survives (anchors, env ids, and math \label{}s share a namespace)
    let mut seen = std::collections::HashSet::new();
    let dedupe_re = regex::Regex::new(
        r#"#metadata\(none\)#label\("([\w:.-]+)"\)|, id: "([\w:.-]+)""#,
    )
    .unwrap();
    let body = dedupe_re
        .replace_all(&body, |c: &regex::Captures| {
            let id = c.get(1).or(c.get(2)).unwrap().as_str().to_string();
            if seen.insert(id) {
                c[0].to_string()
            } else {
                String::new()
            }
        })
        .to_string();
    emit::localize_calls(format!("{}\n{}", PAGE_PREAMBLE, body))
}
