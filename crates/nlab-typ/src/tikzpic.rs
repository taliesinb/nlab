//! Minimal tikzpicture -> CeTZ converter.
//!
//! Covers the vocabulary of the nLab's embedded pictures (the braid
//! group diagrams and friends): straight `to`/`--` lines, cubic
//! `.. controls .. and .. ..` Béziers (including the white-underlay
//! crossing trick), `node {...}` / `\node at (..) {...}` text,
//! `scope[shift/scale/color/line width]` groups with proper affine
//! composition, and env-level `xscale/yscale/scale/line width`.
//! Anything else fails the statement (collected as a warning) rather
//! than the picture.

use crate::emit;

/// Affine transform + inherited style for a scope.
#[derive(Clone)]
struct Frame {
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
    color: String,
    width: f64,
}

impl Frame {
    fn apply(&self, p: (f64, f64)) -> (f64, f64) {
        (self.tx + self.sx * p.0, self.ty + self.sy * p.1)
    }
    /// Child frame: parent ∘ translate(shift) ∘ scale(s).
    fn child(&self, shift: (f64, f64), scale: (f64, f64)) -> Frame {
        Frame {
            tx: self.tx + self.sx * shift.0,
            ty: self.ty + self.sy * shift.1,
            sx: self.sx * scale.0,
            sy: self.sy * scale.1,
            color: self.color.clone(),
            width: self.width,
        }
    }
}

const COLORS: [&str; 9] = [
    "white", "black", "red", "blue", "green", "orange", "gray", "purple", "olive",
];

/// Parse a tikz option list into (shift, scale, color, width) deltas.
fn parse_opts(opts: &str) -> ((f64, f64), (f64, f64), Option<String>, Option<f64>) {
    let mut shift = (0.0, 0.0);
    let mut scale = (1.0, 1.0);
    let mut color = None;
    let mut width = None;
    // split on commas not inside {...}
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in opts.trim_matches(|c| c == '[' || c == ']').chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    parts.push(cur);
    for o in parts {
        let o = o.trim().replace(' ', "");
        if let Some(v) = o.strip_prefix("shift=") {
            if let Some(p) = parse_pair(v) {
                shift = p;
            }
        } else if let Some(v) = o.strip_prefix("xscale=") {
            scale.0 *= v.parse().unwrap_or(1.0);
        } else if let Some(v) = o.strip_prefix("yscale=") {
            scale.1 *= v.parse().unwrap_or(1.0);
        } else if let Some(v) = o.strip_prefix("scale=") {
            let f: f64 = v.parse().unwrap_or(1.0);
            scale.0 *= f;
            scale.1 *= f;
        } else if let Some(v) = o.strip_prefix("linewidth=") {
            width = v.trim_end_matches("pt").parse().ok();
        } else if COLORS.contains(&o.as_str()) {
            color = Some(o);
        }
    }
    (shift, scale, color, width)
}

