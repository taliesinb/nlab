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

/// A named node: center plus half-extents, all in canvas units.
#[derive(Clone, Copy)]
struct NamedNode {
    x: f64,
    y: f64,
    hw: f64,
    hh: f64,
}

// canvas length per tikz unit: true tikz is 1cm = 28.45pt, which the
// coordinate-graph pictures (named \node statements) are laid out
// for; the braid pictures were calibrated against the server renders
// at a tighter 19pt.
const UNIT_PT: f64 = 19.0;
const UNIT_PT_GRAPH: f64 = 28.45;

const COLORS: [&str; 9] = [
    "white", "black", "red", "blue", "green", "orange", "gray", "purple", "olive",
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
    let mut o = Opts { scale: (1.0, 1.0), ..Default::default() };
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
            o.scale.0 *= v.parse().unwrap_or(1.0);
        } else if let Some(v) = p.strip_prefix("yscale=") {
            o.scale.1 *= v.parse().unwrap_or(1.0);
        } else if let Some(v) = p.strip_prefix("scale=") {
            let f: f64 = v.parse().unwrap_or(1.0);
            o.scale.0 *= f;
            o.scale.1 *= f;
        } else if let Some(v) = p.strip_prefix("linewidth=") {
            o.width = len_pt(v);
        } else if p == "thick" {
            o.width = Some(0.8);
        } else if p == "verythick" {
            o.width = Some(1.2);
        } else if let Some(v) = p.strip_prefix("fill=") {
            if COLORS.contains(&v) {
                o.fill = Some(v.to_string());
            }
        } else if COLORS.contains(&p.as_str()) {
            o.color = Some(p);
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
    let unit = if body.contains("\\node") {
        UNIT_PT_GRAPH
    } else {
        UNIT_PT
    };
    let env = parse_opts(env_opts, unit);
    let root = Frame {
        sx: env.scale.0,
        sy: env.scale.1,
        tx: 0.0,
        ty: 0.0,
        color: env.color.unwrap_or_else(|| "black".into()),
        width: env.width.unwrap_or(0.4),
    };
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
            let mut f = frames.last().unwrap().child(o.shift, o.scale);
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
            let (_, ni) = read_stmt(&b, i);
            i = ni;
            warnings.push("tikzstyle".into());
            continue;
        }
        if starts(&b, i, "\\coordinate") || starts(&b, i, "\\node") {
            let is_coord = starts(&b, i, "\\coordinate");
            i += if is_coord { 11 } else { 5 };
            let (stmt, ni) = read_stmt(&b, i);
            i = ni;
            match parse_node_stmt(&stmt, frames.last().unwrap(), is_coord, unit) {
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
        if starts(&b, i, "\\draw") || starts(&b, i, "\\fill") {
            let is_fill = starts(&b, i, "\\fill");
            i += 5;
            let (optxt, ni) = read_bracket(&b, i);
            i = ni;
            let mut o = parse_opts(&optxt, unit);
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
            match parse_path(&stmt, &frame, &o, &nodes, unit) {
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
    if b.get(i) == Some(&'[') {
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
    if !(b.get(i) == Some(&'a') && b.get(i + 1) == Some(&'t')) {
        return Err("node without at".into());
    }
    i += 2;
    i = skip_ws(&b, i);
    if b.get(i) != Some(&'(') {
        return Err("node without coord".into());
    }
    let mut j = i + 1;
    while j < b.len() && b[j] != ')' {
        j += 1;
    }
    let inner: String = b[i + 1..j].iter().collect();
    let p = parse_pair(&format!("({inner})")).ok_or("bad node coord")?;
    let (x, y) = frame.apply(p);
    i = j + 1;
    i = skip_ws(&b, i);
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
    let trimmed = body.trim();
    if is_coord || trimmed.is_empty() {
        return Ok((name, NamedNode { x, y, hw: 0.0, hh: 0.0 }, None));
    }
    let hw = (crate::tikzcd::est_halfwidth_pt(trimmed) + 3.0) / unit;
    let hh = 8.0 / unit;
    let op = format!(
        "content(({:.3}, {:.3}), {})",
        x,
        y,
        node_content(trimmed, None)
    );
    Ok((name, NamedNode { x, y, hw, hh }, Some(op)))
}

fn parse_pair(s: &str) -> Option<(f64, f64)> {
    let s = s.trim().trim_start_matches('{').trim_start_matches('(');
    let inner: String = s.chars().take_while(|c| *c != ')').collect();
    if let Some((ang, rad)) = inner.split_once(':') {
        // polar (angle:radius)
        let a: f64 = ang.trim().parse().ok()?;
        let r: f64 = rad.trim().parse().ok()?;
        let a = a.to_radians();
        return Some((r * a.cos(), r * a.sin()));
    }
    let mut it = inner.split(',');
    let x: f64 = it.next()?.trim().parse().ok()?;
    let y: f64 = it.next()?.trim().parse().ok()?;
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
}

fn parse_path(
    path: &str,
    frame: &Frame,
    opts: &Opts,
    nodes: &HashMap<String, NamedNode>,
    unit: f64,
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
    let mark = match (opts.arrow_start, opts.arrow_end) {
        (true, true) => ", mark: (start: \">\", end: \">\", fill: black)".to_string(),
        (true, false) => ", mark: (start: \">\", fill: black)".to_string(),
        (false, true) => ", mark: (end: \">\", fill: black)".to_string(),
        _ => String::new(),
    };
    let (cdx, cdy) = opts.canvas_shift;

    let b: Vec<char> = path.chars().collect();
    let mut i = 0usize;
    let mut ops = Vec::new();
    let mut last: Option<EndPt> = None;
    // labels waiting to ride the NEXT segment (tikz `to node {..} (b)`)
    let mut pending_label: Option<(String, Opts)> = None;

    let read_end = |b: &[char], mut i: usize| -> Option<(EndPt, usize)> {
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
        let inner: String = b[i + 1..j].iter().collect();
        let inner = inner.trim();
        if let Some(n) = nodes.get(inner) {
            return Some((EndPt { x: n.x, y: n.y, hw: n.hw, hh: n.hh }, j + 1));
        }
        let p = parse_pair(&format!("({inner})"))?;
        let (x, y) = frame.apply(p);
        Some((EndPt { x, y, hw: 0.0, hh: 0.0 }, j + 1))
    };

    // trim `d` canvas units off point `p` toward `q`, plus p's border
    let trim = |p: EndPt, q: EndPt, extra: f64| -> (f64, f64) {
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        let len = dx.hypot(dy).max(1e-6);
        let (ux, uy) = (dx / len, dy / len);
        let border = if p.hw > 0.0 || p.hh > 0.0 {
            let k = (ux.abs() / p.hw.max(1e-6)).max(uy.abs() / p.hh.max(1e-6));
            (1.0 / k) + 2.0 / unit
        } else {
            0.0
        };
        let d = border + extra;
        (p.x + ux * d, p.y + uy * d)
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
        if rest.starts_with("to") || rest.starts_with("--") {
            i += 2;
            // optional to[bend ...] options
            let mut seg_bend = opts.bend;
            if b.get(i) == Some(&'[') {
                let mut j = i;
                while j < b.len() && b[j] != ']' {
                    j += 1;
                }
                let o = parse_opts(&b[i..=j.min(b.len() - 1)].iter().collect::<String>(), unit);
                if o.bend != 0.0 {
                    seg_bend = o.bend;
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
            let p0 = trim(s0, e, opts.shorten_start / unit);
            let p1 = trim(e, s0, opts.shorten_end / unit);
            let p0 = (p0.0 + cdx, p0.1 + cdy);
            let p1 = (p1.0 + cdx, p1.1 + cdy);
            if seg_bend != 0.0 {
                // symmetric quadratic-ish bezier: control points offset
                // perpendicular (left of travel = +90 deg CCW)
                let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
                let len = dx.hypot(dy).max(1e-6);
                let (nx, ny) = (-dy / len, dx / len);
                let sag = len * (seg_bend.abs().to_radians() / 2.0).sin() * 0.7
                    * seg_bend.signum();
                let c1 = (p0.0 + dx / 3.0 + nx * sag, p0.1 + dy / 3.0 + ny * sag);
                let c2 = (p0.0 + 2.0 * dx / 3.0 + nx * sag, p0.1 + 2.0 * dy / 3.0 + ny * sag);
                ops.push(format!(
                    "bezier({}, {}, {}, {}, {}{})",
                    fmt_pt(p0),
                    fmt_pt(p1),
                    fmt_pt(c1),
                    fmt_pt(c2),
                    stroke,
                    mark
                ));
            } else {
                ops.push(format!(
                    "line({}, {}, {}{})",
                    fmt_pt(p0),
                    fmt_pt(p1),
                    stroke,
                    mark
                ));
            }
            if let Some((tex, lo)) = pending_label.take() {
                let mx = (p0.0 + p1.0) / 2.0 + lo.xshift;
                let my = (p0.1 + p1.1) / 2.0 + lo.yshift;
                let (anchor, gx, gy) = place_anchor(lo.place.as_deref().or(opts.place.as_deref()), unit);
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
