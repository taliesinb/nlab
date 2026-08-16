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
    let b: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < b.len() {
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
        if b.get(i) == Some(&'[') {
            let mut j = i + 1;
            while j < b.len() && b[j] != ']' {
                dirs.push(b[j]);
                j += 1;
            }
            i = j + 1;
        }
        // labels: (^|_|,|) [-] then {group} | \command | single char
        while let Some(&m) = b.get(i) {
            if m != '^' && m != '_' && m != '|' {
                break;
            }
            i += 1;
            if b.get(i) == Some(&'-') {
                i += 1; // "at midpoint" position modifier: our default
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
            let quoted = tex.replace('"', "");
            labels.push(match m {
                '_' => format!("\"{{{}}}\"{{swap}}", quoted),
                '|' => format!("\"{{{}}}\"{{description}}", quoted),
                _ => format!("\"{{{}}}\"", quoted),
            });
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
    if s.contains("(->") || s.contains("(-->") {
        return if s.starts_with('_') {
            vec!["hook'".into()]
        } else {
            vec!["hook".into()]
        };
    }
    match s.as_str() {
        "=" => vec!["equals".into()],
        "=>" => vec!["Rightarrow".into()],
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
    let n: f64 = rest
        .trim_end_matches("pc")
        .trim_end_matches("ex")
        .trim_end_matches("em")
        .trim()
        .parse()
        .unwrap_or(1.0);
    let deg = (12.0 + 16.0 * n).clamp(20.0, 75.0);
    Some(format!("bend {}={}", side, deg as i64))
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