/// Convert a `\begin{tikzpicture}...\end{tikzpicture}` body to a typst
/// `box(cetz.canvas(...))` expression. Returns None if nothing usable
/// could be parsed.
pub(crate) fn to_cetz(body: &str, env_opts: &str) -> Option<(String, Vec<String>)> {
    let (_, scale, color, width) = parse_opts(env_opts);
    let root = Frame {
        sx: scale.0,
        sy: scale.1,
        tx: 0.0,
        ty: 0.0,
        color: color.unwrap_or_else(|| "black".into()),
        width: width.unwrap_or(0.4),
    };
    let b: Vec<char> = body.chars().collect();
    let mut i = 0usize;
    let mut out: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut frames: Vec<Frame> = vec![root];

    let starts = |b: &[char], i: usize, w: &str| {
        b[i..].iter().take(w.len()).collect::<String>() == w
    };

    while i < b.len() {
        if starts(&b, i, "\\begin{scope}") {
            i += "\\begin{scope}".len();
            let mut opts = String::new();
            if b.get(i) == Some(&'[') {
                let mut j = i;
                while j < b.len() && b[j] != ']' {
                    j += 1;
                }
                opts = b[i..=j.min(b.len() - 1)].iter().collect();
                i = j + 1;
            }
            let (shift, scale, color, width) = parse_opts(&opts);
            let mut f = frames.last().unwrap().child(shift, scale);
            if let Some(c) = color {
                f.color = c;
            }
            if let Some(w) = width {
                f.width = w;
            }
            frames.push(f);
            continue;
        }
        if starts(&b, i, "\\end{scope}") {
            i += "\\end{scope}".len();
            if frames.len() > 1 {
                frames.pop();
            }
            continue;
        }
        if starts(&b, i, "\\draw") || starts(&b, i, "\\node") {
            let is_node_stmt = starts(&b, i, "\\node");
            i += 5;
            let frame = frames.last().unwrap().clone();
            let mut width = frame.width;
            let mut color = frame.color.clone();
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            if b.get(i) == Some(&'[') {
                let mut j = i;
                while j < b.len() && b[j] != ']' {
                    j += 1;
                }
                let opts: String = b[i..=j.min(b.len() - 1)].iter().collect();
                let (_, _, c, w) = parse_opts(&opts);
                if let Some(c) = c {
                    color = c;
                }
                if let Some(w) = w {
                    width = w;
                }
                i = j + 1;
            }
            // statement: until ';' at brace depth 0
            let mut j = i;
            let mut depth = 0i32;
            while j < b.len() && (b[j] != ';' || depth > 0) {
                match b[j] {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            let stmt: String = b[i..j].iter().collect();
            i = j + 1;
            let stmt = if is_node_stmt {
                // \node at (x,y) {body} -> (x,y) node {body}
                match stmt.trim().strip_prefix("at") {
                    Some(rest) => {
                        let rest = rest.trim_start();
                        if let Some(k) = rest.find(')') {
                            format!("{} node {}", &rest[..=k], &rest[k + 1..])
                        } else {
                            stmt
                        }
                    }
                    None => stmt,
                }
            } else {
                stmt
            };
            match parse_path(&stmt, &frame, width, &color) {
                Ok(mut ops) => out.append(&mut ops),
                Err(e) => warnings.push(e),
            }
            continue;
        }
        i += 1;
    }
    if out.is_empty() {
        return None;
    }
    let code = format!(
        "box(baseline: 42%, cetz.canvas(length: 19pt, {{\n    import cetz.draw: *\n    {}\n  }}))",
        out.join("\n    ")
    );
    Some((code, warnings))
}

fn parse_pair(s: &str) -> Option<(f64, f64)> {
    let s = s.trim().trim_start_matches('{').trim_start_matches('(');
    let inner: String = s.chars().take_while(|c| *c != ')').collect();
    let mut it = inner.split(',');
    let x: f64 = it.next()?.trim().parse().ok()?;
    let y: f64 = it.next()?.trim().parse().ok()?;
    Some((x, y))
}

fn fmt_pt(p: (f64, f64)) -> String {
    format!("({:.3}, {:.3})", p.0, p.1)
}

fn parse_path(
    path: &str,
    frame: &Frame,
    width: f64,
    color: &str,
) -> Result<Vec<String>, String> {
    let stroke = format!("stroke: {}pt + {}", width, color);
    let b: Vec<char> = path.chars().collect();
    let mut i = 0usize;
    let mut ops = Vec::new();
    let mut last: Option<(f64, f64)> = None;

    let read_coord = |b: &[char], mut i: usize| -> Option<((f64, f64), usize)> {
        while i < b.len() && b[i].is_whitespace() {
            i += 1;
        }
        if b.get(i) != Some(&'(') {
            return None;
        }
        let mut j = i + 1;
        while j < b.len() && b[j] != ')' {
            j += 1;
        }
        let s: String = b[i + 1..j].iter().collect();
        parse_pair(&format!("({s})")).map(|p| (frame.apply(p), j + 1))
    };

    while i < b.len() {
        while i < b.len() && b[i].is_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] == '(' {
            let (c, ni) = read_coord(&b, i).ok_or("bad coordinate")?;
            last = Some(c);
            i = ni;
            continue;
        }
        let rest: String = b[i..].iter().collect();
        if rest.starts_with("to") || rest.starts_with("--") {
            i += 2;
            let (c, ni) = read_coord(&b, i).ok_or("line without target")?;
            let from = last.ok_or("line without start")?;
            ops.push(format!("line({}, {}, {})", fmt_pt(from), fmt_pt(c), stroke));
            last = Some(c);
            i = ni;
            continue;
        }
        if rest.starts_with("..") {
            // .. controls (a) and (b) .. (end)
            i += 2;
            let rest2: String = b[i..].iter().collect();
            let rest2 = rest2.trim_start();
            if !rest2.starts_with("controls") {
                return Err("unsupported .. path".into());
            }
            i += b[i..].iter().collect::<String>().find("controls").unwrap() + 8;
            let (c1, ni) = read_coord(&b, i).ok_or("bezier c1")?;
            i = ni;
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            let rest3: String = b[i..].iter().collect();
            if rest3.trim_start().starts_with("and") {
                i += rest3.find("and").unwrap() + 3;
            }
            let (c2, ni) = read_coord(&b, i).ok_or("bezier c2")?;
            i = ni;
            // consume trailing '..'
            while i < b.len() && (b[i].is_whitespace() || b[i] == '.') {
                i += 1;
            }
            let (e, ni) = read_coord(&b, i).ok_or("bezier end")?;
            let s0 = last.ok_or("bezier without start")?;
            ops.push(format!(
                "bezier({}, {}, {}, {}, {})",
                fmt_pt(s0),
                fmt_pt(e),
                fmt_pt(c1),
                fmt_pt(c2),
                stroke
            ));
            last = Some(e);
            i = ni;
            continue;
        }
        if rest.starts_with("node") {
            i += 4;
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            if b.get(i) == Some(&'[') {
                while i < b.len() && b[i] != ']' {
                    i += 1;
                }
                i += 1;
                while i < b.len() && b[i].is_whitespace() {
                    i += 1;
                }
            }
            if b.get(i) != Some(&'{') {
                return Err("node without body".into());
            }
            let mut depth = 0i32;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            let body: String = b[i + 1..j].iter().collect();
            i = j + 1;
            let at = last.ok_or("node without position")?;
            let default_fill = if color != "black" { Some(color) } else { None };
            ops.push(format!(
                "content({}, {})",
                fmt_pt(at),
                node_content(&body, default_fill)
            ));
            continue;
        }
        // unknown token: skip a char (tolerate stray text)
        i += 1;
    }
    if ops.is_empty() {
        Err("empty path".into())
    } else {
        Ok(ops)
    }
}

/// Peel `\color{..}` / `\scalebox{f}{..}` wrappers (any order, any
/// nesting) around a `$math$` node body. `default_fill` is the scope's
/// inherited color (tikz nodes inherit the ambient draw color).
fn node_content(body: &str, default_fill: Option<&str>) -> String {
    let mut fill: Option<String> = None;
    let mut size = 1.0f64;
    let mut s = body.trim().to_string();
    fn balanced(b: &[char], start: usize) -> Option<(String, usize)> {
        if b.get(start) != Some(&'{') {
            return None;
        }
        let mut depth = 0i32;
        for (k, c) in b.iter().enumerate().skip(start) {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((b[start + 1..k].iter().collect(), k + 1));
                    }
                }
                _ => {}
            }
        }
        None
    }
    loop {
        s = s.trim().to_string();
        // strip a redundant outer brace group
        let b: Vec<char> = s.chars().collect();
        if b.first() == Some(&'{') {
            if let Some((inner, end)) = balanced(&b, 0) {
                if end == b.len() {
                    s = inner;
                    continue;
                }
            }
        }
        if let Some(rest) = s.strip_prefix("\\color") {
            let rb: Vec<char> = rest.trim_start().chars().collect();
            if let Some((name, end)) = balanced(&rb, 0) {
                fill = Some(name.trim().to_string());
                s = rb[end..].iter().collect();
                continue;
            }
        }
        if let Some(rest) = s.strip_prefix("\\scalebox") {
            let rb: Vec<char> = rest.trim_start().chars().collect();
            if let Some((f, end)) = balanced(&rb, 0) {
                if let Ok(factor) = f
                    .trim()
                    .parse::<f64>()
                    .or_else(|_| format!("0{}", f.trim()).parse::<f64>())
                {
                    // negative factors mirror in LaTeX (used to cancel a
                    // mirrored ambient transform); we never mirror text
                    size *= factor.abs();
                }
                let mut k = end;
                while rb.get(k).map(|c| c.is_whitespace()).unwrap_or(false) {
                    k += 1;
                }
                if let Some((inner, _)) = balanced(&rb, k) {
                    s = inner;
                    continue;
                }
            }
        }
        break;
    }
    let math = s.trim().trim_start_matches('$').trim_end_matches('$');
    let mut inner = format!("mi({})", emit::ts(math));
    if (size - 1.0).abs() > 0.01 {
        inner = format!("text({:.2}em, {})", size, inner);
    }
    if let Some(f) = fill.or_else(|| default_fill.map(String::from)) {
        inner = format!("text(fill: {}, {})", f, inner);
    }
    inner
}
