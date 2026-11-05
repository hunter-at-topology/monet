//! Shaders written in the sketch, in native WGSL.
//!
//! ```text
//! // rings that breathe (a comment above becomes its docs)
//! picture breathe(colors = "dawn", rings = 8, speed = .5) {
//!     let d = length(asp(uv));
//!     return vec4f(pal(colors, fract(d * rings - u.time * speed)), 1.0);
//! }
//!
//! effect ripple(amount = .02) {
//!     return pic(uv + sin(uv.yx * 40.0 + u.time) * amount);
//! }
//!
//! breathe("ember").ripple(sine.range(0, .03))
//! ```
//!
//! A `picture` returns the colour at `uv`; an `effect` returns the new colour at
//! `uv` and reads the picture so far with `pic(p)`, anywhere, so one kind covers
//! both colour effects and warps. Parameters take their type from their default:
//! a number is an `f32` fed by numbers, patterns and signals; a palette name is
//! a `u32` read with `pal(colors, t)`; a picture is called like a function.
//!
//! Each shader is checked with naga on its own as it's written, so a mistake is
//! underlined in the sketch at the line that made it, whether or not the shader
//! is used yet.

use std::fmt::Write;
use std::ops::Range;

use super::Error;
use super::build::{Helper, UArg, UserFn};
use super::parse::DefKind;

/// Marks where sketch code starts in generated WGSL: `// @user <byte offset>`.
pub const USER_MARK: &str = "// @user ";
const END_MARK: &str = "// @end";
/// On a generated function's header line: `// @def <start> <end>` (the name).
const DEF_MARK: &str = "// @def ";

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Rename whole identifiers (not members after `.`, not inside comments).
pub fn rename(body: &str, map: &[(&str, String)]) -> String {
    if map.is_empty() {
        return body.to_string();
    }
    let b = body.as_bytes();
    let mut out = String::with_capacity(body.len() + 32);
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            let e = body[i..].find('\n').map_or(b.len(), |k| i + k);
            out.push_str(&body[i..e]);
            i = e;
            continue;
        }
        if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            let e = body[i + 2..].find("*/").map_or(b.len(), |k| i + 2 + k + 2);
            out.push_str(&body[i..e]);
            i = e;
            continue;
        }
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let s = i;
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            let word = &body[s..i];
            let member = body[..s].trim_end().ends_with('.');
            match map.iter().find(|(from, _)| *from == word) {
                Some((_, to)) if !member => out.push_str(to),
                _ => out.push_str(word),
            }
            continue;
        }
        let w = body[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&body[i..i + w]);
        i += w;
    }
    out
}

/// Write a sketch shader as a WGSL function `fname(uv, params…) -> vec4f`, marked so
/// errors can be traced back to the sketch. Picture parameters and `pic` are renamed
/// to the functions that draw them.
pub fn emit(out: &mut String, fname: &str, f: &UserFn, renames: &[(&str, String)]) {
    let mut params = String::new();
    for p in &f.params {
        match p.kind {
            UArg::Num => {
                let _ = write!(params, ", {}: f32", p.name);
            }
            UArg::Pal => {
                let _ = write!(params, ", {}: u32", p.name);
            }
            UArg::Pic => {}
        }
    }
    let _ = writeln!(
        out,
        "fn {fname}(uv: vec2f{params}) -> vec4f {{ {DEF_MARK}{} {}",
        f.name_span.start, f.name_span.end
    );
    let _ = writeln!(out, "{USER_MARK}{}", f.body_start);
    out.push_str(&rename(&f.body, renames));
    let _ = writeln!(out, "\n{END_MARK}\n}}");
}

/// The sketch's `wgsl { … }` blocks, marked.
pub fn helpers(hs: &[Helper]) -> String {
    let mut out = String::new();
    for h in hs {
        let _ = writeln!(out, "{USER_MARK}{}\n{}\n{END_MARK}", h.start, h.code);
    }
    out
}

