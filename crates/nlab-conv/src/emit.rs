//! Port of rewrites/emit_fletcher.py: fix_tex plus the fletcher /
//! signature-grid / table / equation emitters. Target: byte-identical
//! output to the Python for every diagram in the corpus.

use crate::grid::{self, kind, parts, Cell};
use once_cell_lite::Lazy;
use regex::Regex;

// A tiny Lazy shim so we don't pull once_cell just for statics.
mod once_cell_lite {
    pub struct Lazy<T>(std::sync::OnceLock<T>, fn() -> T);
    impl<T> Lazy<T> {
        pub const fn new(f: fn() -> T) -> Self {
            Self(std::sync::OnceLock::new(), f)
        }
    }
    impl<T> std::ops::Deref for Lazy<T> {
        type Target = T;
        fn deref(&self) -> &T {
            self.0.get_or_init(self.1)
        }
    }
}

const PREAMBLE: &str = "#import \"@preview/fletcher:0.5.8\": diagram, node, edge\n#import \"@preview/mitex:0.2.6\": mi, mitex\n#set page(width: auto, height: auto, margin: 4pt, fill: white)\n#set text(size: 11pt)\n";
const PREAMBLE_LOCAL: &str = "#import \"@preview/fletcher:0.5.8\": diagram, node, edge\n#import \"@local/mitex:0.2.7\": mi-itex, mitex-itex\n#set page(width: auto, height: auto, margin: 4pt, fill: white)\n#set text(size: 11pt)\n";

/// In local-mitex mode the itex word-identifier dialect lives in the
/// converter (mi-itex), so the emitted calls switch and the word-grouping
/// regex pass is skipped.
pub(crate) fn localize_calls(code: String) -> String {
    if !local_mitex() {
        return code;
    }
    code.replace("#mitex(", "#mitex-itex(").replace("mi(", "mi-itex(")
}

/// In local-mitex mode (see context.rs) emit against the locally built
/// mitex package (fork branch `nlab`), whose fixes make several fix_tex
/// workarounds unnecessary: the circled-operator/set-operator unicode
/// substitutions, \mathscr -> \mathcal, \mathsf -> \textsf, and the
/// plain (non-pair) \underoverset translation.
fn local_mitex() -> bool {
    crate::context::local_mitex()
}

fn preamble() -> &'static str {
    if local_mitex() { PREAMBLE_LOCAL } else { PREAMBLE }
}

/// Python dict.get truthiness: an empty-string value is as good as absent.
fn cget<'a>(c: &'a Cell, k: &str) -> Option<&'a str> {
    grid::get(c, k).filter(|s| !s.is_empty())
}

// ------------------------------------------------------------- fix_tex

static CIRCLED: &[(&str, &str)] = &[
    ("bigotimes", "⨂"),
    ("bigoplus", "⨁"),
    ("bigodot", "⨀"),
    ("otimes", "⊗"),
    ("oplus", "⊕"),
    ("ominus", "⊖"),
    ("odot", "⊙"),
    ("oslash", "⊘"),
    ("circledast", "⊛"),
    ("circledcirc", "⊚"),
    ("cap", "∩"),
    ("cup", "∪"),
    ("bigcap", "⋂"),
    ("bigcup", "⋃"),
    ("setminus", "∖"),
];

