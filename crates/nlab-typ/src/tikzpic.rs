//! Minimal tikzpicture -> CeTZ converter.
//!
//! Two families of nLab pictures drive the vocabulary: the braid
//! diagrams (straight `to`/`--` lines, cubic Béziers with the
//! white-underlay crossing trick, scope frames with shift/scale,
//! inherited color and line width) and coordinate-graph drawings
//! (named `\node (n) at (x,y) {..}`, edges between named nodes with
//! arrow heads, border trimming, riding labels, bends, filled
//! circles). Statements outside the vocabulary fail individually
//! (collected as warnings) rather than the picture.

use crate::emit;
use std::collections::HashMap;

/// Affine transform + inherited style for a scope.
#[derive(Clone)]
struct Frame {
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
    rot: f64, // radians, applied to coordinates before scaling
    color: String,
    width: f64,
}

impl Frame {
    fn apply(&self, p: (f64, f64)) -> (f64, f64) {
        let (x, y) = if self.rot != 0.0 {
            let (s, c) = self.rot.sin_cos();
            (c * p.0 - s * p.1, s * p.0 + c * p.1)
        } else {
            p
        };
        (self.tx + self.sx * x, self.ty + self.sy * y)
    }
    /// Child frame: parent ∘ translate(shift) ∘ rotate ∘ scale(s).
    fn child(&self, shift: (f64, f64), scale: (f64, f64)) -> Frame {
        Frame {
            tx: self.tx + self.sx * shift.0,
            ty: self.ty + self.sy * shift.1,
            sx: self.sx * scale.0,
            sy: self.sy * scale.1,
            rot: self.rot,
            color: self.color.clone(),
            width: self.width,
        }
    }
}

/// A named node: center plus half-extents, all in canvas units.
#[derive(Clone, Copy)]
struct NamedNode {
    x: f64,
    y: f64,
    hw: f64,
    hh: f64,
    round: bool, // circular outline: wires land exactly on the rim
}

// canvas length per tikz unit: true tikz is 1cm = 28.45pt, which the
// coordinate-graph pictures (named \node statements) are laid out
// for; the braid pictures were calibrated against the server renders
// at a tighter 19pt.
const UNIT_PT: f64 = 19.0;
const UNIT_PT_GRAPH: f64 = 28.45;

const COLORS: [&str; 14] = [
    "white", "black", "red", "blue", "green", "orange", "gray", "purple",
    "olive", "teal", "yellow", "navy", "lime", "maroon",
];

/// Options parsed from a `[...]` list.
#[derive(Default, Clone)]
struct Opts {
    shift: (f64, f64),
    scale: (f64, f64),
    color: Option<String>,
    width: Option<f64>,
    fill: Option<String>,
    arrow_end: bool,
    arrow_start: bool,
    dashed: bool,
    dotted: bool,
    bend: f64, // +left / -right, degrees
    shorten_start: f64,
    shorten_end: f64,
    canvas_shift: (f64, f64),
    place: Option<String>, // above/below/left/right + combos
    xshift: f64,
    yshift: f64,
    rotate: f64,             // degrees
    out_ang: Option<f64>,    // to[out=..] departure angle, degrees
    in_ang: Option<f64>,     // to[in=..] arrival angle, degrees
    underlay: Option<f64>,   // preaction white-line width, pt (crossing gap)
    mid_arrow: bool,         // decoration mark: arrowhead mid-segment
    mid_pos: f64,            // decoration mark position along the path
    mid_rev: bool,           // \arrowreversed: head points backward
    looseness: f64,          // control-point distance multiplier
    latex_tip: bool,         // >=latex / >=stealth: filled arrowheads
    path_only: bool,         // \path: invisible, only decorations show
}

/// tikz color expressions: plain names plus `blue!15` tints.
fn map_color(s: &str) -> Option<String> {
    let s = s.trim();
    fn base_of(b: &str) -> Option<&str> {
        match b {
            "violet" => return Some("purple"),
            "cyan" => return Some("aqua"),
            "magenta" => return Some("fuchsia"),
            "brown" => return Some("maroon"),
            "grey" => return Some("gray"),
            "lightgray" | "lightgrey" => return Some("silver"),
            "darkgray" | "darkgrey" => return Some("gray.darken(35%)"),
            _ => {}
        }
        COLORS.contains(&b).then_some(b)
    }
    if let Some((base, rest)) = s.split_once('!') {
        let base = base_of(base)?;
        let pct: f64 = rest.split('!').next()?.trim().parse().ok()?;
        return Some(format!("{}.lighten({:.0}%)", base, (100.0 - pct).clamp(0.0, 100.0)));
    }
    base_of(s).map(String::from)
}


/// Is this a `.anchor` word (center/north/south east/...)?
fn is_anchor_word(a: &str) -> bool {
    !a.is_empty() && a.chars().all(|c| c.is_alphabetic() || c == ' ')
}

/// Resolve `NAME` or `NAME.anchor` to a node center.
fn node_pt(name: &str, nodes: &HashMap<String, NamedNode>) -> Option<(f64, f64)> {
    let name = name.trim();
    if let Some(n) = nodes.get(name) {
        return Some((n.x, n.y));
    }
    if let Some((base, anchor)) = name.rsplit_once('.') {
        if is_anchor_word(anchor) {
            if let Some(n) = nodes.get(base.trim()) {
                return Some((n.x, n.y));
            }
        }
    }
    None
}

/// Coordinate expressions: plain pairs and polar forms, node names,
/// and calc syntax - `$(A)!0.5!(B)+(0,-0.25)$`, `(centre) + (-1, 1)`.
fn eval_coord(
    expr: &str,
    nodes: &HashMap<String, NamedNode>,
    frame: &Frame,
) -> Option<(f64, f64)> {
    let e: String = expr
        .trim()
        .trim_start_matches('$')
        .trim_end_matches('$')
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let e = e.as_str();
    // linear part of the frame (offsets scale/rotate but don't move)
    let origin = frame.apply((0.0, 0.0));
    let lin = |p: (f64, f64)| {
        let q = frame.apply(p);
        (q.0 - origin.0, q.1 - origin.1)
    };
    // plain coordinate or node?
    if let Some(pt) = node_pt(e, nodes) {
        return Some(pt);
    }
    if !e.contains('(') {
        return parse_pair(&format!("({e})")).map(|p| frame.apply(p));
    }
    // split into top-level chunks around + / - (outside parens)
    let b: Vec<char> = e.chars().collect();
    let mut terms: Vec<(f64, String)> = Vec::new();
    let mut depth = 0i32;
    let mut sign = 1.0f64;
    let mut cur = String::new();
    for &c in &b {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            '+' | '-' if depth == 0 => {
                if !cur.trim().is_empty() {
                    terms.push((sign, std::mem::take(&mut cur)));
                } else {
                    cur.clear();
                }
                sign = if c == '-' { -1.0 } else { 1.0 };
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        terms.push((sign, cur));
    }
    let mut acc: Option<(f64, f64)> = None;
    for (k, (sgn, term)) in terms.iter().enumerate() {
        let t = term.trim();
        // interpolation (A)!f!(B)
        let val: Option<(f64, f64)> = if let Some(bang) = t.find(")!") {
            let a = t[..bang + 1].trim();
            let rest = &t[bang + 2..];
            let bang2 = rest.find('!')?;
            let f: f64 = rest[..bang2].trim().parse().ok()?;
            let bt = rest[bang2 + 1..].trim();
            let pa = eval_coord(a.trim_matches(|c| c == '(' || c == ')'), nodes, frame)
                .or_else(|| node_pt(a.trim_matches(|c| c == '(' || c == ')'), nodes))?;
            let pb = eval_coord(bt.trim_matches(|c| c == '(' || c == ')'), nodes, frame)?;
            Some((pa.0 + f * (pb.0 - pa.0), pa.1 + f * (pb.1 - pa.1)))
        } else {
            let inner = t.trim_matches(|c| c == '(' || c == ')').trim();
            if let Some(pt) = node_pt(inner, nodes) {
                Some(pt)
            } else if let Some(p) = parse_pair(&format!("({inner})")) {
                // absolute for the leading term, vector afterwards
                if k == 0 { Some(frame.apply(p)) } else { Some(lin(p)) }
            } else {
                None
            }
        };
        let v = val?;
        acc = Some(match acc {
            None => (sgn * v.0, sgn * v.1),
            Some(a) => (a.0 + sgn * v.0, a.1 + sgn * v.1),
        });
    }
    acc
}

/// Evaluate `45+240`-style +/- chains (tikz polar angles use them).
fn eval_num(s: &str) -> Option<f64> {
    let s = s.trim().trim_start_matches('(').trim_end_matches(')');
    let mut total = 0.0f64;
    let mut cur = String::new();
    let mut sign = 1.0;
    let mut any = false;
    for (k, c) in s.chars().enumerate() {
        if (c == '+' || c == '-') && k > 0 && !cur.trim().is_empty() {
            total += sign * cur.trim().parse::<f64>().ok()?;
            sign = if c == '-' { -1.0 } else { 1.0 };
            cur.clear();
            any = true;
        } else {
            cur.push(c);
        }
    }
    total += sign * cur.trim().parse::<f64>().ok()?;
    let _ = any;
    Some(total)
}

/// `%` comments run to end of line (but `\%` is a literal).
fn strip_tex_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev = '\0';
    let mut skipping = false;
    for c in s.chars() {
        if skipping {
            if c == '\n' {
                skipping = false;
            }
            continue;
        }
        if c == '%' && prev != '\\' {
            skipping = true;
            continue;
        }
        out.push(c);
        prev = c;
    }
    out
}