/// A shader on its own, with stand-ins for what it reads, for checking.
fn standalone(f: Option<&UserFn>, hs: &[Helper]) -> String {
    let mut body = helpers(hs);
    if let Some(f) = f {
        let mut renames: Vec<(&str, String)> = Vec::new();
        let stand_in = |name: &str| format!("fn {name}(p: vec2f) -> vec4f {{ return vec4f(p, 0.0, 1.0); }}\n");
        if f.kind == DefKind::Effect {
            body.push_str(&stand_in("sketch_pic"));
            renames.push(("pic", "sketch_pic".into()));
        }
        for (i, p) in f.params.iter().enumerate() {
            if p.kind == UArg::Pic {
                let n = format!("sketch_p{i}");
                body.push_str(&stand_in(&n));
                renames.push((p.name.as_str(), n));
            }
        }
        emit(&mut body, "sketch_shader", f, &renames);
    }
    format!("{}\n{body}", super::codegen::shake(&body))
}

/// Strip naga's internal handles ("Function [3]") from messages.
pub fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        match rest[i..].find(']') {
            Some(j) if rest[i + 1..i + j].chars().all(|c| c.is_ascii_digit()) => {
                rest = &rest[i + j + 1..];
                if out.ends_with(' ') && rest.starts_with([' ', ',', '.', ')']) {
                    out.pop();
                }
            }
            _ => {
                out.push('[');
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

type NagaError = (Option<Range<usize>>, String);

/// Parse and validate with naga: where it went wrong (in `text`) and why.
fn naga_error(text: &str) -> Option<NagaError> {
    let module = match naga::front::wgsl::parse_str(text) {
        Ok(m) => m,
        Err(e) => {
            let span = e.labels().next().and_then(|(s, _)| s.to_range());
            return Some((span, e.message().to_string()));
        }
    };
    let err = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .err()?;
    // the most specific place it points at
    let span = err
        .spans()
        .filter_map(|(s, _)| s.to_range())
        .filter(|r| !r.is_empty())
        .min_by_key(|r| r.len());
    let mut msg = err.as_inner().to_string();
    let mut cur: Option<&dyn std::error::Error> = std::error::Error::source(err.as_inner());
    while let Some(c) = cur {
        msg = c.to_string();
        cur = c.source();
    }
    Some((span, tidy(&msg)))
}

/// naga, remembered: the same shader text is never checked twice.
fn naga_cached(text: &str) -> Option<NagaError> {
    thread_local! {
        static SEEN: std::cell::RefCell<std::collections::HashMap<String, Option<NagaError>>> =
            Default::default();
    }
    if let Some(hit) = SEEN.with(|s| s.borrow().get(text).cloned()) {
        return hit;
    }
    let r = naga_error(text);
    SEEN.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() > 64 {
            s.clear();
        }
        s.insert(text.to_string(), r.clone());
    });
    r
}

/// Where in the sketch a byte range of generated `text` came from.
fn locate(text: &str, at: &Range<usize>, src: &str) -> Option<Range<usize>> {
    let start = at.start.min(text.len());
    let line_no = text[..start].matches('\n').count();
    let lines: Vec<&str> = text.split('\n').collect();
    let line = lines.get(line_no)?;
    if let Some(k) = line.find(DEF_MARK) {
        let mut n = line[k + DEF_MARK.len()..].split_whitespace().map(str::parse::<usize>);
        if let (Some(Ok(a)), Some(Ok(b))) = (n.next(), n.next()) {
            return Some(a..b);
        }
    }
    let mut marker = None;
    for j in (0..line_no).rev() {
        let l = lines[j].trim_start();
        if l.starts_with(END_MARK) || l.contains(DEF_MARK) {
            return None;
        }
        if let Some(n) = l.strip_prefix(USER_MARK) {
            marker = Some((j, n.trim().parse::<usize>().ok()?));
            break;
        }
    }
    let (j, base) = marker?;
    // the k-th line of the code as written
    let k = line_no - j - 1;
    let mut off = base;
    for _ in 0..k {
        off += src.get(off..)?.find('\n')? + 1;
    }
    let src_line_end = src[off..].find('\n').map_or(src.len(), |e| off + e);
    let src_line = &src[off..src_line_end];
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let col = start - line_start;
    // the same text in the line as written (renames only change other words)
    let want = &text[start..at.end.min(text.len()).min(line_start + line.len()).max(start)];
    if !want.trim().is_empty()
        && let Some(c) = src_line.match_indices(want).map(|(c, _)| c).min_by_key(|c| c.abs_diff(col))
    {
        return Some(off + c..off + c + want.len());
    }
    let lead = src_line.len() - src_line.trim_start().len();
    let trail = src_line.trim_end().len();
    Some(off + lead..off + trail.max(lead + 1).min(src_line.len().max(lead + 1)))
}