static CIRCLED_RE: Lazy<Regex> = Lazy::new(|| {
    let alts: Vec<&str> = CIRCLED.iter().map(|(k, _)| *k).collect();
    Regex::new(&format!(r"\\({})\b", alts.join("|"))).unwrap()
});
static LIM_L_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\{?\\lim_\{?\\leftarrow\}?\}?").unwrap());
static LIM_R_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\{?\\lim_\{?\\(?:to|rightarrow)\}?\}?").unwrap());
static LAP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\\math[lr]lap\s*").unwrap());
static RLAP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\\r?lap\s*").unwrap());
static PHANTOM_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\\phantom\s*\{[^{}]*\}").unwrap());
static HSPACE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\\hspace\s*\{[^{}]*\}").unwrap());
static SLASH_N_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\\n\b").unwrap());
static MATHSCR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\\mathscr\b").unwrap());
static MATHSF_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\\mathsf\b").unwrap());
static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
static WORD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(\\[a-zA-Z]+)|([a-zA-Z]{2,})").unwrap());
static PROTECTED_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"\\(?:text|textsf|textrm|texttt|mathrm|mathit|mathbf|mathsf|mathtt|operatorname|mathcal|mathfrak|mathscr|mathbb|begin|end)\s*\{[^{}]*\}",
    )
    .unwrap()
});
static STACKED_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\\stackrel\s*\{\s*\\stackrel\s*").unwrap());
static STACKREL_HEAD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\\stackrel\s*").unwrap());
static LABELED_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\\(stackrel|overset|underset)\s*").unwrap());
static ARROWISH_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\\(long)?(left|right)arrow|\\to\b").unwrap());
static STASH_RE: Lazy<Regex> = Lazy::new(|| Regex::new("\u{1}(\\d+)\u{1}").unwrap());
static SAVED_RE: Lazy<Regex> = Lazy::new(|| Regex::new("\u{0}(\\d+)\u{0}").unwrap());

fn stack_long(cmd: &str) -> String {
    let long = match cmd {
        "to" | "rightarrow" => "longrightarrow",
        "leftarrow" => "longleftarrow",
        "mapsto" => "longmapsto",
        c => c,
    };
    format!("\\{}", long)
}

fn translate_stacked_pairs(mut tex: String, stash: &mut Vec<String>) -> String {
    loop {
        let m = match STACKED_RE.find(&tex) {
            Some(m) => m,
            None => return tex,
        };
        let (a, rest) = grid::read_group(&tex[m.end()..]);
        let (ar1, rest) = grid::read_group(&rest);
        let rest = rest.trim_start().to_string();
        if !rest.starts_with('}') {
            return tex;
        }
        let (inner, rest2) = grid::read_group(&rest[1..]);
        let m2 = match STACKREL_HEAD_RE.find(&inner) {
            Some(m2) => m2,
            None => return tex,
        };
        let (b, r) = grid::read_group(&inner[m2.end()..]);
        let (ar2, r) = grid::read_group(&r);
        if !r.trim().is_empty() {
            return tex;
        }
        let longen = |ar: &str| stack_long(ar.trim().trim_start_matches('\\'));
        let repl = format!(
            "\\underset{{\\displaystyle\\underset{{{}}}{{{}}}}}{{\\displaystyle\\overset{{{}}}{{{}}}}}",
            b,
            longen(&ar2),
            a,
            longen(&ar1)
        );
        stash.push(repl);
        tex = format!("{}\u{1}{}\u{1}{}", &tex[..m.start()], stash.len() - 1, rest2);
    }
}

fn translate_underoverset(mut tex: String, stash: &mut Vec<String>) -> String {
    let mut search_from = 0usize;
    while let Some(off) = tex[search_from..].find("\\underoverset") {
        let i = search_from + off;
        let (below, rest) = grid::read_group(&tex[i + "\\underoverset".len()..]);
        let (above, rest) = grid::read_group(&rest);
        let (base, rest) = grid::read_group(&rest);
        let is_pair = ARROWISH_RE.is_match(&above) && ARROWISH_RE.is_match(&below);
        if local_mitex() && !is_pair {
            // the fixed mitex knows \underoverset natively
            search_from = i + "\\underoverset".len();
            continue;
        }
        let repl = if is_pair {
            let base_s = base.trim();
            let base_s = if base_s.is_empty() { "\\;" } else { base_s };
            let repl = format!(
                "\\underset{{\\displaystyle {}}}{{\\overset{{\\displaystyle {}}}{{{}}}}}",
                below, above, base_s
            );
            stash.push(repl);
            format!("\u{1}{}\u{1}", stash.len() - 1)
        } else {
            format!("\\overset{{{}}}{{\\underset{{{}}}{{{}}}}}", above, below, base)
        };
        tex = format!("{}{}{}", &tex[..i], repl, rest);
    }
    tex
}

fn translate_labeled_arrows(mut tex: String) -> String {
    let mut pos = 0usize;
    loop {
        let (m_start, m_end) = match LABELED_RE.find_at(&tex, pos) {
            Some(m) => (m.start(), m.end()),
            None => return tex,
        };
        let which: String = tex[m_start + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        let (label, rest) = grid::read_group(&tex[m_end..]);
        let (arrow, rest2) = grid::read_group(&rest);
        let cmd = arrow.trim().trim_start_matches('\\').to_string();
        let is_r = matches!(cmd.as_str(), "to" | "rightarrow" | "longrightarrow");
        let is_l = matches!(cmd.as_str(), "leftarrow" | "longleftarrow");
        if arrow.trim() == format!("\\{}", cmd) && (is_r || is_l) && which != "underset" {
            let base = if is_r { "\\xrightarrow" } else { "\\xleftarrow" };
            let repl = format!("{}{{{}}}", base, label);
            tex = format!("{}{}{}", &tex[..m_start], repl, rest2);
            pos = m_start + repl.len();
        } else {
            pos = m_end;
        }
    }
}

pub(crate) fn fix_tex(tex: &str) -> String {
    let mut stash: Vec<String> = Vec::new();
    // an n-ary operator with limits underneath is the big variant
    // (authors write \underset{i \in I}{\cup} meaning \bigcup)
    static UNDER_NARY: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"\\underset\{([^{}]*)\}\{\\(cup|cap|vee|wedge|otimes|oplus|sqcup)\}",
        )
        .unwrap()
    });
    let tex = UNDER_NARY
        .replace_all(tex, "\\underset{$1}{\\big$2}")
        .to_string();
    let tex = translate_stacked_pairs(tex.to_string(), &mut stash);
    let tex = translate_underoverset(tex, &mut stash);
    let tex = translate_labeled_arrows(tex);
    let tex = STASH_RE
        .replace_all(&tex, |c: &regex::Captures| {
            stash[c[1].parse::<usize>().unwrap()].clone()
        })
        .to_string();
    let tex = LIM_L_RE
        .replace_all(&tex, "\\underset{\\longleftarrow}{\\lim}{}")
        .to_string();
    let tex = LIM_R_RE
        .replace_all(&tex, "\\underset{\\longrightarrow}{\\lim}{}")
        .to_string();
    let tex = if local_mitex() {
        tex
    } else {
        CIRCLED_RE
            .replace_all(&tex, |c: &regex::Captures| {
                CIRCLED
                    .iter()
                    .find(|(k, _)| *k == &c[1])
                    .map(|(_, v)| *v)
                    .unwrap()
                    .to_string()
            })
            .to_string()
    };
    let tex = LAP_RE.replace_all(&tex, "").to_string();
    let tex = RLAP_RE.replace_all(&tex, "").to_string();
    let tex = PHANTOM_RE.replace_all(&tex, "").to_string();
    let tex = HSPACE_RE.replace_all(&tex, "\\;").to_string();
    let tex = pipe_rule(&tex);
    let tex = SLASH_N_RE.replace_all(&tex, "n").to_string();
    let tex = if local_mitex() {
        tex
    } else {
        let tex = MATHSCR_RE.replace_all(&tex, "\\mathcal").to_string();
        MATHSF_RE.replace_all(&tex, "\\textsf").to_string()
    };
    let tex = WS_RE.replace_all(&tex, " ").to_string();

    if local_mitex() {
        // itex word tokenization is native in the local mitex (mi-itex)
        return tex.trim().to_string();
    }
    let mut saved: Vec<String> = Vec::new();
    let tex = PROTECTED_RE
        .replace_all(&tex, |c: &regex::Captures| {
            saved.push(c[0].to_string());
            format!("\u{0}{}\u{0}", saved.len() - 1)
        })
        .to_string();
    let tex = WORD_RE
        .replace_all(&tex, |c: &regex::Captures| {
            if let Some(cmd) = c.get(1) {
                cmd.as_str().to_string()
            } else {
                format!("\\mathrm{{{}}}", &c[2])
            }
        })
        .to_string();
    let tex = SAVED_RE
        .replace_all(&tex, |c: &regex::Captures| {
            saved[c[1].parse::<usize>().unwrap()].clone()
        })
        .to_string();
    tex.trim().to_string()
}

