//! xypic `\begin{xymatrix}` -> tikzcd source translation.
//!
//! The nLab's xymatrix vocabulary (surveyed over the whole corpus) is
//! a strict subset that maps token-for-token onto tikzcd, which our
//! fletcher emitter already handles: cells split identically on `&` /
//! `\\`, and only the `\ar` decorations differ. Anything outside the
//! survey passes through unchanged (and will surface in the leak scan
//! rather than corrupt the page).

/// Translate an xymatrix body to a tikzcd body.
pub(crate) fn xymatrix_to_tikzcd(body: &str) -> String {
    // plain-TeX row separators
    let body = body.replace("\\cr", "\\\\");
    let b: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    let mut twocell_n = 0usize;
    while i < b.len() {
        // \rtwocell^f_g{a} and friends: a bent arrow pair with a 2-cell
        if b[i] == '\\' {
            let word: String = b[i + 1..].iter().take(9).collect();
            let (dir, wlen) = match word.as_str() {
                w if w.starts_with("drtwocell") => (Some("dr"), 10),
                w if w.starts_with("urtwocell") => (Some("ur"), 10),
                w if w.starts_with("dltwocell") => (Some("dl"), 10),
                w if w.starts_with("ultwocell") => (Some("ul"), 10),
                w if w.starts_with("rtwocell") => (Some("r"), 9),
                w if w.starts_with("ltwocell") => (Some("l"), 9),
                w if w.starts_with("dtwocell") => (Some("d"), 9),
                w if w.starts_with("utwocell") => (Some("u"), 9),
                _ => (None, 0),
            };
            if let Some(dir) = dir {
                i += wlen;
                // optional <width> spec: \drtwocell<5.5>...
                while i < b.len() && b[i].is_whitespace() {
                    i += 1;
                }
                if b.get(i) == Some(&'<') {
                    while i < b.len() && b[i] != '>' {
                        i += 1;
                    }
                    i += 1;
                }
                let (top, bot, center, ni) = twocell_labels(&b, i);
                i = ni;
                twocell_n += 1;
                let (ta, tb) = (
                    format!("xy2a{}", twocell_n),
                    format!("xy2b{}", twocell_n),
                );
                out.push_str(&format!(
                    "\\ar[{d}, bend left=15, \"{{{top}}}\"{{name={ta}}}]\
\\ar[{d}, bend right=15, \"{{{bot}}}\"{{swap, name={tb}}}]\
\\ar[Rightarrow, from={ta}, to={tb}, \"{{{center}}}\"]",
                    d = dir,
                    top = top,
                    bot = bot,
                    ta = ta,
                    tb = tb,
                    center = center
                ));
                continue;
            }
        }
        let is_ar = b[i] == '\\'
            && b[i + 1..].iter().take(2).collect::<String>() == "ar"
            && matches!(b.get(i + 3), Some('@') | Some('[') | Some('('));
        if !is_ar {
            out.push(b[i]);
            i += 1;
            continue;
        }
        i += 3; // past \ar
        let mut opts: Vec<String> = Vec::new();
        let mut labels: Vec<String> = Vec::new();
        let mut dirs = String::new();
        // decorations: @{style} @/curve/ @<shift> @(out,in)
        while b.get(i) == Some(&'@') {
            i += 1;
            if b.get(i).map(|c| c.is_ascii_digit()).unwrap_or(false) {
                // @2 / @3: line multiplicity; an optional {style} follows
                i += 1;
                if b.get(i) == Some(&'{') {
                    let (style, end) = balanced(&b, i, '{', '}');
                    i = end;
                    for o in style_opts(&style) {
                        if o != "Rightarrow" {
                            opts.push(o);
                        }
                    }
                }
                opts.push("Rightarrow".into());
                continue;
            }
            match b.get(i) {
                Some('{') => {
                    let (style, end) = balanced(&b, i, '{', '}');
                    i = end;
                    opts.extend(style_opts(&style));
                }
                Some('/') => {
                    let mut j = i + 1;
                    while j < b.len() && b[j] != '/' {
                        j += 1;
                    }
                    let spec: String = b[i + 1..j].iter().collect();
                    i = j + 1;
                    if let Some(o) = curve_opt(&spec) {
                        opts.push(o);
                    }
                }
                Some('<') => {
                    let mut j = i + 1;
                    while j < b.len() && b[j] != '>' {
                        j += 1;
                    }
                    let spec: String = b[i + 1..j].iter().collect();
                    i = j + 1;
                    if let Some(o) = shift_opt(&spec) {
                        opts.push(o);
                    }
                }
                Some('(') => {
                    let mut j = i + 1;
                    while j < b.len() && b[j] != ')' {
                        j += 1;
                    }
                    let spec: String = b[i + 1..j].iter().collect();
                    i = j + 1;
                    if let Some((o, inn)) = loop_opts(&spec) {
                        opts.push(format!("out={}", o));
                        opts.push(format!("in={}", inn));
                        opts.push("loop".into());
                        opts.push("distance=30pt".into());
                    }
                }
                _ => break,
            }
        }
        // explicit anchor targets `"s"; "t"` may appear right after the
        // decorations or after the labels
        let read_anchors = |b: &[char], start: usize| -> Option<(String, String, usize)> {
            let mut i = start;
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            let read_q = |b: &[char], mut i: usize| -> Option<(String, usize)> {
                if b.get(i) != Some(&'"') {
                    return None;
                }
                let mut j = i + 1;
                let mut t = String::new();
                while j < b.len() && b[j] != '"' {
                    t.push(b[j]);
                    j += 1;
                }
                let mut k = j + 1;
                // "name"+(dx,dy): encode the offset into the name so
                // the tikzcd resolver can nudge the anchor point
                let mut out = t;
                while matches!(b.get(k), Some('+') | Some('-'))
                    && b.get(k + 1) == Some(&'(')
                {
                    let neg = b[k] == '-';
                    let start = k + 2;
                    while k < b.len() && b[k] != ')' {
                        k += 1;
                    }
                    let inner: String = b[start..k].iter().collect();
                    k += 1;
                    let mut it = inner.split(',');
                    let dx: f64 = it
                        .next()
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0.0);
                    let dy: f64 = it
                        .next()
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0.0);
                    let sign = if neg { -1.0 } else { 1.0 };
                    out = format!("{}@{}@{}", out, sign * dx, sign * dy);
                }
                Some((out, k))
            };
            let (src_a, ni) = read_q(b, i)?;
            let mut k = ni;
            while k < b.len() && (b[k].is_whitespace() || b[k] == ';') {
                k += 1;
            }
            let (dst_a, ni2) = read_q(b, k)?;
            Some((src_a, dst_a, ni2))
        };
        if b.get(i) == Some(&'[') {
            let mut j = i + 1;
            while j < b.len() && b[j] != ']' {
                dirs.push(b[j]);
                j += 1;
            }
            i = j + 1;
        } else if let Some((sa, ta, ni)) = read_anchors(&b, i) {
            opts.push(format!("from={}", sa));
            opts.push(format!("to={}", ta));
            i = ni;
        }
        // trailing <shift> after the target: \ar@{}[rr]<3ex>^{..}
        if b.get(i) == Some(&'<') {
            let mut j = i + 1;
            while j < b.len() && b[j] != '>' {
                j += 1;
            }
            let spec: String = b[i + 1..j].iter().collect();
            if let Some(o) = shift_opt(&spec) {
                opts.push(o);
                i = j + 1;
            }
        }
        // labels: (^|_|,|) [-<>]* then {group} | \command | single char,
        // optionally suffixed ="name"; xy tolerates whitespace between
        // the arrow and its labels (and between labels)
        loop {
            let mut k = i;
            while k < b.len() && b[k].is_whitespace() {
                k += 1;
            }
            let Some(&m) = b.get(k) else { break };
            if m != '^' && m != '_' && m != '|' {
                break;
            }
            i = k + 1;
            // position modifiers: - midpoint (default), < toward the
            // start, > toward the end (repeats push further)
            let mut toward = 0i32;
            let mut frac: Option<f64> = None;
            while matches!(b.get(i), Some('-') | Some('<') | Some('>')) {
                match b[i] {
                    '<' => toward -= 1,
                    '>' => toward += 1,
                    _ => {}
                }
                i += 1;
            }
            // fractional position: ^(0.25){label}
            if b.get(i) == Some(&'(') {
                let mut j = i + 1;
                let mut t = String::new();
                while j < b.len() && b[j] != ')' {
                    t.push(b[j]);
                    j += 1;
                }
                if let Ok(v) = t.trim().parse::<f64>() {
                    frac = Some(v);
                    i = j + 1;
                }
            }
            let tex = if b.get(i) == Some(&'{') {
                let (t, end) = balanced(&b, i, '{', '}');
                i = end;
                t
            } else if b.get(i) == Some(&'\\') {
                let mut t = String::from('\\');
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_alphabetic() {
                    t.push(b[j]);
                    j += 1;
                }
                i = j;
                t
            } else if let Some(&c) = b.get(i) {
                i += 1;
                c.to_string()
            } else {
                break;
            };
            // ="name" names the label as an anchor
            let mut name = None;
            if b.get(i) == Some(&'=') && b.get(i + 1) == Some(&'"') {
                let mut j = i + 2;
                let mut t = String::new();
                while j < b.len() && b[j] != '"' {
                    t.push(b[j]);
                    j += 1;
                }
                i = j + 1;
                name = Some(t);
            }
            let quoted = tex.replace('"', "");
            let mut mods: Vec<String> = Vec::new();
            match m {
                '_' => mods.push("swap".into()),
                '|' => mods.push("description".into()),
                _ => {}
            }
            match frac {
                Some(v) if v <= 0.15 => mods.push("very near start".into()),
                Some(v) if v <= 0.4 => mods.push("near start".into()),
                Some(v) if v >= 0.85 => mods.push("very near end".into()),
                Some(v) if v >= 0.6 => mods.push("near end".into()),
                Some(_) => {}
                None => match toward {
                    t if t <= -2 => mods.push("very near start".into()),
                    -1 => mods.push("near start".into()),
                    1 => mods.push("near end".into()),
                    t if t >= 2 => mods.push("very near end".into()),
                    _ => {}
                },
            }
            if let Some(n) = name {
                mods.push(format!("name={}", n));
            }
            labels.push(if mods.is_empty() {
                format!("\"{{{}}}\"", quoted)
            } else {
                format!("\"{{{}}}\"{{{}}}", quoted, mods.join(", "))
            });
        }
        if !opts.iter().any(|o| o.starts_with("from=")) {
            if let Some((sa, ta, ni)) = read_anchors(&b, i) {
                opts.push(format!("from={}", sa));
                opts.push(format!("to={}", ta));
                i = ni;
            }
        }
        // description labels on a phantom carrier just sit on the line
        if opts.iter().any(|o| o == "phantom") {
            for l in &mut labels {
                if let Some(stripped) = l.strip_suffix("{description}") {
                    *l = stripped.to_string();
                }
            }
        }
        let mut parts: Vec<String> = Vec::new();
        if !dirs.is_empty() {
            parts.push(dirs);
        }
        parts.extend(opts);
        parts.extend(labels);
        out.push_str(&format!("\\ar[{}]", parts.join(", ")));
    }
    out
}