/// Pull `NAME/.style={...}` definitions out of an option list.
fn extract_styles(opts: &str) -> (String, HashMap<String, String>) {
    let mut styles = HashMap::new();
    let mut out = String::new();
    let b: Vec<char> = opts.chars().collect();
    let mut i = 0usize;
    while i < b.len() {
        let rest: String = b[i..].iter().collect();
        if let Some(k) = rest.find("/.style=") {
            // name runs back to the previous top-level comma
            let name: String = rest[..k]
                .rsplit(',')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(|c| c == '[' || c == ']' || c == '{' || c == '}')
                .trim()
                .to_string();
            let mut j = i + k + "/.style=".len();
            while j < b.len() && b[j].is_whitespace() {
                j += 1;
            }
            let mut spec = String::new();
            if b.get(j) == Some(&'{') {
                let mut depth = 0i32;
                while j < b.len() {
                    match b[j] {
                        '{' => {
                            depth += 1;
                            if depth > 1 {
                                spec.push('{');
                            }
                        }
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                j += 1;
                                break;
                            }
                            spec.push('}');
                        }
                        c => spec.push(c),
                    }
                    j += 1;
                }
            }
            // keep everything before the name in the remaining options
            let keep: String = rest[..k].rsplit_once(',').map(|(a, _)| a).unwrap_or("").to_string();
            out.push_str(&keep);
            out.push(',');
            if !name.is_empty() {
                styles.insert(name, spec);
            }
            i = j;
        } else {
            out.push_str(&rest);
            break;
        }
    }
    (out, styles)
}

fn len_pt(s: &str) -> Option<f64> {
    let t = s.trim();
    for (suf, f) in [("pt", 1.0), ("em", 11.0), ("ex", 4.4), ("cm", 28.45), ("mm", 2.845)] {
        if let Some(v) = t.strip_suffix(suf) {
            return v.trim().parse::<f64>().ok().map(|x| x * f);
        }
    }
    t.parse::<f64>().ok()
}

fn parse_opts(opts: &str, unit: f64) -> Opts {
    let mut o = Opts { scale: (1.0, 1.0), looseness: 1.0, mid_pos: 0.6, ..Default::default() };
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
    for raw in parts {
        let p = raw.trim().replace(' ', "");
        if let Some(v) = p.strip_prefix("shift=") {
            if let Some(pr) = parse_pair(v) {
                o.shift = pr;
            }
        } else if let Some(v) = p.strip_prefix("xscale=") {
            o.scale.0 *= eval_num(v).unwrap_or(1.0);
        } else if let Some(v) = p.strip_prefix("yscale=") {
            o.scale.1 *= eval_num(v).unwrap_or(1.0);
        } else if let Some(v) = p.strip_prefix("scale=") {
            let f = eval_num(v).unwrap_or(1.0);
            o.scale.0 *= f;
            o.scale.1 *= f;
        } else if let Some(v) = p.strip_prefix("rotate=") {
            o.rotate = eval_num(v).unwrap_or(0.0);
        } else if let Some(v) = p.strip_prefix("linewidth=") {
            o.width = len_pt(v);
        } else if p == "thick" {
            o.width = Some(0.8);
        } else if p == "verythick" {
            o.width = Some(1.2);
        } else if let Some(v) = p.strip_prefix("fill=") {
            if let Some(c) = map_color(v) {
                o.fill = Some(c);
            }
        } else if let Some(v) = p.strip_prefix("draw=") {
            if let Some(c) = map_color(v) {
                o.color = Some(c);
            }
        } else if let Some(c) = map_color(&p) {
            o.color = Some(c);
        } else if let Some(v) = p.strip_prefix("looseness=") {
            o.looseness = v.parse().unwrap_or(1.0);
        } else if let Some(v) = p.strip_prefix("out=") {
            o.out_ang = eval_num(v);
        } else if let Some(v) = p.strip_prefix("in=") {
            o.in_ang = eval_num(v);
        } else if p.starts_with("preaction={draw=white") || p.starts_with("preaction={decorate") {
            if p.contains("draw=white") {
                let w = p
                    .split("linewidth=")
                    .nth(1)
                    .and_then(|v| len_pt(v.trim_end_matches('}')))
                    .unwrap_or(4.0);
                o.underlay = Some(w);
            }
        } else if p.starts_with("decoration={markings") || p == "postaction={decorate}" {
            // mid-line arrowhead decorations
            if p.contains("markings") {
                o.mid_arrow = true;
                if let Some(k) = p.find("atposition") {
                    let num: String = p[k + 10..]
                        .chars()
                        .take_while(|c| c.is_ascii_digit() || *c == '.')
                        .collect();
                    if let Ok(v) = num.parse() {
                        o.mid_pos = v;
                    }
                }
                if p.contains("arrowreversed") {
                    o.mid_rev = true;
                }
                if p.contains("{Latex}") || p.contains("{Stealth}") {
                    o.latex_tip = true;
                }
            }
        } else if let Some(v) = p.strip_prefix(">=") {
            o.latex_tip = matches!(v, "latex" | "stealth" | "Latex" | "Stealth");
        } else if p == "->" {
            o.arrow_end = true;
        } else if p == "<-" {
            o.arrow_start = true;
        } else if p == "<->" {
            o.arrow_end = true;
            o.arrow_start = true;
        } else if p == "dashed" {
            o.dashed = true;
        } else if p == "dotted" {
            o.dotted = true;
        } else if p == "bendleft" {
            o.bend = 30.0;
        } else if p == "bendright" {
            o.bend = -30.0;
        } else if let Some(v) = p.strip_prefix("bendleft=") {
            o.bend = v.parse().unwrap_or(30.0);
        } else if let Some(v) = p.strip_prefix("bendright=") {
            o.bend = -v.parse().unwrap_or(30.0);
        } else if let Some(v) = p.strip_prefix("shorten<=") {
            o.shorten_start = len_pt(v).unwrap_or(0.0);
        } else if let Some(v) = p.strip_prefix("shorten>=") {
            o.shorten_end = len_pt(v).unwrap_or(0.0);
        } else if let Some(v) = p.strip_prefix("transformcanvas={") {
            for part in v.trim_end_matches('}').split(',') {
                if let Some(x) = part.strip_prefix("xshift=") {
                    o.canvas_shift.0 += len_pt(x).unwrap_or(0.0) / unit;
                } else if let Some(y) = part.strip_prefix("yshift=") {
                    o.canvas_shift.1 += len_pt(y).unwrap_or(0.0) / unit;
                }
            }
        } else if let Some(v) = p.strip_prefix("xshift=") {
            o.xshift = len_pt(v).unwrap_or(0.0) / unit;
        } else if let Some(v) = p.strip_prefix("yshift=") {
            o.yshift = len_pt(v).unwrap_or(0.0) / unit;
        } else if p == "auto" {
            o.place = Some("auto".into());
        } else if let Some(v) = p.strip_prefix("anchor=") {
            // node anchored at its own <v>: extends the opposite way
            o.place = match v {
                "south" => Some("above".into()),
                "north" => Some("below".into()),
                "east" => Some("left".into()),
                "west" => Some("right".into()),
                "southwest" => Some("aboveright".into()),
                "southeast" => Some("aboveleft".into()),
                "northwest" => Some("belowright".into()),
                "northeast" => Some("belowleft".into()),
                _ => o.place.take(),
            };
        } else if matches!(
            p.as_str(),
            "above" | "below" | "left" | "right" | "aboveleft" | "aboveright"
                | "belowleft" | "belowright" | "midway"
        ) {
            if p != "midway" {
                o.place = Some(p);
            }
        }
    }
    o
}