/// Make naga's message speak the sketch's language where we can.
fn advise(msg: &str, kind: Option<DefKind>) -> String {
    let unknown = msg
        .strip_prefix("no definition in scope for identifier: ")
        .map(|s| s.trim().trim_matches('`'));
    if let Some(n) = unknown {
        return match n {
            "time" | "cycle" | "frame" | "dt" | "mouse" | "res" => format!("`{n}` is u.{n} in a shader"),
            "resolution" => "the canvas size is u.res in a shader".into(),
            "pic" if kind != Some(DefKind::Effect) => {
                "only effects have pic(): it reads the picture so far".into()
            }
            "c" | "col" | "color" | "colour" if kind == Some(DefKind::Effect) => {
                format!("unknown name `{n}`. Read the picture so far with pic(uv)")
            }
            _ => {
                let hint = super::suggest(n, SHADER_NAMES.iter().map(|s| s.0))
                    .map(|s| format!(". Did you mean `{s}`?"))
                    .unwrap_or_default();
                format!("unknown name `{n}`{hint}")
            }
        };
    }
    // naga's own words where they read badly
    if msg.contains("`return` expression None") {
        return "this shader never returns. End it with return vec4f(rgb, 1.0)".into();
    }
    if msg.contains("does not match the declared return type") {
        return "this isn't a colour. Pictures and effects return a vec4f: vec4f(rgb, 1.0)".into();
    }
    format!("WGSL: {msg}")
}

/// Check every shader written in the sketch, and its `wgsl` helpers.
pub fn check(defs: &[std::rc::Rc<UserFn>], hs: &[Helper], src: &str) -> Result<(), Error> {
    let fallback = |f: Option<&UserFn>| match f {
        Some(f) => f.name_span.clone(),
        None => hs.first().map_or(0..0, |h| h.start.saturating_sub(1)..h.start),
    };
    let mut todo: Vec<Option<&UserFn>> = defs.iter().map(|f| Some(&**f)).collect();
    if todo.is_empty() && !hs.is_empty() {
        todo.push(None);
    }
    for f in todo {
        let text = standalone(f, hs);
        if let Some((at, msg)) = naga_cached(&text) {
            let span = at
                .and_then(|r| locate(&text, &r, src))
                .unwrap_or_else(|| fallback(f));
            return Err(Error::new(span, advise(&msg, f.map(|f| f.kind))));
        }
    }
    Ok(())
}

// ── for the editor: a forgiving outline of what the sketch defines ───────────

/// A `picture` or `effect` in the sketch, found without compiling it.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub kind: DefKind,
    pub name: String,
    pub name_span: Range<usize>,
    /// (name, default as typed)
    pub params: Vec<(String, Option<String>)>,
    /// The `//` lines right above it.
    pub doc: String,
    /// Inside the braces (to the end of the sketch while it's still open).
    pub body: Range<usize>,
}

impl Outline {
    pub fn signature(&self) -> String {
        let args: Vec<String> = self
            .params
            .iter()
            .map(|(n, d)| d.as_ref().map_or_else(|| n.clone(), |d| format!("{n} = {d}")))
            .collect();
        format!("{}({})", self.name, args.join(", "))
    }

    pub fn summary(&self) -> String {
        if !self.doc.is_empty() {
            return self.doc.clone();
        }
        match self.kind {
            DefKind::Picture => "your picture, written in WGSL in this sketch".into(),
            DefKind::Effect => "your effect, written in WGSL in this sketch".into(),
        }
    }
}