/// `^{top}_{bot}{center}` after a twocell macro (labels optional,
/// single tokens allowed); returns (top, bot, center, next index).
fn twocell_labels(b: &[char], mut i: usize) -> (String, String, String, usize) {
    let mut top = String::new();
    let mut bot = String::new();
    let read_tok = |b: &[char], mut i: usize| -> (String, usize) {
        if b.get(i) == Some(&'{') {
            let (t, e) = balanced(b, i, '{', '}');
            (t, e)
        } else if b.get(i) == Some(&'\\') {
            let mut t = String::from('\\');
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_alphabetic() {
                t.push(b[j]);
                j += 1;
            }
            (t, j)
        } else if let Some(&c) = b.get(i) {
            i += 1;
            (c.to_string(), i)
        } else {
            (String::new(), i)
        }
    };
    loop {
        while i < b.len() && b[i].is_whitespace() {
            i += 1;
        }
        match b.get(i) {
            Some('^') => {
                let (t, ni) = read_tok(b, i + 1);
                top = t;
                i = ni;
            }
            Some('_') => {
                let (t, ni) = read_tok(b, i + 1);
                bot = t;
                i = ni;
            }
            Some('{') => {
                let (c, ni) = balanced(b, i, '{', '}');
                return (top, bot, c, ni);
            }
            _ => return (top, bot, String::new(), i),
        }
    }
}