/// Python: (?<![\\{])\|(?!_) -> {\mid}   (hand-rolled: no lookarounds in
/// the regex crate)
fn pipe_rule(tex: &str) -> String {
    let chars: Vec<char> = tex.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == '|' {
            let prev_ok = i == 0 || (chars[i - 1] != '\\' && chars[i - 1] != '{');
            let next_ok = i + 1 >= chars.len() || chars[i + 1] != '_';
            if prev_ok && next_ok {
                out.push_str("{\\mid}");
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// itex2MML built-ins (and common author shorthands) that mitex's spec
/// lacks. Applied only in the prose/tikzcd layer — never inside ts(),
/// which must stay byte-identical to the Python pipeline.
pub(crate) fn fix_itex_builtins(tex: &str) -> String {
    const SUBS: &[(&str, &str)] = &[
        ("id", "\\mathrm{id}"),
        ("im", "\\mathrm{im}"),
        ("dom", "\\mathrm{dom}"),
        ("cod", "\\mathrm{cod}"),
        ("coker", "\\mathrm{coker}"),
        ("supp", "\\mathrm{supp}"),
        ("len", "\\mathrm{len}"),
        ("Map", "\\mathrm{Map}"),
        ("Maps", "\\mathrm{Maps}"),
        ("Hom", "\\mathrm{Hom}"),
        ("colim", "\\mathrm{colim}"),
        ("Aut", "\\mathrm{Aut}"),
        ("into", "\\hookrightarrow"),
        ("square", "□"),
        ("Box", "□"),
        ("op", "\\mathrm{op}"),
        ("widebar", "\\overline"),
        ("parr", "⅋"),
        ("GL", "\\mathrm{GL}"),
        ("Cat", "\\mathrm{Cat}"),
        ("bottom", "\\bot"),
        ("onto", "\\twoheadrightarrow"),
        ("End", "\\mathrm{End}"),
        ("Ob", "\\mathrm{Ob}"),
        ("Mor", "\\mathrm{Mor}"),
        ("esh", "ʃ"),
        ("qed", "∎"),
        ("infinity", "\\infty"),
        ("sslash", "⫽"),
        ("product", "\\prod"),
        ("swArrow", "⇙"),
        ("seArrow", "⇘"),
        ("neArrow", "⇗"),
        ("nwArrow", "⇖"),
    ];
    let re = regex::Regex::new(r"\\([a-zA-Z]+)").unwrap();
    re.replace_all(tex, |c: &regex::Captures| {
        for (name, sub) in SUBS {
            if &c[1] == *name {
                return format!("{} ", sub);
            }
        }
        c[0].to_string()
    })
    .to_string()
}

pub(crate) fn ts(tex: &str) -> String {
    let fixed = fix_tex(tex);
    format!("\"{}\"", fixed.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Rewrite `mi("...")` / `#mitex("...")` calls in emitted code into
/// native typst math (`$...$` / `$ ... $`) by running the fork's
/// converter offline — the exact code path the wasm plugin would run at
/// compile time, so rendering is identical. Conversion failures keep
/// the plugin call as a fallback.
pub(crate) fn nativize_calls(code: &str) -> String {
    let re =
        regex::Regex::new(r#"(#?)\b(mi|mitex)\("((?:[^"\\]|\\.)*)"\)"#).unwrap();
    re.replace_all(code, |c: &regex::Captures| {
        let tex = unescape_str(&c[3]);
        match mitex::convert_math_itex(&tex, None) {
            Ok(res) if !res.trim().is_empty() => {
                let res = res.trim().replace('\n', " ");
                // restriction bars: mitex spaces `|` like a relation,
                // but `p|_X` is an ordinary tight atom in LaTeX
                let res = res.replace("| _(", "#math.class(\"normal\", sym.bar.v)_(");
                // a cell starting with a bare script has nothing to
                // attach to; give it a zero-width base
                let res = regex::Regex::new(r"(^|[,;(])(\s*)([_^])\(")
                    .unwrap()
                    .replace_all(&res, "${1}${2}zws ${3}(")
                    .to_string();
                // empty accent/function calls choke typst: fill with a
                // zero-width base
                let res = res.replace("()", "(zws)");
                // reversed floor/ceil pairs (author typos) are parse
                // errors: typst matches ⌊..⌋ like brackets. Only when
                // a family is mis-nested, demote ALL its glyphs to
                // plain symbols (well-paired ones must stay delimiters)
                let mut res = res;
                for (l, r, ls, rs) in [
                    ('⌊', '⌋', "#sym.floor.l;", "#sym.floor.r;"),
                    ('⌈', '⌉', "#sym.ceil.l;", "#sym.ceil.r;"),
                ] {
                    let mut depth = 0i64;
                    let mut ok = true;
                    for ch in res.chars() {
                        if ch == l {
                            depth += 1;
                        } else if ch == r {
                            depth -= 1;
                            if depth < 0 {
                                ok = false;
                            }
                        }
                    }
                    if !ok || depth != 0 {
                        res = res
                            .replace(l, ls)
                            .replace(r, rs);
                    }
                }
                // \atop with a missing side leaves a dangling comma
                let res = regex::Regex::new(r"atop\(([^()]*), *\)")
                    .unwrap()
                    .replace_all(&res, "atop(${1}, zws)")
                    .to_string();
                // cross-cell \left/\right (or similar) leaves the
                // converted fragment with unbalanced parens; that is a
                // guaranteed typst parse error, so degrade instead
                let mut depth = 0i64;
                let mut bad = false;
                let mut prev = ' ';
                for ch in res.chars() {
                    if prev != '\\' {
                        match ch {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth < 0 {
                                    bad = true;
                                }
                            }
                            _ => {}
                        }
                    }
                    prev = ch;
                }
                if std::env::var("NLAB_DEBUG_BAL").is_ok() && (bad || depth != 0) {
                    eprintln!("UNBALANCED ({} {}): {}", bad, depth, res);
                }
                if bad || depth != 0 {
                    let mut short = tex.trim().replace('\n', " ");
                    if short.len() > 120 {
                        short.truncate(120);
                        short.push('\u{2026}');
                    }
                    return format!("{}nlab-tex-fallback({:?})", &c[1], short);
                }
                if &c[2] == "mitex" {
                    format!("$ {} $", res) // display/block math
                } else {
                    format!("${}$", res)
                }
            }
            Err(_) => {
                // the plugin would fail identically at compile time and
                // kill the page: degrade to a verbatim formula instead
                let mut short = tex.trim().replace('\n', " ");
                if short.len() > 120 {
                    short.truncate(120);
                    short.push('\u{2026}');
                }
                // match the call context: `mi(..)` sits in code, but
                // `#mi(..)` / `#mitex(..)` sit in markup
                format!("{}nlab-tex-fallback({:?})", &c[1], short)
            }
            _ => c[0].to_string(),
        }
    })
    .to_string()
}

fn unescape_str(s: &str) -> String {
    let mut out = String::new();
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '\\' && i + 1 < b.len() {
            out.push(b[i + 1]);
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

// -------------------------------------------------------------- tables

fn mark_for(dir: &str) -> &'static str {
    match dir {
        "lr" => "<->",
        "~" => "-",
        "veq" => "=",
        _ => "->",
    }
}

fn hook_mark(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "hookrightarrow" | "hookleftarrow" => "hook->",
        "twoheadrightarrow" | "twoheadleftarrow" => "->>",
        "mapsto" | "longmapsto" => "|->",
        "Rightarrow" | "Leftarrow" => "=>",
        "Downarrow" | "Uparrow" => "=>",
        "seArrow" | "swArrow" | "neArrow" | "nwArrow" => "=>",
        _ => return None,
    })
}

fn edge_mark(cell: &Cell) -> &'static str {
    cget(cell, "cmd")
        .and_then(hook_mark)
        .unwrap_or_else(|| mark_for(cget(cell, "dir").unwrap()))
}

fn steps(dir: &str) -> (i32, i32) {
    match dir {
        "se" => (1, 1),
        "sw" => (1, -1),
        "ne" => (-1, 1),
        _ => (-1, -1), // nw
    }
}

fn travel(dir: &str) -> (i32, i32) {
    match dir {
        "r" | "lr" | "~" => (1, 0),
        "l" => (-1, 0),
        "d" | "veq" => (0, 1),
        "u" => (0, -1),
        "se" => (1, 1),
        "sw" => (-1, 1),
        "ne" => (1, -1),
        _ => (-1, -1), // nw
    }
}

fn want(placement: &str) -> (i32, i32) {
    match placement {
        "above" => (0, -1),
        "below" => (0, 1),
        "east" => (1, 0),
        _ => (-1, 0), // west
    }
}

fn label_side(direction: &str, placement: &str) -> &'static str {
    let (tx, ty) = travel(direction);
    let (lx, ly) = (ty, -tx);
    let (wx, wy) = want(placement);
    if lx * wx + ly * wy > 0 {
        "left"
    } else {
        "right"
    }
}

fn tilde_label(cmd: &str) -> &'static str {
    match cmd {
        "simeq" => "\\simeq",
        "cong" => "\\cong",
        "equiv" => "\\equiv",
        _ => "=",
    }
}

// -------------------------------------------------- signature and table

fn arrow_tex(cell: &Cell) -> String {
    let mut base = if cget(cell, "dir") == Some("~") && cget(cell, "cmd") == Some("=") {
        "=".to_string()
    } else {
        format!("\\{}", cget(cell, "cmd").unwrap_or("to"))
    };
    if let Some(a) = cget(cell, "above") {
        base = format!("\\overset{{{}}}{{{}}}", a, base);
    }
    if let Some(b) = cget(cell, "below") {
        base = format!("\\underset{{{}}}{{{}}}", b, base);
    }
    base
}

fn signature_rows(grid: &[Vec<Cell>]) -> Option<Vec<[Cell; 3]>> {
    let mut rows = Vec::new();
    for row in grid {
        let filled: Vec<&Cell> = row.iter().filter(|c| kind(c) != "e").collect();
        if filled.len() == 3
            && kind(filled[0]) == "o"
            && kind(filled[1]) == "h"
            && kind(filled[2]) == "o"
        {
            rows.push([filled[0].clone(), filled[1].clone(), filled[2].clone()]);
        } else {
            return None;
        }
    }
    if rows.len() >= 2 {
        Some(rows)
    } else {
        None
    }
}

fn sig_sym(cmd: &str) -> &'static str {
    match cmd {
        "mapsto" | "longmapsto" => "arrow.r.bar",
        "hookrightarrow" => "arrow.r.hook",
        "twoheadrightarrow" => "arrow.r.twohead",
        "Rightarrow" => "arrow.r.double",
        "leftarrow" | "longleftarrow" => "arrow.l",
        "leftrightarrow" => "arrow.l.r",
        _ => "arrow.r",
    }
}