pub(crate) fn to_cetz(body: &str, env_opts: &str) -> Option<(String, Vec<String>)> {
    let body = strip_tex_comments(body);
    // \colorlet{name}{spec}: substitute the spec for the name
    let mut body = body;
    {
        let re = regex::Regex::new(r"\\colorlet\{([A-Za-z][A-Za-z0-9]*)\}\{([^}]*)\}").unwrap();
        let defs: Vec<(String, String)> = re
            .captures_iter(&body)
            .map(|c| (c[1].to_string(), c[2].to_string()))
            .collect();
        body = re.replace_all(&body, "").to_string();
        for (name, spec) in defs {
            let wr = regex::Regex::new(&format!(r"\b{}\b", regex::escape(&name))).unwrap();
            body = wr.replace_all(&body, spec.as_str()).to_string();
        }
    }
    let body = body.as_str();
    let env_opts = strip_tex_comments(env_opts);
    let (env_opts, mut styles) = extract_styles(&env_opts);
    let unit = if body.contains("\\node") {
        UNIT_PT_GRAPH
    } else {
        UNIT_PT
    };
    let env = parse_opts(&env_opts, unit);
    let root = Frame {
        sx: env.scale.0,
        sy: env.scale.1,
        tx: 0.0,
        ty: 0.0,
        rot: env.rotate.to_radians(),
        color: env.color.unwrap_or_else(|| "black".into()),
        width: env.width.unwrap_or(0.4),
    };
    let node_scale = styles
        .get("every node")
        .map(|sp| parse_opts(sp, unit).scale.0)
        .unwrap_or(1.0);
    let b: Vec<char> = body.chars().collect();
    let mut i = 0usize;
    let mut out: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut frames: Vec<Frame> = vec![root];
    let mut nodes: HashMap<String, NamedNode> = HashMap::new();

    let starts = |b: &[char], i: usize, w: &str| {
        b[i..].iter().take(w.len()).collect::<String>() == w
    };
    let read_bracket = |b: &[char], mut i: usize| -> (String, usize) {
        while i < b.len() && b[i].is_whitespace() {
            i += 1;
        }
        if b.get(i) != Some(&'[') {
            return (String::new(), i);
        }
        let mut j = i;
        let mut depth = 0i32;
        while j < b.len() {
            match b[j] {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return (b[i..=j].iter().collect(), j + 1);
                    }
                }
                _ => {}
            }
            j += 1;
        }
        (String::new(), i)
    };
    let read_paren = |b: &[char], mut i: usize| -> Option<(String, usize)> {
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
        Some((b[i + 1..j].iter().collect(), j + 1))
    };
    let read_stmt = |b: &[char], mut i: usize| -> (String, usize) {
        let start = i;
        let mut depth = 0i32;
        while i < b.len() && (b[i] != ';' || depth > 0) {
            match b[i] {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        (b[start..i].iter().collect(), i + 1)
    };

    while i < b.len() {
        if starts(&b, i, "\\begin{scope}") {
            i += "\\begin{scope}".len();
            let (optxt, ni) = read_bracket(&b, i);
            i = ni;
            let o = parse_opts(&optxt, unit);
            let shift = (o.shift.0 + o.xshift, o.shift.1 + o.yshift);
            let mut f = frames.last().unwrap().child(shift, o.scale);
            if o.rotate != 0.0 {
                f.rot += o.rotate.to_radians();
            }
            if let Some(c) = o.color {
                f.color = c;
            }
            if let Some(w) = o.width {
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
        if starts(&b, i, "\\tikzstyle") {
            // \tikzstyle{name}=[spec] - possibly without a trailing
            // semicolon, so read exactly its own tokens
            i += "\\tikzstyle".len();
            let mut name = String::new();
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            if b.get(i) == Some(&'{') {
                i += 1;
                while i < b.len() && b[i] != '}' {
                    name.push(b[i]);
                    i += 1;
                }
                i += 1;
            }
            while i < b.len() && (b[i].is_whitespace() || b[i] == '=') {
                i += 1;
            }
            let mut spec = String::new();
            if b.get(i) == Some(&'[') {
                i += 1;
                while i < b.len() && b[i] != ']' {
                    spec.push(b[i]);
                    i += 1;
                }
                i += 1;
            }
            while i < b.len() && (b[i].is_whitespace() || b[i] == ';') {
                i += 1;
            }
            if !name.trim().is_empty() {
                styles.insert(name.trim().to_string(), spec);
            }
            continue;
        }
        if starts(&b, i, "\\tikzset") {
            // brace group, no trailing semicolon: read it balanced
            i += "\\tikzset".len();
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            if b.get(i) == Some(&'{') {
                let mut depth = 0i32;
                let start = i + 1;
                while i < b.len() {
                    match b[i] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                let inner: String = b[start..i].iter().collect();
                i += 1;
                let (_, defs) = extract_styles(&inner);
                styles.extend(defs);
            }
            continue;
        }
        if starts(&b, i, "\\usetikzlibrary") {
            // no trailing semicolon: skip the word and its {..} group
            i += "\\usetikzlibrary".len();
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            if b.get(i) == Some(&'{') {
                while i < b.len() && b[i] != '}' {
                    i += 1;
                }
                i += 1;
            }
            continue;
        }
        if starts(&b, i, "\\coordinate") || starts(&b, i, "\\node") {
            let is_coord = starts(&b, i, "\\coordinate");
            i += if is_coord { 11 } else { 5 };
            let (stmt, ni) = read_stmt(&b, i);
            i = ni;
            match parse_node_stmt(&stmt, frames.last().unwrap(), is_coord, unit, &styles, node_scale, &nodes) {
                Ok((name, nn, op)) => {
                    if let Some(n) = name {
                        nodes.insert(n, nn);
                    }
                    if let Some(op) = op {
                        out.push(op);
                    }
                }
                Err(e) => warnings.push(e),
            }
            continue;
        }
        if starts(&b, i, "\\draw") || starts(&b, i, "\\fill") || starts(&b, i, "\\path")
            || starts(&b, i, "\\filldraw")
        {
            let is_filldraw = starts(&b, i, "\\filldraw");
            let is_fill = starts(&b, i, "\\fill") && !is_filldraw;
            let is_path = starts(&b, i, "\\path");
            i += if is_filldraw { 9 } else { 5 };
            let (optxt, ni) = read_bracket(&b, i);
            i = ni;
            // expand style names through the dictionary (depth-aware split)
            let optxt = {
                let inner = optxt.trim_matches(|c| c == '[' || c == ']');
                let mut parts: Vec<String> = Vec::new();
                let mut depth = 0i32;
                let mut cur = String::new();
                for c in inner.chars() {
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
                let expanded: Vec<String> = parts
                    .into_iter()
                    .map(|t| {
                        let tt = t.trim();
                        let base = tt.strip_prefix("style=").unwrap_or(tt).trim();
                        styles.get(base).cloned().unwrap_or(t)
                    })
                    .collect();
                format!("[{}]", expanded.join(","))
            };
            let mut o = parse_opts(&optxt, unit);
            o.latex_tip = o.latex_tip || env.latex_tip;
            o.path_only = is_path;
            if is_filldraw && o.fill.is_none() {
                o.fill = o.color.clone().or_else(|| Some("black".into()));
            }
            let frame = frames.last().unwrap().clone();
            if o.color.is_none() {
                o.color = Some(frame.color.clone());
            }
            if o.width.is_none() {
                o.width = Some(frame.width);
            }
            if is_fill && o.fill.is_none() {
                o.fill = o.color.clone();
            }
            let (stmt, ni) = read_stmt(&b, i);
            i = ni;
            match parse_path(&stmt, &frame, &o, &nodes, unit, is_fill) {
                Ok(mut ops) => out.append(&mut ops),
                Err(e) => warnings.push(e),
            }
            continue;
        }
        i += 1;
        let _ = read_paren;
    }
    if out.is_empty() {
        return None;
    }
    let code = format!(
        "box(baseline: 42%, cetz.canvas(length: {}pt, {{\n    import cetz.draw: *\n    {}\n  }}))",
        unit,
        out.join("\n    ")
    );
    Some((code, warnings))
}

/// `[opts] (name) at (x,y) {body}` after \node or \coordinate.
fn parse_node_stmt(
    stmt: &str,
    frame: &Frame,
    is_coord: bool,
    unit: f64,
    styles: &HashMap<String, String>,
    node_scale: f64,
    nodes: &HashMap<String, NamedNode>,
) -> Result<(Option<String>, NamedNode, Option<String>), String> {
    let b: Vec<char> = stmt.chars().collect();
    let mut i = 0usize;
    let skip_ws = |b: &[char], mut i: usize| {
        while i < b.len() && b[i].is_whitespace() {
            i += 1;
        }
        i
    };
    i = skip_ws(&b, i);
    let mut node_opts = String::new();
    if b.get(i) == Some(&'[') {
        let start = i;
        let mut depth = 0i32;
        while i < b.len() {
            match b[i] {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        node_opts = b[start + 1..i.saturating_sub(1)].iter().collect();
    }
    i = skip_ws(&b, i);
    let mut name = None;
    if b.get(i) == Some(&'(') {
        let mut j = i + 1;
        while j < b.len() && b[j] != ')' {
            j += 1;
        }
        name = Some(b[i + 1..j].iter().collect::<String>().trim().to_string());
        i = j + 1;
    }
    i = skip_ws(&b, i);
    // relative placement: [above right of=X] with node distance
    if !(b.get(i) == Some(&'a') && b.get(i + 1) == Some(&'t')) {
        let mut toks: Vec<String> = Vec::new();
        for t in node_opts.split(',') {
            let t = t.trim();
            let base = t.strip_prefix("style=").unwrap_or(t).trim();
            if let Some(spec) = styles.get(base) {
                toks.extend(spec.split(',').map(|x| x.trim().to_string()));
            } else if !t.is_empty() {
                toks.push(t.to_string());
            }
        }
        // node distance: option, else the every-node default, else 1cm
        let mut dist_pt = styles
            .get("every node")
            .and_then(|sp| {
                sp.split(',').find_map(|t| {
                    t.replace(' ', "").strip_prefix("nodedistance=").and_then(len_pt)
                })
            })
            .unwrap_or(28.45);
        let mut rel: Option<((f64, f64), String)> = None;
        for t in &toks {
            let t = t.replace(' ', "");
            if let Some(v) = t.strip_prefix("nodedistance=") {
                if let Some(d) = len_pt(v) {
                    dist_pt = d;
                }
            } else if let Some(k) = t.find("of=") {
                let dir = &t[..k];
                let name = t[k + 3..].trim().to_string();
                let s2 = std::f64::consts::FRAC_1_SQRT_2;
                let v = match dir {
                    "above" => (0.0, 1.0),
                    "below" => (0.0, -1.0),
                    "left" => (-1.0, 0.0),
                    "right" => (1.0, 0.0),
                    "aboveright" => (s2, s2),
                    "aboveleft" => (-s2, s2),
                    "belowright" => (s2, -s2),
                    "belowleft" => (-s2, -s2),
                    _ => (0.0, 0.0),
                };
                rel = Some((v, name));
            }
        }
        if let Some(((vx, vy), base)) = rel {
            if let Some(n) = nodes.get(&base) {
                let d = dist_pt / unit;
                let (x, y) = (n.x + vx * d, n.y + vy * d);
                return finish_node(
                    &b, i, name, x, y, is_coord, unit, styles, node_scale, node_opts,
                );
            }
            return Err("of= names an unknown node".into());
        }
        // no `at`, no relative placement: tikz puts it at the origin
        return finish_node(
            &b, i, name, 0.0, 0.0, is_coord, unit, styles, node_scale, node_opts,
        );
    }
    i += 2;
    i = skip_ws(&b, i);
    if b.get(i) != Some(&'(') {
        return Err("node without coord".into());
    }
    let mut depth = 0i32;
    let mut j = i;
    while j < b.len() {
        match b[j] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    let inner: String = b[i + 1..j].iter().collect();
    let (x, y) = node_pt(inner.trim(), nodes)
        .or_else(|| eval_coord(&inner, nodes, frame))
        .ok_or("bad node coord")?;
    i = j + 1;
    finish_node(&b, i, name, x, y, is_coord, unit, styles, node_scale, node_opts)
}

#[allow(clippy::too_many_arguments)]
fn finish_node(
    b: &[char],
    mut i: usize,
    mut name: Option<String>,
    x: f64,
    y: f64,
    is_coord: bool,
    unit: f64,
    styles: &HashMap<String, String>,
    node_scale: f64,
    node_opts: String,
) -> Result<(Option<String>, NamedNode, Option<String>), String> {
    let skip_ws = |b: &[char], mut i: usize| {
        while i < b.len() && b[i].is_whitespace() {
            i += 1;
        }
        i
    };
    i = skip_ws(b, i);
    // the name may also follow the coordinate: \node at (x,y) (n) {..}
    if name.is_none() && b.get(i) == Some(&'(') {
        let mut j = i + 1;
        while j < b.len() && b[j] != ')' {
            j += 1;
        }
        name = Some(b[i + 1..j].iter().collect::<String>().trim().to_string());
        i = skip_ws(b, j + 1);
    }
    let mut body = String::new();
    if b.get(i) == Some(&'{') {
        let mut depth = 0i32;
        let mut k = i;
        while k < b.len() {
            match b[k] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        body = b[i + 1..k].iter().collect();
    }
    // resolve style=NAME / bare NAME tokens through the dictionary
    let mut toks: Vec<String> = Vec::new();
    for t in node_opts.split(',') {
        let t = t.trim();
        let base = t.strip_prefix("style=").unwrap_or(t).trim();
        if let Some(spec) = styles.get(base) {
            toks.extend(spec.split(',').map(|x| x.trim().to_string()));
        } else if !t.is_empty() {
            toks.push(t.to_string());
        }
    }
    let (mut fill, mut draw, mut circle, mut rectangle, mut inner_sep) =
        (None::<String>, false, false, false, None::<f64>);
    let mut invisible = false;
    for t in &toks {
        let t = t.replace(' ', "");
        if let Some(v) = t.strip_prefix("fill=") {
            if v != "none" && COLORS.contains(&v) {
                fill = Some(v.to_string());
            }
        } else if t == "fill" {
            fill = Some("black".into());
        } else if t == "opacity=0" || t == "opacity=0.0" {
            invisible = true;
        } else if t == "draw" {
            draw = true;
        } else if let Some(v) = t.strip_prefix("draw=") {
            draw = v != "none";
        } else if t == "circle" || t == "shape=circle" {
            circle = true;
        } else if t == "rectangle" || t == "shape=rectangle" {
            rectangle = true;
        } else if let Some(v) = t.strip_prefix("innersep=") {
            inner_sep = len_pt(v);
        }
    }
    let trimmed = body.trim();
    if invisible {
        // opacity=0 phantoms exist only for alignment
        let hw = node_scale * (crate::tikzcd::est_halfwidth_pt(trimmed) + 3.0) / unit;
        return Ok((name, NamedNode { x, y, hw, hh: node_scale * 8.0 / unit, round: false }, None));
    }
    if circle && trimmed.is_empty() && (fill.is_some() || draw) {
        // a styled dot (quiver `bn` nodes and friends)
        let r = (inner_sep.unwrap_or(1.8) + 1.0) / unit;
        let paint = fill.unwrap_or_else(|| "black".into());
        let stroke = if draw { "0.5pt + black" } else { "none" };
        let op = format!(
            "circle(({:.3}, {:.3}), radius: {:.3}, stroke: {}, fill: {})",
            x, y, r, stroke, paint
        );
        return Ok((name, NamedNode { x, y, hw: r, hh: r, round: true }, Some(op)));
    }
    if is_coord {
        return Ok((name, NamedNode { x, y, hw: 0.0, hh: 0.0, round: false }, None));
    }
    if trimmed.is_empty() {
        // an empty \node still has its inner-sep box: plain edges to
        // it stop short (lines land on disk rims drawn at the point)
        let r = inner_sep.unwrap_or(3.3) / unit;
        return Ok((name, NamedNode { x, y, hw: r, hh: r, round: false }, None));
    }
    let mut hw = node_scale * (crate::tikzcd::est_halfwidth_pt(trimmed) + 3.0) / unit;
    let mut hh = node_scale * 8.0 / unit;
    let mut round = false;
    let op = if rectangle && draw {
        // boxed node (string-diagram morphisms)
        hw += 3.0 / unit;
        hh += 2.0 / unit;
        format!(
            "content(({:.3}, {:.3}), frame: \"rect\", stroke: 0.5pt + black, fill: {}, padding: (x: 5pt, y: 4pt), {})",
            x,
            y,
            fill.unwrap_or_else(|| "white".into()),
            node_content_scaled(trimmed, None, node_scale)
        )
    } else if circle && draw {
        // circled node: draw the circle ourselves at a known radius so
        // wires can land exactly on the rim
        // radius: half-diagonal of the measured content plus padding
        // (tikz circle nodes enclose the content box)
        let content_ts = emit::ts(&crate::tikzcd::clean_cell_tex(
            trimmed.trim_matches('$'),
        ));
        let r_pt = match crate::tikzcd::measure_one(&content_ts) {
            Some((w, h)) => {
                (w * node_scale / 2.0).hypot(h * node_scale / 2.0)
                    + inner_sep.unwrap_or(1.5)
                    + 0.5
            }
            None => {
                let est = crate::tikzcd::est_halfwidth_pt(trimmed) * node_scale;
                est.max(4.6 * node_scale) + inner_sep.unwrap_or(1.5) + 0.8
            }
        };
        hw = r_pt / unit;
        hh = hw;
        round = true;
        format!(
            "circle(({x:.3}, {y:.3}), radius: {r:.3}, stroke: 0.5pt + black, fill: {f})\n    content(({x:.3}, {y:.3}), {c})",
            x = x,
            y = y,
            r = hw,
            f = fill.unwrap_or_else(|| "white".into()),
            c = node_content_scaled(trimmed, None, node_scale)
        )
    } else {
        // [above]/[below]/[left]/[right] shift off the coordinate
        let o = parse_opts(&node_opts, unit);
        let (anchor, gx, gy) = place_anchor(o.place.as_deref(), unit);
        if anchor != "center" {
            format!(
                "content(({:.3}, {:.3}), anchor: \"{}\", {})",
                x + gx,
                y + gy,
                anchor,
                node_content_scaled(trimmed, None, node_scale)
            )
        } else {
            format!(
                "content(({:.3}, {:.3}), {})",
                x,
                y,
                node_content_scaled(trimmed, None, node_scale)
            )
        }
    };
    Ok((name, NamedNode { x, y, hw, hh, round }, Some(op)))
}

fn parse_pair(s: &str) -> Option<(f64, f64)> {
    let s = s.trim().trim_start_matches('{').trim_start_matches('(');
    let inner: String = s.chars().take_while(|c| *c != ')').collect();
    if let Some((ang, rad)) = inner.split_once(':') {
        // polar (angle:radius); angles may be sums like 45+240
        let a = eval_num(ang)?;
        let r = eval_num(rad)?;
        let a = a.to_radians();
        return Some((r * a.cos(), r * a.sin()));
    }
    let mut it = inner.split(',');
    let x = eval_num(it.next()?)?;
    let y = eval_num(it.next()?)?;
    Some((x, y))
}

fn fmt_pt(p: (f64, f64)) -> String {
    format!("({:.3}, {:.3})", p.0, p.1)
}

/// One endpoint: coordinates plus the node's half-extents if named.
#[derive(Clone, Copy)]
struct EndPt {
    x: f64,
    y: f64,
    hw: f64,
    hh: f64,
    round: bool,
}

fn parse_path(
    path: &str,
    frame: &Frame,
    opts: &Opts,
    nodes: &HashMap<String, NamedNode>,
    unit: f64,
    fill_mode: bool,
) -> Result<Vec<String>, String> {
    let width = opts.width.unwrap_or(0.4);
    let color = opts.color.clone().unwrap_or_else(|| "black".into());
    let mut stroke_parts = vec![format!("{}pt + {}", width, color)];
    if opts.dashed {
        stroke_parts.push("dash: \"dashed\"".into());
    } else if opts.dotted {
        stroke_parts.push("dash: \"dotted\"".into());
    }
    let stroke = if stroke_parts.len() == 1 {
        format!("stroke: {}", stroke_parts[0])
    } else {
        format!(
            "stroke: (paint: {}, thickness: {}pt, dash: {})",
            color,
            width,
            if opts.dashed { "\"dashed\"" } else { "\"dotted\"" }
        )
    };
    // tikz's default arrow tip is the thin "to" barb; only >=latex
    // and friends fill a solid triangle
    let tip = if opts.latex_tip {
        "\">\", fill: black"
    } else {
        "\"straight\""
    };
    let mark = match (opts.arrow_start, opts.arrow_end) {
        (true, true) => format!(", mark: (start: {tip}, end: {tip})"),
        (true, false) => format!(", mark: (start: {tip})"),
        (false, true) => format!(", mark: (end: {tip})"),
        _ => String::new(),
    };
    let (cdx, cdy) = opts.canvas_shift;

    // `(a) edge[opts] node {..} (b)` behaves like `to` for our purposes
    let path = regex::Regex::new(r"\bedge\b")
        .unwrap()
        .replace_all(path, "to")
        .to_string();
    let path = path.as_str();
    let b: Vec<char> = path.chars().collect();
    let mut i = 0usize;
    let mut ops = Vec::new();
    let mut last: Option<EndPt> = None;
    // fill mode: segments accumulate into one closed filled path
    let mut segs: Vec<String> = Vec::new();
    let mut path_start: Option<EndPt> = None;
    // tikz shortens the first and last segment of the whole path, not
    // every corner of a polyline
    let n_segs = {
        let mut n = 0usize;
        let mut depth = 0i32;
        let mut k = 0usize;
        while k < b.len() {
            match b[k] {
                '{' | '[' => depth += 1,
                '}' | ']' => depth -= 1,
                '-' if depth == 0 && b.get(k + 1) == Some(&'-') => {
                    n += 1;
                    k += 1;
                }
                't' if depth == 0
                    && b.get(k + 1) == Some(&'o')
                    && (k == 0 || !b[k - 1].is_alphanumeric())
                    && b.get(k + 2).map(|c| !c.is_alphanumeric()).unwrap_or(true) =>
                {
                    n += 1;
                    k += 1;
                }
                _ => {}
            }
            k += 1;
        }
        n
    };
    let mut seg_idx = 0usize;
    // labels waiting to ride the NEXT segment (tikz `to node {..} (b)`)
    let mut pending_label: Option<(String, Opts)> = None;

    let read_end = |b: &[char], mut i: usize| -> Option<(EndPt, usize)> {
        while i < b.len() && b[i].is_whitespace() {
            i += 1;
        }
        if b.get(i) != Some(&'(') {
            return None;
        }
        // balanced: calc coords nest parens
        let mut depth = 0i32;
        let mut j = i;
        while j < b.len() {
            match b[j] {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let inner: String = b[i + 1..j].iter().collect();
        let inner = inner.trim();
        if let Some(n) = nodes.get(inner) {
            return Some((EndPt { x: n.x, y: n.y, hw: n.hw, hh: n.hh, round: n.round }, j + 1));
        }
        // `(name.center)`: an exact point on the node, no trimming.
        // The suffix must be a word - otherwise `(2.75, 0)` would
        // resolve against a node named "2"
        if let Some((base, anchor)) = inner.rsplit_once('.') {
            if is_anchor_word(anchor) {
                if let Some(n) = nodes.get(base.trim()) {
                    return Some((EndPt { x: n.x, y: n.y, hw: 0.0, hh: 0.0, round: false }, j + 1));
                }
            }
        }
        let (x, y) = eval_coord(&inner, nodes, frame)?;
        Some((EndPt { x, y, hw: 0.0, hh: 0.0, round: false }, j + 1))
    };

    // trim `d` canvas units off point `p` toward `q`, plus p's border
    let trim = |p: EndPt, q: EndPt, extra: f64| -> (f64, f64) {
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        let len = dx.hypot(dy).max(1e-6);
        let (ux, uy) = (dx / len, dy / len);
        let border = if p.hw > 0.0 || p.hh > 0.0 {
            let k = (ux.abs() / p.hw.max(1e-6)).max(uy.abs() / p.hh.max(1e-6));
            if k <= 1e-9 {
                // degenerate zero-length chord
                0.0
            } else {
                // wires TOUCH round rims and small dots; text keeps a gap
                let pad = if p.round || p.hw.max(p.hh) <= 3.5 / unit { 0.0 } else { 2.0 / unit };
                ((1.0 / k) + pad).min(len * 0.5)
            }
        } else {
            0.0
        };
        let d = border + extra;
        (p.x + ux * d, p.y + uy * d)
    };

    // arrowhead stub riding a cubic at opts.mid_pos
    let mid_stub = |p0: (f64, f64), p1: (f64, f64), c1: (f64, f64), c2: (f64, f64)| -> String {
        let t = opts.mid_pos;
        let bez = |t: f64| {
            let u = 1.0 - t;
            (
                u * u * u * p0.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * p1.0,
                u * u * u * p0.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * p1.1,
            )
        };
        let (sa, sb) = if opts.mid_rev {
            (bez((t + 0.06).min(1.0)), bez(t))
        } else {
            (bez((t - 0.06).max(0.0)), bez(t))
        };
        if (sb.0 - sa.0).hypot(sb.1 - sa.1) < 1e-4 {
            // cetz divides by zero placing marks on empty segments
            return String::new();
        }
        format!(
            "line({}, {}, {}, mark: (end: {}))",
            fmt_pt(sa),
            fmt_pt(sb),
            stroke,
            tip
        )
    };
    while i < b.len() {
        while i < b.len() && b[i].is_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] == '(' {
            let (e, ni) = read_end(&b, i).ok_or("bad coordinate")?;
            if path_start.is_none() {
                path_start = Some(e);
            }
            last = Some(e);
            i = ni;
            continue;
        }
        let rest: String = b[i..].iter().collect();
        if rest.starts_with("circle") {
            i += 6;
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            let mut radius = 0.1;
            if b.get(i) == Some(&'[') {
                let mut j = i;
                while j < b.len() && b[j] != ']' {
                    j += 1;
                }
                let spec: String = b[i + 1..j].iter().collect();
                if let Some(v) = spec.replace(' ', "").strip_prefix("radius=") {
                    radius = v.parse().unwrap_or(0.1);
                }
                i = j + 1;
            } else if b.get(i) == Some(&'(') {
                let mut j = i + 1;
                while j < b.len() && b[j] != ')' {
                    j += 1;
                }
                let spec: String = b[i + 1..j].iter().collect();
                radius = spec.trim().parse().unwrap_or(0.1);
                i = j + 1;
            }
            let c = last.ok_or("circle without center")?;
            let fill = opts
                .fill
                .as_ref()
                .map(|f| format!(", fill: {}", f))
                .unwrap_or_default();
            ops.push(format!(
                "circle(({:.3}, {:.3}), radius: {:.3}, {}{})",
                c.x + cdx,
                c.y + cdy,
                radius,
                stroke,
                fill
            ));
            continue;
        }
        if rest.starts_with("rectangle") {
            i += 9;
            let (e, ni) = read_end(&b, i).ok_or("rectangle without corner")?;
            i = ni;
            let c = last.ok_or("rectangle without start")?;
            let fill = opts
                .fill
                .as_ref()
                .map(|f| format!(", fill: {}", f))
                .unwrap_or_default();
            ops.push(format!(
                "rect(({:.3}, {:.3}), ({:.3}, {:.3}), {}{})",
                c.x + cdx,
                c.y + cdy,
                e.x + cdx,
                e.y + cdy,
                stroke,
                fill
            ));
            last = Some(e);
            continue;
        }
        if rest.starts_with("to") || rest.starts_with("--") {
            i += 2;
            {
                let mut k = i;
                while k < b.len() && b[k].is_whitespace() {
                    k += 1;
                }
                if b[k..].iter().take(5).collect::<String>() == "cycle" {
                    // close the path back to its first point
                    if let (Some(st), Some(l)) = (path_start, last) {
                        let geom = format!("line({}, {}", fmt_pt((l.x + cdx, l.y + cdy)), fmt_pt((st.x + cdx, st.y + cdy)));
                        if fill_mode {
                            segs.push(format!("{})", geom));
                        } else {
                            ops.push(format!("{}, {})", geom, stroke));
                        }
                    }
                    i = k + 5;
                    continue;
                }
            }
            // optional to[bend ...] / to[out=..,in=..] options
            let mut seg_bend = opts.bend;
            let mut seg_out = opts.out_ang;
            let mut seg_in = opts.in_ang;
            if b.get(i) == Some(&'[') {
                let mut j = i;
                while j < b.len() && b[j] != ']' {
                    j += 1;
                }
                let o = parse_opts(&b[i..=j.min(b.len() - 1)].iter().collect::<String>(), unit);
                if o.bend != 0.0 {
                    seg_bend = o.bend;
                }
                if o.out_ang.is_some() {
                    seg_out = o.out_ang;
                }
                if o.in_ang.is_some() {
                    seg_in = o.in_ang;
                }
                i = j + 1;
            }
            // labels riding the segment: node[opts] {tex} (possibly several)
            loop {
                while i < b.len() && b[i].is_whitespace() {
                    i += 1;
                }
                let r: String = b[i..].iter().take(4).collect();
                if r != "node" {
                    break;
                }
                i += 4;
                let mut lab_opts = Opts { scale: (1.0, 1.0), ..Default::default() };
                while i < b.len() && b[i].is_whitespace() {
                    i += 1;
                }
                if b.get(i) == Some(&'[') {
                    let mut j = i;
                    while j < b.len() && b[j] != ']' {
                        j += 1;
                    }
                    lab_opts = parse_opts(&b[i..=j.min(b.len() - 1)].iter().collect::<String>(), unit);
                    i = j + 1;
                    while i < b.len() && b[i].is_whitespace() {
                        i += 1;
                    }
                }
                if b.get(i) != Some(&'{') {
                    return Err("segment node without body".into());
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
                pending_label = Some((body, lab_opts));
            }
            let (e, ni) = read_end(&b, i).ok_or("line without target")?;
            let s0 = last.ok_or("line without start")?;
            let sh_s = if seg_idx == 0 { opts.shorten_start } else { 0.0 };
            let sh_e = if seg_idx + 1 >= n_segs { opts.shorten_end } else { 0.0 };
            seg_idx += 1;
            let p0 = trim(s0, e, sh_s / unit);
            let p1 = trim(e, s0, sh_e / unit);
            let p0 = (p0.0 + cdx, p0.1 + cdy);
            let p1 = (p1.0 + cdx, p1.1 + cdy);
            let mut seg_curve: Option<((f64, f64), (f64, f64))> = None;
            if let (Some(oa), Some(ia)) = (seg_out, seg_in) {
                // tikz to[out=..,in=..]: cubic with tangent directions;
                // control distance ~0.39 x chord (looseness 1).
                // Chord-direction trimming is wrong for a curve: at a
                // small dot it pokes out the far side - end at center
                let (oa, ia) = (oa.to_radians(), ia.to_radians());
                // curves depart/arrive along their tangents: rims and
                // shorten apply in the tangent direction, not the chord
                let ins0 = if s0.round { s0.hw } else { 0.0 } + sh_s / unit;
                let ins1 = if e.round { e.hw } else { 0.0 } + sh_e / unit;
                let p0 = if ins0 > 0.0 {
                    (s0.x + cdx + ins0 * oa.cos(), s0.y + cdy + ins0 * oa.sin())
                } else {
                    p0
                };
                let p1 = if ins1 > 0.0 {
                    (e.x + cdx + ins1 * ia.cos(), e.y + cdy + ins1 * ia.sin())
                } else {
                    p1
                };
                let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
                let d = 0.39 * dx.hypot(dy);
                let c1 = (p0.0 + d * oa.cos(), p0.1 + d * oa.sin());
                let c2 = (p1.0 + d * ia.cos(), p1.1 + d * ia.sin());
                seg_curve = Some((c1, c2));
                if fill_mode {
                    segs.push(format!(
                        "bezier({}, {}, {}, {})",
                        fmt_pt(p0), fmt_pt(p1), fmt_pt(c1), fmt_pt(c2)
                    ));
                    last = Some(e);
                    i = ni;
                    continue;
                }
                if opts.path_only {
                    if opts.mid_arrow {
                        let stub = mid_stub(p0, p1, c1, c2);
                        if !stub.is_empty() {
                            ops.push(stub);
                        }
                    }
                    last = Some(e);
                    i = ni;
                    continue;
                }
                if let Some(w) = opts.underlay {
                    // the white-underlay crossing trick: a fat white
                    // stroke below erases whatever this wire crosses
                    ops.push(format!(
                        "bezier({}, {}, {}, {}, stroke: {}pt + white)",
                        fmt_pt(p0),
                        fmt_pt(p1),
                        fmt_pt(c1),
                        fmt_pt(c2),
                        w
                    ));
                }
                ops.push(format!(
                    "bezier({}, {}, {}, {}, {}{})",
                    fmt_pt(p0),
                    fmt_pt(p1),
                    fmt_pt(c1),
                    fmt_pt(c2),
                    stroke,
                    mark
                ));
                if opts.mid_arrow {
                    let stub = mid_stub(p0, p1, c1, c2);
                    if !stub.is_empty() {
                        ops.push(stub);
                    }
                }
            } else if seg_bend != 0.0 {
                // tikz bend left=B: tangents rotated B off the chord,
                // control distance 0.3915 x len x looseness - smooth
                // through 90-degree caps where a flat dome kinks
                let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
                let ang = dy.atan2(dx);
                let brad = seg_bend.to_radians();
                let oa = ang + brad;
                let ia = ang + std::f64::consts::PI - brad;
                // rims/shorten along the departure/arrival tangents
                let ins0 = if s0.round { s0.hw } else { 0.0 } + sh_s / unit;
                let ins1 = if e.round { e.hw } else { 0.0 } + sh_e / unit;
                let p0 = if ins0 > 0.0 {
                    (s0.x + cdx + ins0 * oa.cos(), s0.y + cdy + ins0 * oa.sin())
                } else {
                    p0
                };
                let p1 = if ins1 > 0.0 {
                    (e.x + cdx + ins1 * ia.cos(), e.y + cdy + ins1 * ia.sin())
                } else {
                    p1
                };
                let len = (p1.0 - p0.0).hypot(p1.1 - p0.1).max(1e-6);
                let d = 0.3915 * len * opts.looseness * (1.0 + 0.9 * (brad.abs() / std::f64::consts::PI));
                let c1 = (p0.0 + d * oa.cos(), p0.1 + d * oa.sin());
                let c2 = (p1.0 + d * ia.cos(), p1.1 + d * ia.sin());
                seg_curve = Some((c1, c2));
                seg_curve = Some((c1, c2));
                if fill_mode {
                    segs.push(format!(
                        "bezier({}, {}, {}, {})",
                        fmt_pt(p0), fmt_pt(p1), fmt_pt(c1), fmt_pt(c2)
                    ));
                    last = Some(e);
                    i = ni;
                    continue;
                }
                if opts.path_only {
                    if opts.mid_arrow {
                        let stub = mid_stub(p0, p1, c1, c2);
                        if !stub.is_empty() {
                            ops.push(stub);
                        }
                    }
                    last = Some(e);
                    i = ni;
                    continue;
                }
                ops.push(format!(
                    "bezier({}, {}, {}, {}, {}{})",
                    fmt_pt(p0),
                    fmt_pt(p1),
                    fmt_pt(c1),
                    fmt_pt(c2),
                    stroke,
                    mark
                ));
                if opts.mid_arrow {
                    let stub = mid_stub(p0, p1, c1, c2);
                    if !stub.is_empty() {
                        ops.push(stub);
                    }
                }
            } else {
                if fill_mode {
                    segs.push(format!("line({}, {})", fmt_pt(p0), fmt_pt(p1)));
                    last = Some(e);
                    i = ni;
                    continue;
                }
                if opts.path_only {
                    // \path draws nothing; only decorations show
                    if opts.mid_arrow {
                        let t = opts.mid_pos;
                        let at = |t: f64| {
                            (p0.0 + t * (p1.0 - p0.0), p0.1 + t * (p1.1 - p0.1))
                        };
                        let (sa, sb) = if opts.mid_rev {
                            (at((t + 0.08).min(1.0)), at(t))
                        } else {
                            (at((t - 0.08).max(0.0)), at(t))
                        };
                        if (sb.0 - sa.0).hypot(sb.1 - sa.1) >= 1e-4 {
                            ops.push(format!(
                                "line({}, {}, {}, mark: (end: {}))",
                                fmt_pt(sa),
                                fmt_pt(sb),
                                stroke,
                                tip
                            ));
                        }
                    }
                    last = Some(e);
                    i = ni;
                    continue;
                }
                if let Some(w) = opts.underlay {
                    ops.push(format!(
                        "line({}, {}, stroke: {}pt + white)",
                        fmt_pt(p0),
                        fmt_pt(p1),
                        w
                    ));
                }
                if opts.mid_arrow {
                    // full line, then a short marked stub rides at the
                    // decoration position (reversed heads point back)
                    let t = opts.mid_pos;
                    let at = |t: f64| {
                        (p0.0 + t * (p1.0 - p0.0), p0.1 + t * (p1.1 - p0.1))
                    };
                    let (sa, sb) = if opts.mid_rev {
                        (at((t + 0.08).min(1.0)), at(t))
                    } else {
                        (at((t - 0.08).max(0.0)), at(t))
                    };
                    ops.push(format!("line({}, {}, {})", fmt_pt(p0), fmt_pt(p1), stroke));
                    if (sb.0 - sa.0).hypot(sb.1 - sa.1) >= 1e-4 {
                        ops.push(format!(
                            "line({}, {}, {}, mark: (end: {}))",
                            fmt_pt(sa),
                            fmt_pt(sb),
                            stroke,
                            tip
                        ));
                    }
                } else {
                    let m = if (p1.0 - p0.0).hypot(p1.1 - p0.1) < 1e-4 { "" } else { mark.as_str() };
                    ops.push(format!(
                        "line({}, {}, {}{})",
                        fmt_pt(p0),
                        fmt_pt(p1),
                        stroke,
                        m
                    ));
                }
            }
            if let Some((tex, lo)) = pending_label.take() {
                // labels ride the CURVE midpoint, not the chord
                let (bx, by) = match seg_curve {
                    Some((c1, c2)) => (
                        (p0.0 + 3.0 * c1.0 + 3.0 * c2.0 + p1.0) / 8.0,
                        (p0.1 + 3.0 * c1.1 + 3.0 * c2.1 + p1.1) / 8.0,
                    ),
                    None => ((p0.0 + p1.0) / 2.0, (p0.1 + p1.1) / 2.0),
                };
                let mx = bx + lo.xshift;
                let my = by + lo.yshift;
                let mut place = lo.place.as_deref().or(opts.place.as_deref());
                // tikz `auto`: beside the line, left of travel
                if place == Some("auto") {
                    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
                    let len = dx.hypot(dy).max(1e-6);
                    let (nx, ny) = (-dy / len, dx / len);
                    place = Some(if ny.abs() >= nx.abs() {
                        if ny > 0.0 { "above" } else { "below" }
                    } else if nx > 0.0 {
                        "right"
                    } else {
                        "left"
                    });
                }
                let (anchor, gx, gy) = place_anchor(place, unit);
                ops.push(format!(
                    "content(({:.3}, {:.3}), anchor: \"{}\", {})",
                    mx + gx,
                    my + gy,
                    anchor,
                    node_content(&tex, None)
                ));
            }
            last = Some(e);
            i = ni;
            continue;
        }
        if rest.starts_with("..") {
            // .. controls (a) and (b) .. (end)
            i += 2;
            let rest2: String = b[i..].iter().collect();
            if !rest2.trim_start().starts_with("controls") {
                return Err("unsupported .. path".into());
            }
            i += rest2.find("controls").unwrap() + 8;
            let (c1, ni) = read_end(&b, i).ok_or("bezier c1")?;
            i = ni;
            while i < b.len() && b[i].is_whitespace() {
                i += 1;
            }
            let rest3: String = b[i..].iter().collect();
            if rest3.trim_start().starts_with("and") {
                i += rest3.find("and").unwrap() + 3;
            }
            let (c2, ni) = read_end(&b, i).ok_or("bezier c2")?;
            i = ni;
            while i < b.len() && (b[i].is_whitespace() || b[i] == '.') {
                i += 1;
            }
            let (e, ni) = read_end(&b, i).ok_or("bezier end")?;
            let s0 = last.ok_or("bezier without start")?;
            ops.push(format!(
                "bezier(({:.3}, {:.3}), ({:.3}, {:.3}), ({:.3}, {:.3}), ({:.3}, {:.3}), {}{})",
                s0.x + cdx,
                s0.y + cdy,
                e.x + cdx,
                e.y + cdy,
                c1.x + cdx,
                c1.y + cdy,
                c2.x + cdx,
                c2.y + cdy,
                stroke,
                mark
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
            let mut lab_opts = Opts { scale: (1.0, 1.0), ..Default::default() };
            if b.get(i) == Some(&'[') {
                let mut j = i;
                while j < b.len() && b[j] != ']' {
                    j += 1;
                }
                lab_opts = parse_opts(&b[i..=j.min(b.len() - 1)].iter().collect::<String>(), unit);
                i = j + 1;
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
            let default_fill = if color != "black" { Some(color.as_str()) } else { None };
            let (anchor, gx, gy) = place_anchor(lab_opts.place.as_deref(), unit);
            ops.push(format!(
                "content(({:.3}, {:.3}), anchor: \"{}\", {})",
                at.x + cdx + gx + lab_opts.xshift,
                at.y + cdy + gy + lab_opts.yshift,
                anchor,
                node_content(&body, default_fill)
            ));
            continue;
        }
        // unknown token: skip a char (tolerate stray text)
        i += 1;
    }
    if fill_mode && !segs.is_empty() {
        let paint = opts
            .fill
            .clone()
            .or_else(|| opts.color.clone())
            .unwrap_or_else(|| "black".into());
        ops.push(format!(
            "merge-path(close: true, stroke: none, fill: {}, {{\n    {}\n  }})",
            paint,
            segs.join("\n    ")
        ));
    }
    if ops.is_empty() {
        Err("empty path".into())
    } else {
        Ok(ops)
    }
}

/// tikz placement keyword -> cetz content anchor + a small gap.
fn place_anchor(place: Option<&str>, unit: f64) -> (&'static str, f64, f64) {
    let g = 3.0 / unit;
    match place {
        Some("above") => ("south", 0.0, g),
        Some("below") => ("north", 0.0, -g),
        Some("left") => ("east", -g, 0.0),
        Some("right") => ("west", g, 0.0),
        Some("aboveleft") => ("south-east", -g, g),
        Some("aboveright") => ("south-west", g, g),
        Some("belowleft") => ("north-east", -g, -g),
        Some("belowright") => ("north-west", g, -g),
        _ => ("center", 0.0, 0.0),
    }
}

/// Peel `\color{..}` / `\scalebox{f}{..}` wrappers (any order, any
/// nesting) around a `$math$` node body. `default_fill` is the scope's
/// inherited color (tikz nodes inherit the ambient draw color).
fn node_content(body: &str, default_fill: Option<&str>) -> String {
    node_content_scaled(body, default_fill, 1.0)
}

fn node_content_scaled(body: &str, default_fill: Option<&str>, node_scale: f64) -> String {
    let mut fill: Option<String> = None;
    let mut size = node_scale;
    let mut boxed = false;
    let mut bg: Option<String> = None;
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
        if let Some(rest) = s.strip_prefix("\\large") {
            size *= 1.2;
            s = rest.to_string();
            continue;
        }
        // \colorbox{c}{X}: X on a painted background patch
        if let Some(rest) = s.strip_prefix("\\colorbox") {
            let rb: Vec<char> = rest.trim_start().chars().collect();
            if let Some((c, end)) = balanced(&rb, 0) {
                bg = Some(c.trim().to_string());
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
        if let Some(rest) = s.strip_prefix("\\fbox") {
            let rb: Vec<char> = rest.trim_start().chars().collect();
            if let Some((inner, _)) = balanced(&rb, 0) {
                boxed = true;
                s = inner;
                continue;
            }
        }
        break;
    }
    let math = s.trim().trim_start_matches('$').trim_end_matches('$');
    let math = crate::tikzcd::clean_cell_tex(math);
    let mut inner = format!("mi({})", emit::ts(&math));
    if (size - 1.0).abs() > 0.01 {
        inner = format!("text({:.2}em, {})", size, inner);
    }
    if let Some(f) = fill.or_else(|| default_fill.map(String::from)) {
        inner = format!("text(fill: {}, {})", f, inner);
    }
    if boxed || bg.is_some() {
        // measured box: inset-only boxes collapse around equations
        inner = format!(
            "nlab-tbox({}, {}, {})",
            if boxed { "0.5pt + black" } else { "none" },
            bg.unwrap_or_else(|| "white".into()),
            inner
        );
    }
    inner
}