/// Read a balanced group starting at `open` position; returns (inner,
/// index past the close).
fn balanced(b: &[char], start: usize, open: char, close: char) -> (String, usize) {
    let mut depth = 0i32;
    for (k, &c) in b.iter().enumerate().skip(start) {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return (b[start + 1..k].iter().collect(), k + 1);
            }
        }
    }
    (b[start + 1..].iter().collect(), b.len())
}

/// `@{...}` arrow styles.
fn style_opts(style: &str) -> Vec<String> {
    let s: String = style.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return vec!["phantom".into()];
    }
    // hooks arrive as `^{(}->` / `_{(}->`: the hook glyph in braces
    let unbraced: String = s.chars().filter(|c| *c != '{' && *c != '}').collect();
    if unbraced.contains("(->") || unbraced.contains("(-->") {
        return if s.starts_with('_') {
            vec!["hook'".into()]
        } else {
            vec!["hook".into()]
        };
    }
    match s.as_str() {
        "=" => vec!["equals".into()],
        "=>" => vec!["Rightarrow".into()],
        "==>" => vec!["Rightarrow".into(), "dashed".into()],
        _ if s.contains("<-") && s.contains(")") => vec!["<-hook".into()],
        "<<-" => vec!["<<-".into()],
        "<-" => vec!["<-".into()],
        "|->" => vec!["mapsto".into()],
        "->>" => vec!["two heads".into()],
        ".>" => vec!["dotted".into()],
        "-->" => vec!["dashed".into()],
        "->" | "" => vec![],
        _ => vec![], // unknown style: plain arrow beats corrupt output
    }
}