fn sig_arrow(cell: &Cell) -> String {
    if cget(cell, "dir") == Some("~") {
        return format!("mi({})", ts(&arrow_tex(cell)));
    }
    let sym = sig_sym(cget(cell, "cmd").unwrap_or(""));
    let core = format!("stretch({}, size: #2.4em)", sym);
    let mut attach = Vec::new();
    // \phantom labels only matched arrow lengths in LaTeX; the fixed
    // stretch does that here, and the invisible label inflates the row
    let visible = |l: &str| !l.trim_start().starts_with("\\phantom");
    if let Some(a) = cget(cell, "above").filter(|l| visible(l)) {
        attach.push(format!("t: #text(0.72em, mi({}))", ts(a)));
    }
    if let Some(b) = cget(cell, "below").filter(|l| visible(l)) {
        attach.push(format!("b: #text(0.72em, mi({}))", ts(b)));
    }
    if !attach.is_empty() {
        format!("$attach({}, {})$", core, attach.join(", "))
    } else {
        format!("${}$", core)
    }
}

fn emit_signature(rows: &[[Cell; 3]]) -> String {
    let mut cells = Vec::new();
    for [lhs, arrow, rhs] in rows {
        cells.push(format!(
            "  mi({}),",
            ts(&format!("\\displaystyle {}", cget(lhs, "tex").unwrap()))
        ));
        cells.push(format!("  {},", sig_arrow(arrow)));
        cells.push(format!(
            "  mi({}),",
            ts(&format!("\\displaystyle {}", cget(rhs, "tex").unwrap()))
        ));
    }
    format!(
        "#grid(\n  columns: 3, column-gutter: 0.4em, row-gutter: 0.8em,\n  align: (right + horizon, center + horizon, left + horizon),\n{}\n)\n",
        cells.join("\n")
    )
}

fn emit_table(grid: &[Vec<Cell>]) -> String {
    let cols = grid.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut cells = Vec::new();
    for row in grid {
        for i in 0..cols {
            let c = row.get(i);
            match c {
                Some(c) if kind(c) == "o" => cells.push(format!(
                    "  mi({}),",
                    ts(&format!("\\displaystyle {}", cget(c, "tex").unwrap()))
                )),
                _ => cells.push("  [],".to_string()),
            }
        }
    }
    format!(
        "#grid(\n  columns: {}, column-gutter: 1.4em, row-gutter: 1em,\n  align: center + horizon,\n{}\n)\n",
        cols,
        cells.join("\n")
    )
}

// ------------------------------------------------------------- diagram

type RC = (usize, usize);

fn nearest_object(grid: &[Vec<Cell>], r: usize, c: usize, dr: i32, dc: i32) -> Option<RC> {
    let mut rr = r as i32 + dr;
    let mut cc = c as i32 + dc;
    while rr >= 0 && (rr as usize) < grid.len() {
        if cc >= 0 && (cc as usize) < grid[rr as usize].len() {
            if kind(&grid[rr as usize][cc as usize]) == "o" {
                return Some((rr as usize, cc as usize));
            }
        } else {
            return None;
        }
        rr += dr;
        cc += dc;
    }
    None
}