/// What a parameter takes, from its default as typed.
pub fn param_kind(default: Option<&str>, picture: impl Fn(&str) -> bool) -> UArg {
    let Some(d) = default.map(str::trim) else { return UArg::Num };
    if d.starts_with(['"', '\'', '`']) {
        let inner = d.trim_matches(['"', '\'', '`']);
        let numeric = inner.chars().all(|c| c.is_ascii_digit() || " .-<>[]~*!@,".contains(c));
        return if numeric && !inner.trim().is_empty() { UArg::Num } else { UArg::Pal };
    }
    let root: String = d.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    if !root.is_empty() && picture(&root) { UArg::Pic } else { UArg::Num }
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// The `)` closing the `(` at `open`, skipping strings.
fn close_paren(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut q: Option<u8> = None;
    for (i, &c) in b.iter().enumerate().skip(open) {
        if let Some(qc) = q {
            if c == qc {
                q = None;
            }
            continue;
        }
        match c {
            b'"' | b'\'' | b'`' => q = Some(c),
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'\n' if depth == 0 => return None,
            _ => {}
        }
    }
    None
}

/// Split `a = 1, b = f(1, 2)` at top-level commas.
fn split_params(s: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let (mut depth, mut q, mut cur) = (0i32, None::<char>, String::new());
    let mut push = |cur: &mut String| {
        let t = cur.trim();
        if !t.is_empty() {
            let (n, d) = match t.split_once('=') {
                Some((n, d)) => (n.trim().to_string(), Some(d.trim().to_string())),
                None => (t.to_string(), None),
            };
            out.push((n, d));
        }
        cur.clear();
    };
    for c in s.chars() {
        match (q, c) {
            (Some(qc), _) if c == qc => q = None,
            (Some(_), _) => {}
            (None, '"' | '\'' | '`') => q = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, ',') if depth == 0 => {
                push(&mut cur);
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    push(&mut cur);
    out
}

/// The `//` lines directly above byte `at` (its line start), joined.
fn doc_above(src: &str, at: usize) -> String {
    let mut lines = Vec::new();
    let mut end = src[..at].rfind('\n').map_or(0, |i| i + 1);
    while end > 0 {
        let s = src[..end - 1].rfind('\n').map_or(0, |i| i + 1);
        let l = src[s..end - 1].trim();
        let Some(text) = l.strip_prefix("//") else { break };
        lines.push(text.trim().to_string());
        end = s;
    }
    lines.reverse();
    lines.join(" ")
}

/// Shaders and `wgsl` blocks in the sketch: (definitions, every block's inside).
pub fn outline(src: &str) -> (Vec<Outline>, Vec<Range<usize>>) {
    let b = src.as_bytes();
    let (mut defs, mut blocks) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < b.len() {
        let line_end = src[i..].find('\n').map_or(b.len(), |k| i + k);
        let s = skip_ws(&b[..line_end], i);
        let word_end = (s..line_end).find(|&k| !is_ident(b[k])).unwrap_or(line_end);
        let word = &src[s..word_end];
        let next = i;
        i = line_end + 1;
        let open_block = |from: usize| -> Option<(usize, usize)> {
            let o = skip_ws(b, from);
            (b.get(o) == Some(&b'{')).then(|| (o, super::lex::block_end(b, o).unwrap_or(b.len())))
        };
        match word {
            "wgsl" => {
                if let Some((o, e)) = open_block(word_end) {
                    blocks.push(o + 1..e);
                    i = e + 1;
                }
            }
            "picture" | "effect" => {
                let ns = skip_ws(&b[..line_end], word_end);
                let ne = (ns..line_end).find(|&k| !is_ident(b[k])).unwrap_or(line_end);
                if ne == ns || b[ns].is_ascii_digit() {
                    continue;
                }
                let po = skip_ws(&b[..line_end], ne);
                if b.get(po) != Some(&b'(') {
                    continue;
                }
                let pc = close_paren(b, po);
                let params = split_params(&src[po + 1..pc.unwrap_or(line_end)]);
                let (body, after) = match pc.and_then(|pc| open_block(pc + 1)) {
                    Some((o, e)) => (o + 1..e, e + 1),
                    None => (line_end..line_end, line_end + 1),
                };
                if !body.is_empty() || after > line_end + 1 {
                    blocks.push(body.clone());
                }
                defs.push(Outline {
                    kind: if word == "picture" { DefKind::Picture } else { DefKind::Effect },
                    name: src[ns..ne].to_string(),
                    name_span: ns..ne,
                    params,
                    doc: doc_above(src, next),
                    body,
                });
                i = i.max(after);
            }
            _ => {}
        }
    }
    (defs, blocks)
}

/// The block (inside its braces) that byte `at` is in, if any.
pub fn block_at(src: &str, at: usize) -> Option<Range<usize>> {
    outline(src).1.into_iter().find(|r| r.start <= at && at <= r.end)
}

/// What a shader can use, for completion, hover and the reference.
/// (name, signature, what it is)
pub const SHADER_NAMES: &[(&str, &str, &str)] = &[
    ("uv", "uv: vec2f", "where this pixel is: 0 to 1 across the frame, y up"),
    ("pic", "pic(p: vec2f) -> vec4f", "the picture so far, at any position (effects only)"),
    ("u", "u", "the frame: u.time, u.cycle, u.mouse, u.res, u.dt, u.frame"),
    ("pal", "pal(colors: u32, t: f32) -> vec3f", "a smooth colour from a palette parameter, t from 0 to 1"),
    ("palc", "palc(colors: u32, i: u32) -> vec3f", "the i-th colour of a palette parameter"),
    ("paln", "paln(colors: u32) -> u32", "how many colours a palette parameter has"),
    ("asp", "asp(uv) -> vec2f", "centred and aspect-correct: y from -0.5 to 0.5, x wider on wide screens"),
    ("unasp", "unasp(p) -> vec2f", "back from asp() to uv"),
    ("plane", "plane(uv) -> vec2f", "the pannable, zoomable plane fractals use (drag and scroll the canvas)"),
    ("rot", "rot(angle) -> mat2x2f", "a rotation by angle (radians): rot(a) * p"),
    ("noise", "noise(p: vec2f) -> f32", "smooth gradient noise, about 0 to 1"),
    ("noise3", "noise3(p: vec2f, t: f32) -> f32", "noise that evolves smoothly with t"),
    ("fbm", "fbm(p: vec2f, octaves: i32) -> f32", "layered noise, cloud-like; 4 to 6 octaves"),
    ("voronoi", "voronoi(p: vec2f, t: f32) -> vec3f", "cells: x is the distance to the nearest, y to the second, z an id"),
    ("hash21", "hash21(p: vec2f) -> f32", "a random number per integer cell, 0 to 1"),
    ("hash22", "hash22(p: vec2f) -> vec2f", "two random numbers per integer cell"),
    ("hash11", "hash11(x: f32) -> f32", "a random number per integer, 0 to 1"),
    ("luma", "luma(rgb: vec3f) -> f32", "brightness of a colour"),
    ("sat", "sat(x: f32) -> f32", "clamp to 0..1"),
    ("tri", "tri(x: f32) -> f32", "a triangle wave, 0 to 1 and back"),
    ("srgb_to_oklab", "srgb_to_oklab(rgb) -> vec3f", "to OKLab, where mixing colours looks even"),
    ("oklab_to_srgb", "oklab_to_srgb(lab) -> vec3f", "back from OKLab"),
    ("aspect", "aspect() -> f32", "width over height"),
    ("PI", "PI", "3.14159…"),
    ("TAU", "TAU", "2π, a full turn"),
    ("mix", "mix(a, b, t)", "blend from a to b"),
    ("smoothstep", "smoothstep(lo, hi, x)", "a soft 0 to 1 step between lo and hi"),
    ("step", "step(edge, x)", "0 below edge, 1 above"),
    ("clamp", "clamp(x, lo, hi)", "keep x between lo and hi"),
    ("fract", "fract(x)", "the part after the decimal point: repeats every 1"),
    ("floor", "floor(x)", "round down"),
    ("abs", "abs(x)", "distance from zero"),
    ("length", "length(v)", "how long a vector is"),
    ("distance", "distance(a, b)", "how far apart two points are"),
    ("dot", "dot(a, b)", "dot product"),
    ("normalize", "normalize(v)", "the same direction, length 1"),
    ("sin", "sin(x)", "sine, radians"),
    ("cos", "cos(x)", "cosine, radians"),
    ("atan2", "atan2(y, x)", "the angle of a point, -π to π"),
    ("pow", "pow(x, y)", "x to the power y"),
    ("exp", "exp(x)", "e to the power x"),
    ("min", "min(a, b)", "the smaller"),
    ("max", "max(a, b)", "the larger"),
    ("select", "select(f, t, cond)", "t if cond, else f"),
    ("vec2f", "vec2f(x, y)", "a 2D vector"),
    ("vec3f", "vec3f(r, g, b)", "a 3D vector, or a colour"),
    ("vec4f", "vec4f(rgb, a)", "a colour with alpha: what shaders return"),
    ("f32", "f32(x)", "a number"),
    ("let", "let name = value;", "a constant"),
    ("var", "var name = value;", "a variable you can change"),
    ("return", "return value;", "the answer: a vec4f colour"),
    ("for", "for (var i = 0; i < 8; i++) { }", "a loop"),
    ("if", "if (cond) { } else { }", "a choice"),
];

/// Fields of `u`.
pub const UNIFORMS: &[(&str, &str)] = &[
    ("time", "seconds since the sketch started"),
    ("cycle", "cycles so far: time × speed(), what patterns and signals follow"),
    ("mouse", "the pointer, 0 to 1 (vec2f)"),
    ("res", "the canvas size in pixels (vec2f)"),
    ("dt", "seconds since the last frame"),
    ("frame", "frames so far"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_whole_words_only() {
        let r = rename(
            "let p = pic(uv) + other(uv).x; // pic(\n  return pics + c.pic;",
            &[("pic", "l0s1".into()), ("other", "l0p1s0".into())],
        );
        assert_eq!(r, "let p = l0s1(uv) + l0p1s0(uv).x; // pic(\n  return pics + c.pic;");
    }

    #[test]
    fn tidy_drops_handles() {
        assert_eq!(tidy("Function [3] 'x' is invalid"), "Function 'x' is invalid");
        assert_eq!(tidy("a [x]"), "a [x]");
    }

    #[test]
    fn outline_finds_shaders_while_typing() {
        let src = "// soft rings\n// that breathe\npicture breathe(colors = \"dawn\", n = f(1, 2), x) {\n  return vec4f(1.0);\n}\nwgsl { fn k() {} }\neffect half(";
        let (defs, blocks) = outline(src);
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "breathe");
        assert_eq!(defs[0].doc, "soft rings that breathe");
        assert_eq!(
            defs[0].params,
            vec![
                ("colors".into(), Some("\"dawn\"".into())),
                ("n".into(), Some("f(1, 2)".into())),
                ("x".into(), None)
            ]
        );
        assert_eq!(&src[defs[0].body.clone()], "\n  return vec4f(1.0);\n");
        assert_eq!(defs[0].signature(), "breathe(colors = \"dawn\", n = f(1, 2), x)");
        assert_eq!(defs[1].name, "half");
        assert_eq!(blocks.len(), 2, "{blocks:?}");
        assert!(src[blocks[1].clone()].contains("fn k"));
    }

    #[test]
    fn parameter_kinds_follow_defaults() {
        let pic = |n: &str| n == "clouds";
        assert_eq!(param_kind(None, pic), UArg::Num);
        assert_eq!(param_kind(Some(".5"), pic), UArg::Num);
        assert_eq!(param_kind(Some("sine.range(0, 1)"), pic), UArg::Num);
        assert_eq!(param_kind(Some("\"0 .5 1\""), pic), UArg::Num);
        assert_eq!(param_kind(Some("\"dawn\""), pic), UArg::Pal);
        assert_eq!(param_kind(Some("\"#fff #000\""), pic), UArg::Pal);
        assert_eq!(param_kind(Some("clouds().blur()"), pic), UArg::Pic);
    }
}