/// `@/^1pc/` curving -> bend.
fn curve_opt(spec: &str) -> Option<String> {
    let s = spec.trim();
    let (side, rest) = match s.chars().next()? {
        '^' => ("left", &s[1..]),
        '_' => ("right", &s[1..]),
        _ => return None,
    };
    let unit = if rest.ends_with("pc") {
        12.0
    } else if rest.ends_with("ex") {
        4.4
    } else if rest.ends_with("em") {
        11.0
    } else {
        1.0
    };
    let n: f64 = rest
        .trim_end_matches("pc")
        .trim_end_matches("ex")
        .trim_end_matches("em")
        .trim_end_matches("pt")
        .trim()
        .parse()
        .unwrap_or(1.0);
    Some(format!("sagitta {}={}pt", side, n * unit))
}

/// `@<+.5ex>` lateral shift -> tikzcd shift left/right (ex ~ 4pt).
fn shift_opt(spec: &str) -> Option<String> {
    let s: String = spec.chars().filter(|c| !c.is_whitespace()).collect();
    let v: f64 = s
        .trim_end_matches("ex")
        .trim_end_matches("pt")
        .parse()
        .ok()?;
    let pt = v.abs() * if s.ends_with("pt") { 1.0 } else { 4.0 };
    Some(if v >= 0.0 {
        format!("shift left={}pt", pt)
    } else {
        format!("shift right={}pt", pt)
    })
}

/// `@(ul,ur)` self-loop anchors -> out/in angles.
fn loop_opts(spec: &str) -> Option<(i32, i32)> {
    let angle = |d: &str| -> Option<i32> {
        Some(match d.trim() {
            "r" => 0,
            "ur" => 45,
            "u" => 90,
            "ul" => 135,
            "l" => 180,
            "dl" => 225,
            "d" => 270,
            "dr" => 315,
            _ => return None,
        })
    };
    let (a, b) = spec.split_once(',')?;
    Some((angle(a)?, angle(b)?))
}