pub(crate) fn emit(grid: &[Vec<Cell>]) -> (String, Option<String>) {
    if let Some(sig) = signature_rows(grid) {
        return ("ok".into(), Some(emit_signature(&sig)));
    }
    // a row of nothing but \vdots sitting DIRECTLY above/below an
    // object row hugs that row (the source relies on tight array
    // spacing); demote those cells to flush floats
    let mut grid_owned;
    let mut grid = grid;
    let mut vdots_flush: Vec<(usize, usize, bool)> = Vec::new();
    {
        let mut marks: Vec<(usize, usize, bool)> = Vec::new();
        for r in 0..grid.len() {
            let mut any = false;
            let mut all = true;
            for cell in &grid[r] {
                match kind(cell) {
                    "o" => {
                        let t = cget(cell, "tex").unwrap_or("").trim();
                        if t == "\\vdots" || t == "⋮" {
                            any = true;
                        } else {
                            all = false;
                        }
                    }
                    "e" => {}
                    _ => all = false,
                }
            }
            if !(any && all) {
                continue;
            }
            let obj_row = |rr: &Vec<Cell>| {
                rr.iter().any(|c| {
                    kind(c) == "o"
                        && cget(c, "tex").map_or(false, |t| {
                            t.trim() != "\\vdots" && t.trim() != "⋮"
                        })
                })
            };
            let below = r + 1 < grid.len() && obj_row(&grid[r + 1]);
            let above = r > 0 && obj_row(&grid[r - 1]);
            if below || above {
                for (c, cell) in grid[r].iter().enumerate() {
                    if kind(cell) == "o" {
                        marks.push((r, c, below));
                    }
                }
            }
        }
        if !marks.is_empty() {
            grid_owned = grid.to_vec();
            for (r, c, _) in &marks {
                grid_owned[*r][*c] = grid::cell("e");
            }
            grid = &grid_owned;
            vdots_flush = marks;
        }
    }
    // objects in row-major insertion order (Python dict semantics)
    let mut obj_order: Vec<RC> = Vec::new();
    let mut obj_tex: std::collections::HashMap<RC, String> = Default::default();
    for (r, row) in grid.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if kind(cell) == "o" {
                obj_order.push((r, c));
                obj_tex.insert((r, c), cget(cell, "tex").unwrap().to_string());
            }
        }
    }
    if obj_order.is_empty() {
        return ("empty".into(), None);
    }

    let quadrant_object = |order: &[RC], r: usize, c: usize, dr: i32, dc: i32| -> Option<RC> {
        let mut best: Option<(i32, RC)> = None;
        for &(rr, cc) in order {
            let drr = rr as i32 - r as i32;
            let dcc = cc as i32 - c as i32;
            if dr * drr < 0 || dc * dcc < 0 || (rr, cc) == (r, c) {
                continue;
            }
            let d = drr.abs().max(dcc.abs());
            if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, (rr, cc)));
            }
        }
        best.map(|(_, rc)| rc)
    };
    let resolve_diagonal = |r: usize, c: usize, dir: &str| -> (Option<RC>, Option<RC>) {
        let (dr, dc) = steps(dir);
        let a = nearest_object(grid, r, c, -dr, -dc)
            .or_else(|| quadrant_object(&obj_order, r, c, -dr, -dc));
        let b = nearest_object(grid, r, c, dr, dc)
            .or_else(|| quadrant_object(&obj_order, r, c, dr, dc));
        (a, b)
    };

    // Resolve every arrow to its endpoint objects first.
    let mut edges: Vec<(RC, RC, Cell)> = Vec::new();
    let mut floats: Vec<(usize, usize, String)> = Vec::new();
    // (top object, bottom object, arrow cell, horizontal offset in pt)
    let mut vvlines: Vec<(RC, RC, Cell, f64)> = Vec::new();
    // 2-cell diagonals, held back until we know the other edges
    let mut twocells: Vec<(usize, usize, RC, RC, Cell)> = Vec::new();
    for (r, row) in grid.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let k = kind(cell).to_string();
            if k == "dd" {
                let ps = parts(cell).unwrap_or(&[]);
                let two_way = ps.len() == 2
                    && ps
                        .iter()
                        .map(|p| cget(p, "dir"))
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        == 2;
                for (pi, part) in ps.iter().enumerate() {
                    let (a, b) = resolve_diagonal(r, c, cget(part, "dir").unwrap());
                    match (a, b) {
                        (Some(a), Some(b)) => {
                            let mut pc = part.clone();
                            // opposite diagonals sharing endpoints (an
                            // adjoint pair) must not overlap
                            if two_way {
                                grid::set(
                                    &mut pc,
                                    "vshift",
                                    if pi == 0 { "3" } else { "3" }.to_string(),
                                );
                            }
                            edges.push((a, b, pc));
                        }
                        _ => return ("dangling".into(), None),
                    }
                }
                // a relation between the two arrows (f ↘ ≅ ↙ g) floats
                // at the cell position
                if let Some(mid) = cget(cell, "mid") {
                    floats.push((r, c, mid.to_string()));
                }
                continue;
            }
            if k == "vv" {
                // parallel vertical arrows: same endpoints, spread by
                // horizontal shifts (u-arrows flip both ends and sign)
                let a0 = nearest_object(grid, r, c, -1, 0)
                    .or_else(|| nearest_object(grid, r, c, -1, -1))
                    .or_else(|| nearest_object(grid, r, c, -1, 1));
                let b0 = nearest_object(grid, r, c, 1, 0)
                    .or_else(|| nearest_object(grid, r, c, 1, -1))
                    .or_else(|| nearest_object(grid, r, c, 1, 1));
                let (a0, b0) = match (a0, b0) {
                    (Some(a0), Some(b0)) => (a0, b0),
                    _ => return ("dangling".into(), None),
                };
                if let Some(mid) = cget(cell, "mid") {
                    floats.push((r, c, mid.to_string()));
                }
                let ps = parts(cell).unwrap().to_vec();
                let n = ps.len();
                let mut pcs: Vec<Cell> = Vec::new();
                for (i, part) in ps.iter().enumerate() {
                    let mut pc = part.clone();
                    // the first arrow's label reads better on the west
                    // side, away from its right-hand neighbors
                    if i == 0 && n > 1 {
                        if let Some(e) = cget(&pc, "east").map(str::to_string) {
                            if cget(&pc, "west").is_none() {
                                grid::unset(&mut pc, "east");
                                grid::set(&mut pc, "west", e);
                            }
                        }
                    }
                    pcs.push(pc);
                }
                // spread wider only when something sits BETWEEN arrows
                let inner = cget(cell, "mid").is_some()
                    || pcs.iter().enumerate().any(|(i, p)| {
                        (i + 1 < n && cget(p, "east").is_some())
                            || (i > 0 && cget(p, "west").is_some())
                    });
                let gap = if cget(cell, "mid").is_some() {
                    14.0
                } else if inner {
                    9.0
                } else {
                    6.0
                };
                if a0.1 == b0.1 {
                    // same column: explicit coordinate lines, so narrow
                    // nodes cannot un-clip a shifted edge
                    for (i, pc) in pcs.into_iter().enumerate() {
                        let off = (i as f64 - (n as f64 - 1.0) / 2.0) * gap;
                        vvlines.push((a0, b0, pc, off));
                    }
                } else {
                    for (i, mut pc) in pcs.into_iter().enumerate() {
                        let (mut a, mut b) = (a0, b0);
                        let up = cget(&pc, "dir") == Some("u");
                        if up {
                            std::mem::swap(&mut a, &mut b);
                        }
                        let mut sh =
                            ((i as f64 - (n as f64 - 1.0) / 2.0) * gap) as i32;
                        if up {
                            sh = -sh;
                        }
                        grid::set(&mut pc, "vshift", sh.to_string());
                        edges.push((a, b, pc));
                    }
                }
                continue;
            }
            if k != "h" && k != "v" && k != "d" {
                continue;
            }
            let dir = cget(cell, "dir").unwrap().to_string();
            let (mut a, mut b);
            if k == "h" {
                a = nearest_object(grid, r, c, 0, -1);
                b = nearest_object(grid, r, c, 0, 1);
                if dir == "l" {
                    std::mem::swap(&mut a, &mut b);
                }
                if dir == "~" && (a.is_none() || b.is_none()) {
                    a = nearest_object(grid, r, c, -1, 0);
                    b = nearest_object(grid, r, c, 1, 0);
                }
            } else if k == "v" {
                // a double arrow with no object directly above/below is
                // a 2-cell floating in a square's interior, not an edge
                if matches!(cget(cell, "cmd"), Some("Downarrow") | Some("Uparrow"))
                    && (nearest_object(grid, r, c, -1, 0).is_none()
                        || nearest_object(grid, r, c, 1, 0).is_none())
                {
                    let base = if cget(cell, "cmd") == Some("Uparrow") {
                        "\\Uparrow"
                    } else {
                        "\\Downarrow"
                    };
                    let tex = if let Some(e) = cget(cell, "east") {
                        format!("{}^{{{}}}", base, e)
                    } else if let Some(w) = cget(cell, "west") {
                        format!("{{}}^{{{}}}{}", w, base)
                    } else {
                        base.to_string()
                    };
                    floats.push((r, c, tex));
                    continue;
                }
                a = nearest_object(grid, r, c, -1, 0)
                    .or_else(|| nearest_object(grid, r, c, -1, -1))
                    .or_else(|| nearest_object(grid, r, c, -1, 1));
                b = nearest_object(grid, r, c, 1, 0)
                    .or_else(|| nearest_object(grid, r, c, 1, -1))
                    .or_else(|| nearest_object(grid, r, c, 1, 1));
                if dir == "u" {
                    std::mem::swap(&mut a, &mut b);
                }
            } else {
                let (aa, bb) = resolve_diagonal(r, c, &dir);
                a = aa;
                b = bb;
            }
            match (a, b) {
                (Some(a), Some(b))
                    if k == "d"
                        && matches!(
                            cget(cell, "cmd"),
                            Some("seArrow") | Some("swArrow") | Some("neArrow")
                                | Some("nwArrow")
                        ) =>
                {
                    twocells.push((r, c, a, b, cell.clone()));
                }
                (Some(a), Some(b)) => edges.push((a, b, cell.clone())),
                _ => {
                    if matches!(cget(cell, "cmd"), Some("Downarrow") | Some("Uparrow")) {
                        continue;
                    }
                    return ("dangling".into(), None);
                }
            }
        }
    }

    // a 2-cell diagonal whose endpoints another edge already connects
    // is an annotation riding that path: float it at its own cell
    for (r, c, a, b, cell) in twocells {
        let dup = edges.iter().any(|(ea, eb, _)| {
            (*ea == a && *eb == b) || (*ea == b && *eb == a)
        });
        if dup {
            let glyph = match cget(&cell, "cmd") {
                Some("seArrow") => "⇘",
                Some("neArrow") => "⇗",
                Some("nwArrow") => "⇖",
                _ => "⇙",
            };
            let tex = if let Some(l) =
                cget(&cell, "east").or_else(|| cget(&cell, "west"))
            {
                format!("{{{}}}_{{{}}}", glyph, l)
            } else {
                glyph.to_string()
            };
            floats.push((r, c, tex));
        } else {
            edges.push((a, b, cell));
        }
    }

    if edges.is_empty() && vvlines.is_empty() {
        return ("ok".into(), Some(emit_table(grid)));
    }

    // Merge annotation objects into adjacent endpoint objects.
    let endpoints: std::collections::HashSet<RC> =
        edges.iter().flat_map(|(a, b, _)| [*a, *b]).collect();
    let mut unused: Vec<RC> = obj_order
        .iter()
        .copied()
        .filter(|rc| !endpoints.contains(rc))
        .collect();
    unused.sort();
    for rc in unused {
        let (r, c) = rc;
        for dc in [1i32, -1i32] {
            let nb = (r, (c as i32 + dc) as usize);
            if c as i32 + dc < 0 {
                continue;
            }
            if obj_tex.contains_key(&nb) && endpoints.contains(&nb) {
                let tex = obj_tex[&rc].clone();
                let other = obj_tex[&nb].clone();
                let new = if dc == 1 {
                    format!("{} {}", tex, other)
                } else {
                    format!("{} {}", other, tex)
                };
                obj_tex.insert(nb, new);
                obj_tex.remove(&rc);
                obj_order.retain(|x| *x != rc);
                break;
            }
        }
    }

    // Compress away rows/columns that hold no objects.
    let mut rs: Vec<usize> = obj_order.iter().map(|(r, _)| *r).collect();
    rs.sort();
    rs.dedup();
    let mut cs: Vec<usize> = obj_order.iter().map(|(_, c)| *c).collect();
    cs.sort();
    cs.dedup();
    let rmap = |r: usize| rs.iter().position(|x| *x == r).unwrap();
    let cmap = |c: usize| cs.iter().position(|x| *x == c).unwrap();
    let coord = |rc: RC| format!("({}, {})", cmap(rc.1), rmap(rc.0));

    let mut lines: Vec<String> = Vec::new();
    let mut sorted_objs: Vec<RC> = obj_order.clone();
    sorted_objs.sort();
    for rc in &sorted_objs {
        let label = ts(&format!("\\displaystyle {}", obj_tex[rc]));
        lines.push(format!("  node({}, mi({})),", coord(*rc), label));
    }

    // floating 2-cells sit at their own grid position, interpolated
    // between the occupied rows/columns
    let interp = |sorted: &[usize], v: usize| -> f64 {
        if let Some(i) = sorted.iter().position(|x| *x == v) {
            return i as f64;
        }
        let i = sorted.iter().filter(|x| **x < v).count();
        if i == 0 {
            0.0
        } else if i == sorted.len() {
            (sorted.len() - 1) as f64
        } else {
            let lo = sorted[i - 1] as f64;
            let hi = sorted[i] as f64;
            (i - 1) as f64 + (v as f64 - lo) / (hi - lo)
        }
    };
    for (r, c, tex) in &floats {
        let x = interp(&cs, *c);
        let y = interp(&rs, *r);
        lines.push(format!(
            "  node(({:.2}, {:.2}), mi({})),",
            x,
            y,
            ts(tex)
        ));
    }
    for (r, c, below) in &vdots_flush {
        let x = interp(&cs, *c);
        let y = if *below {
            interp(&rs, r + 1) - 0.42
        } else {
            interp(&rs, r - 1) + 0.42
        };
        lines.push(format!(
            "  node(({:.2}, {:.2}), mi(\"\\\\vdots\")),",
            x, y
        ));
    }

    // parallel vertical arrows: explicit coordinate endpoints at the
    // column center (fractional y controls extent, so nodes cannot
    // un-clip them) separated by absolute pt shifts (consistent gap
    // regardless of column width)
    for (a0, b0, pc, off) in &vvlines {
        let x = cmap(a0.1) as f64;
        let ytop = rmap(a0.0) as f64 + 0.30;
        let ybot = rmap(b0.0) as f64 - 0.30;
        let up = cget(pc, "dir") == Some("u");
        let (y1, y2) = if up { (ybot, ytop) } else { (ytop, ybot) };
        // fletcher +shift is left-of-travel: screen-east for a down
        // edge, screen-west for an up edge
        let shift = if up { -*off } else { *off };
        let mark = edge_mark(pc);
        let mut args = vec![
            format!("({:.2}, {:.2})", x, y1),
            format!("({:.2}, {:.2})", x, y2),
            format!("\"{}\"", mark),
            format!("shift: {:.1}pt", shift),
            "snap-to: (none, none)".to_string(),
        ];
        for (key, side) in [("west", "left"), ("east", "right")] {
            if let Some(l) = cget(pc, key) {
                // label-side is relative to travel: heading DOWN, the
                // travel-left is screen-east — flip for down edges
                let side = if up {
                    side
                } else if side == "left" {
                    "right"
                } else {
                    "left"
                };
                args.push(format!("label: text(0.75em, mi({}))", ts(l)));
                args.push(format!("label-side: {}", side));
                args.push("label-sep: 0.1em".to_string());
                break;
            }
        }
        lines.push(format!("  edge({}),", args.join(", ")));
    }

    let max_x = cs.len() - 1;
    for (a, b, cell) in edges.iter() {
        let mut cell = cell.clone();
        if parts(&cell).is_some() {
            let dir = cget(&cell, "dir").unwrap().to_string();
            let (left, right) = if dir == "l" { (*b, *a) } else { (*a, *b) };
            // a relation base between the pair (\simeq_Q) floats midway
            if let Some(mid) = cget(&cell, "mid") {
                lines.push(format!(
                    "  node(({:.2}, {:.2}), mi({})),",
                    (cmap(a.1) as f64 + cmap(b.1) as f64) / 2.0,
                    rmap(a.0) as f64,
                    ts(mid)
                ));
            }
            let subs = parts(&cell).unwrap();
            let np = subs.len() as i32;
            for (i, sub) in subs.iter().enumerate() {
                let sdir = cget(sub, "dir").unwrap();
                let (aa, bb) = if matches!(sdir, "r" | "lr" | "~") {
                    (left, right)
                } else {
                    (right, left)
                };
                let mark = edge_mark(sub);
                let mut args = vec![coord(aa), coord(bb), format!("\"{}\"", mark)];
                let label = cget(sub, "above").or_else(|| cget(sub, "below"));
                if let Some(label) = label {
                    let want = if i == 0 { "above" } else { "below" };
                    args.push(format!("label: text(0.75em, mi({}))", ts(label)));
                    args.push(format!("label-side: {}", label_side(sdir, want)));
                    args.push("label-sep: 0.35em".to_string());
                }
                // screen offset top-to-bottom; fletcher shift is
                // relative to travel direction, so flip for leftward
                let screen = (np - 1 - 2 * i as i32) * 3;
                let shift = if sdir == "l" { -screen } else { screen };
                args.push(format!("shift: {}pt", shift));
                lines.push(format!("  edge({}),", args.join(", ")));
            }
            continue;
        }
        let mark = edge_mark(&cell);
        let mut args = vec![coord(*a), coord(*b), format!("\"{}\"", mark)];
        if let Some(sh) = cget(&cell, "vshift") {
            args.push(format!("shift: {}pt", sh));
        }
        let mut placement = ["above", "below", "east", "west"]
            .iter()
            .find(|p| cget(&cell, p).is_some())
            .map(|p| p.to_string());
        let ck = kind(&cell).to_string();
        let cdir = cget(&cell, "dir").unwrap().to_string();
        if ck == "v"
            && cget(&cell, "vshift").is_none()
            && matches!(placement.as_deref(), Some("east") | Some("west"))
            && !(cget(&cell, "east").is_some() && cget(&cell, "west").is_some())
        {
            let x = cmap(a.1);
            let p = placement.clone().unwrap();
            let outward = if x * 2 < max_x {
                "west"
            } else if x * 2 > max_x {
                "east"
            } else {
                p.as_str()
            };
            if outward != p {
                let v = cget(&cell, &p).unwrap().to_string();
                grid::unset(&mut cell, &p);
                grid::set(&mut cell, if outward == "west" { "west" } else { "east" }, v);
                placement = Some(outward.to_string());
            }
        }
        let mut second: Option<String> = None;
        if cdir == "~" {
            if cget(&cell, "cmd") == Some("=") {
                args[2] = "\"=\"".to_string();
                if let Some(p) = &placement {
                    args.push(format!(
                        "label: text(0.75em, mi({}))",
                        ts(cget(&cell, p).unwrap())
                    ));
                    args.push(format!("label-side: {}", label_side("~", p)));
                }
            } else {
                let sym = tilde_label(cget(&cell, "cmd").unwrap_or(""));
                args.push(format!("label: text(0.75em, mi({}))", ts(sym)));
            }
        } else if let Some(p) = &placement {
            let label = cget(&cell, p).unwrap().to_string();
            if matches!(label.trim(), "\\simeq" | "\\cong" | "=" | "\\approx") {
                args.push(format!("label: text(0.9em, mi({}))", ts(&label)));
                args.push("label-sep: 0.25em".to_string());
            } else {
                args.push(format!("label: text(0.75em, mi({}))", ts(&label)));
                args.push("label-sep: 0.1em".to_string());
            }
            args.push(format!("label-side: {}", label_side(&cdir, p)));
            let dx = (cmap(a.1) as i32 - cmap(b.1) as i32).abs();
            let dy = (rmap(a.0) as i32 - rmap(b.0) as i32).abs();
            if ck == "d" && dx.max(dy) >= 2 {
                args.push("label-pos: 0.3".to_string());
            }
            second = ["above", "below", "east", "west"]
                .iter()
                .find(|q| **q != p.as_str() && cget(&cell, q).is_some())
                .map(|q| q.to_string());
        }
        if matches!(
            cget(&cell, "cmd"),
            Some("rightrightarrows") | Some("leftleftarrows")
        ) {
            lines.push(format!("  edge({}, shift: 2pt),", args.join(", ")));
            let args2: Vec<String> = args
                .iter()
                .filter(|a| !a.starts_with("label"))
                .cloned()
                .collect();
            lines.push(format!("  edge({}, shift: -2pt),", args2.join(", ")));
            continue;
        }
        lines.push(format!("  edge({}),", args.join(", ")));
        if let Some(sec) = second {
            lines.push(format!(
                "  edge({}, {}, \"-\", stroke: none, label: text(0.75em, mi({})), label-sep: 0.1em, label-side: {}),",
                coord(*a),
                coord(*b),
                ts(cget(&cell, &sec).unwrap()),
                label_side(&cdir, &sec)
            ));
        }
    }

    (
        "ok".into(),
        Some(format!(
            "#diagram(\n  spacing: (2.6em, 2.2em),\n{}\n)\n",
            lines.join("\n")
        )),
    )
}

pub(crate) fn classify(grid: &[Vec<Cell>]) -> String {
    if signature_rows(grid).is_some() {
        return "signature".into();
    }
    if !grid
        .iter()
        .flatten()
        .any(|c| matches!(kind(c), "h" | "v" | "d" | "dd"))
    {
        return "table".into();
    }
    let kinds: Vec<String> = grid
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| if kind(c) == "e" { " ".to_string() } else { kind(c).to_string() })
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    let flat = kinds.join("");
    let rows = grid.len();
    let cols = grid.iter().map(|r| r.len()).max().unwrap_or(0);
    if rows == 1 {
        return "row".into();
    }
    if cols == 1 {
        return "column".into();
    }
    if flat.contains('d') {
        let n_obj = flat.matches('o').count();
        return if n_obj == 3 { "triangle" } else { "diagonal-grid" }.into();
    }
    let n_obj_rows = kinds.iter().filter(|k| k.contains('o')).count();
    if n_obj_rows == 2 && flat.matches('o').count() == 4 {
        return "square".into();
    }
    if cols <= 3 { "ladder" } else { "grid" }.into()
}

// ----------------------------------------------- wrapped/no-array path

static DIAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\\[sn][ew][aA]rrow").unwrap());
static SPACING_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(\\qquad|\\quad|\\;|\\,|\\!|\\phantom\{[^{}]*\}|[\s.,;:])*$")
        .unwrap()
});
static SEP_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^(\\qquad|\\quad|\\;|\\,|\\!|\\phantom\{[^{}]*\}|[\s.,;:])*(=|\\simeq|\\cong|\\text\{[^{}]{1,24}\})?(\\qquad|\\quad|\\;|\\,|\\!|\\phantom\{[^{}]*\}|[\s.,;:])*$",
    )
    .unwrap()
});

pub(crate) fn emit_equation_pub(tex: &str) -> String {
    emit_equation(tex)
}

fn emit_equation(tex: &str) -> String {
    let mut tex = tex.to_string();
    while let Some((start, end, body)) = grid::find_array(&tex) {
        tex = format!(
            "{}\\begin{{matrix}}{}\\end{{matrix}}{}",
            &tex[..start],
            body,
            &tex[end..]
        );
    }
    format!("#mitex({})\n", ts(&tex))
}

/// The whole per-formula policy of the Python main(): returns
/// (class, status, code). Statuses match the typst table; formulas the
/// Python never stores a row for get status "-".
pub(crate) fn emit_formula(tex: &str) -> (String, String, Option<String>) {
    match grid::parse_formula_grid(tex) {
        Ok(g) => {
            let (status, body) = emit(&g);
            let code = body.map(|b| localize_calls(format!("{}{}", preamble(), b)));
            (classify(&g), status, code)
        }
        Err(status) if status == "wrapped" || status == "no-array" => {
            let (cls, status, body) = wrapped_path(tex);
            let code = body.map(|b| localize_calls(format!("{}{}", preamble(), b)));
            (cls, status, code)
        }
        Err(status) => ("-".into(), format!("-{}", status), None),
    }
}

/// `\begin{matrix}` abused as a commutative diagram (vertical arrows in
/// cells) becomes `\array{..}` so the fletcher pipeline picks it up; a
/// grid the parser rejects is converted back by emit_equation.
fn matrix_diagrams_to_arrays(tex: &str) -> String {
    static MATRIX_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?s)\\begin\{matrix\}(.*?)\\end\{matrix\}").unwrap()
    });
    static VARROW_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\\(d|D|u|U)(own|p)arrow|\\[sn][ew]arrow").unwrap()
    });
    MATRIX_RE
        .replace_all(tex, |c: &regex::Captures| {
            if VARROW_RE.is_match(&c[1]) {
                format!("\\array{{{}}}", &c[1])
            } else {
                c[0].to_string()
            }
        })
        .to_string()
}

/// The typst body (no preamble) for a display formula, when the diagram
/// pipeline can handle it; None means "render as plain math".
pub(crate) fn emit_formula_body(tex: &str) -> Option<String> {
    let tex = &matrix_diagrams_to_arrays(tex);
    match grid::parse_formula_grid(tex) {
        Ok(g) => emit(&g).1,
        Err(status) if status == "wrapped" || status == "no-array" => wrapped_path(tex).2,
        Err(_) => None,
    }
}

fn wrapped_path(tex: &str) -> (String, String, Option<String>) {
    // find all top-level arrays
    let mut spans: Vec<(usize, usize, String)> = Vec::new();
    let mut pos = 0usize;
    while let Some((s, e, body)) = grid::find_array(&tex[pos..]) {
        spans.push((pos + s, pos + e, body));
        pos += e;
    }
    if !spans.is_empty() {
        let mut gaps: Vec<&str> = Vec::new();
        gaps.push(&tex[..spans[0].0]);
        for i in 0..spans.len() - 1 {
            gaps.push(&tex[spans[i].1..spans[i + 1].0]);
        }
        gaps.push(&tex[spans[spans.len() - 1].1..]);
        let inner_ok: Vec<Option<regex::Captures>> = gaps[1..gaps.len() - 1]
            .iter()
            .map(|g| SEP_RE.captures(g))
            .collect();
        if SPACING_RE.is_match(gaps[0])
            && SPACING_RE.is_match(gaps[gaps.len() - 1])
            && inner_ok.iter().all(|m| m.is_some())
        {
            let grids: Vec<Option<Vec<Vec<Cell>>>> = spans
                .iter()
                .map(|(_, _, b)| grid::parse_body_grid(b))
                .collect();
            if grids.iter().all(|g| g.is_some()) {
                let grids: Vec<Vec<Vec<Cell>>> = grids.into_iter().map(|g| g.unwrap()).collect();
                let results: Vec<(String, Option<String>)> =
                    grids.iter().map(|g| emit(g)).collect();
                if results.iter().all(|(st, _)| st == "ok") {
                    let mut cells =
                        vec![format!("  [{}],", results[0].1.as_ref().unwrap().trim())];
                    for (m2, (_, b)) in inner_ok.iter().zip(results[1..].iter()) {
                        if let Some(sep) = m2.as_ref().unwrap().get(2) {
                            cells.push(format!("  mi({}),", ts(sep.as_str())));
                        }
                        cells.push(format!("  [{}],", b.as_ref().unwrap().trim()));
                    }
                    let code = format!(
                        "#grid(columns: {}, column-gutter: 2em, align: horizon,\n{}\n)\n",
                        cells.len(),
                        cells.join("\n")
                    );
                    let cls = if grids.len() == 1 {
                        classify(&grids[0])
                    } else {
                        "multi-diagram".to_string()
                    };
                    return (cls, "ok".into(), Some(code));
                }
            }
        }
    }
    if spans.iter().any(|(_, _, b)| DIAG_RE.is_match(b)) {
        return ("wrapped-diagram".into(), "wrapped-diagram".into(), None);
    }
    ("equation".into(), "ok".into(), Some(emit_equation(tex)))
}
