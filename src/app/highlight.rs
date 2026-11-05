//! Highlighter for the chain language. Re-tokenising a few KB per frame costs
//! microseconds, so it runs every frame — which is what lets the playing step
//! of each pattern light up live.

use std::ops::Range;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId, Stroke};

use crate::chain::catalog::{self, Role};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Comment,
    Number,
    Quote,
    Word,
    Color,
    Source,
    Effect,
    Signal,
    Func,
    Punct,
}

pub struct SyntaxTheme {
    pub text: Color32,
    pub comment: Color32,
    pub number: Color32,
    pub quote: Color32,
    pub word: Color32,
    pub source: Color32,
    pub effect: Color32,
    pub signal: Color32,
    pub func: Color32,
    pub punct: Color32,
    pub active_bg: Color32,
    pub active_ink: Color32,
    pub error: Color32,
}

impl SyntaxTheme {
    fn color(&self, k: Kind) -> Color32 {
        match k {
            Kind::Text => self.text,
            Kind::Comment => self.comment,
            Kind::Number | Kind::Color => self.number,
            Kind::Quote => self.quote,
            Kind::Word => self.word,
            Kind::Source => self.source,
            Kind::Effect => self.effect,
            Kind::Signal => self.signal,
            Kind::Func => self.func,
            Kind::Punct => self.punct,
        }
    }
}

/// Shaders the sketch defines: name → is it a picture?
type Mine = std::collections::HashMap<String, bool>;

fn ident_kind(name: &str, after_dot: bool, mine: &Mine) -> Kind {
    if let Some(&picture) = mine.get(name) {
        return if picture { Kind::Source } else { Kind::Effect };
    }
    if after_dot {
        if catalog::find(name).is_some() || name == "blend" {
            return Kind::Effect;
        }
        if catalog::SIGNAL_METHODS.iter().any(|m| m.0 == name) || matches!(name, "x" | "y") {
            return Kind::Signal;
        }
        return Kind::Text;
    }
    match catalog::find(name) {
        Some(s) if s.role == Role::Source => Kind::Source,
        Some(_) => Kind::Effect,
        None if catalog::SIGNALS
            .iter()
            .any(|s| s.0.split('.').next() == Some(name)) =>
        {
            Kind::Signal
        }
        None if catalog::FUNCTIONS.iter().any(|f| f.0 == name) => Kind::Func,
        None => Kind::Text,
    }
}

const WGSL_KEYWORDS: &[&str] = &[
    "fn", "let", "var", "const", "return", "if", "else", "for", "while", "loop", "break", "continue",
    "continuing", "switch", "case", "default", "discard", "struct", "true", "false", "override", "alias",
];

fn wgsl_type(w: &str) -> bool {
    matches!(w, "f32" | "i32" | "u32" | "f16" | "bool" | "array" | "ptr" | "function" | "private")
        || ((w.starts_with("vec") || w.starts_with("mat")) && w.len() <= 7)
}

/// Native WGSL inside a shader block: keywords, types, numbers, the shader's own names.
fn wgsl_tokens(src: &str, r: Range<usize>, own: &[&str], out: &mut Vec<(Range<usize>, Kind)>) {
    let b = src.as_bytes();
    let mut i = r.start;
    while i < r.end {
        let c = b[i];
        let start = i;
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < r.end && b[i] != b'\n' {
                i += 1;
            }
            out.push((start..i, Kind::Comment));
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i = src[i + 2..r.end].find("*/").map_or(r.end, |k| i + 2 + k + 2);
            out.push((start..i, Kind::Comment));
            continue;
        }
        let prev_ident = i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        if !prev_ident
            && (c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)))
        {
            while i < r.end && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            out.push((start..i, Kind::Number));
            // exponents and suffixes (1e-3, 2u, 1.0f) stay out of the scrubbable number
            let s = i;
            if i < r.end && matches!(b[i], b'e' | b'E') {
                i += 1;
                if i < r.end && matches!(b[i], b'-' | b'+') {
                    i += 1;
                }
            }
            while i < r.end && b[i].is_ascii_alphanumeric() {
                i += 1;
            }
            if i > s {
                out.push((s..i, Kind::Number));
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < r.end && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let w = &src[start..i];
            let member = src[..start].ends_with('.');
            let kind = if member {
                Kind::Text
            } else if WGSL_KEYWORDS.contains(&w) {
                Kind::Func
            } else if wgsl_type(w) {
                Kind::Word
            } else if own.contains(&w) || matches!(w, "uv" | "pic" | "u") {
                Kind::Signal
            } else if crate::chain::native::SHADER_NAMES.iter().any(|n| n.0 == w) || src[i..r.end].trim_start().starts_with('(') {
                Kind::Effect
            } else {
                Kind::Text
            };
            out.push((start..i, kind));
            continue;
        }
        let w = src[i..].chars().next().map_or(1, char::len_utf8);
        i += w;
        out.push((start..i, if c.is_ascii_whitespace() { Kind::Text } else { Kind::Punct }));
    }
}

/// Classify every byte. Pure, so it is unit-tested.
pub fn tokens(src: &str) -> Vec<(Range<usize>, Kind)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut after_dot = false;
    let (defs, mut blocks) = crate::chain::native::outline(src);
    blocks.sort_by_key(|r| r.start);
    let mine: Mine = defs
        .iter()
        .map(|d| (d.name.clone(), d.kind == crate::chain::parse::DefKind::Picture))
        .collect();
    let mut next_block = blocks.iter().peekable();
    while i < b.len() {
        let c = b[i];
        let start = i;
        while next_block.peek().is_some_and(|r| r.start < i) {
            next_block.next();
        }
        if let Some(r) = next_block.peek()
            && r.start == i
        {
            let own: Vec<&str> = defs
                .iter()
                .find(|d| d.body == **r)
                .map(|d| d.params.iter().map(|p| p.0.as_str()).collect())
                .unwrap_or_default();
            let r = (*r).clone();
            wgsl_tokens(src, r.clone(), &own, &mut out);
            i = r.end;
            next_block.next();
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            out.push((start..i, Kind::Comment));
            continue;
        }
        if c == b'"' || c == b'\'' || c == b'`' {
            out.push((i..i + 1, Kind::Quote));
            i += 1;
            while i < b.len() && b[i] != c && b[i] != b'\n' {
                let s = i;
                let ch = b[i];
                if ch == b'#' {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_hexdigit() {
                        i += 1;
                    }
                    out.push((s..i, Kind::Color));
                } else if ch.is_ascii_digit()
                    || (ch == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
                    || (ch == b'-'
                        && b.get(i + 1)
                            .is_some_and(|d| d.is_ascii_digit() || *d == b'.'))
                {
                    i += 1;
                    while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                        i += 1;
                    }
                    out.push((s..i, Kind::Number));
                } else if ch.is_ascii_alphabetic() || ch == b'_' {
                    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                        i += 1;
                    }
                    out.push((s..i, Kind::Word));
                } else {
                    let w = src[i..].chars().next().map_or(1, char::len_utf8);
                    i += w;
                    out.push((s..i, Kind::Quote));
                }
            }
            if i < b.len() && b[i] == c {
                out.push((i..i + 1, Kind::Quote));
                i += 1;
            }
            after_dot = false;
            continue;
        }
        if c.is_ascii_digit()
            || (c == b'.'
                && b.get(i + 1).is_some_and(u8::is_ascii_digit)
                && !(i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b')')))
        {
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                if b[i] == b'.' && !b.get(i + 1).is_some_and(u8::is_ascii_digit) {
                    break;
                }
                i += 1;
            }
            out.push((start..i, Kind::Number));
            after_dot = false;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let w = &src[start..i];
            let rest = src[i..].trim_start();
            let line_start = src[..start].rsplit('\n').next().is_some_and(|l| l.trim().is_empty());
            let keyword = line_start
                && match w {
                    "picture" | "effect" => rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'),
                    "wgsl" => rest.starts_with('{'),
                    _ => false,
                };
            out.push((start..i, if keyword { Kind::Func } else { ident_kind(w, after_dot, &mine) }));
            after_dot = false;
            continue;
        }
        let w = src[i..].chars().next().map_or(1, char::len_utf8);
        i += w;
        if c.is_ascii_whitespace() {
            out.push((start..i, Kind::Text));
        } else {
            after_dot = c == b'.';
            out.push((start..i, Kind::Punct));
        }
    }
    out
}

/// Number literals (outside comments), for scrubbing.
pub fn numbers(src: &str) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for (r, k) in tokens(src) {
        if k != Kind::Number {
            continue;
        }
        // fold a leading minus: "-0.8" scrubs as one number
        let neg = r.start > 0 && src.as_bytes()[r.start - 1] == b'-' && {
            let before = src[..r.start - 1].trim_end();
            before.ends_with(['(', ',', ' ', '"', '\'', '[', '<', '='])
                || before.is_empty()
                || src.as_bytes()[r.start - 2..r.start - 1]
                    .iter()
                    .any(|c| matches!(c, b'(' | b',' | b' ' | b'"' | b'[' | b'<'))
        };
        out.push(if neg { r.start - 1..r.end } else { r });
    }
    out
}

pub fn layout(
    src: &str,
    font: FontId,
    theme: &SyntaxTheme,
    active: &[Range<usize>],
    error: Option<Range<usize>>,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (r, kind) in tokens(src) {
        // split each token at active / error boundaries
        let mut cuts = vec![r.start, r.end];
        for a in active.iter().chain(error.iter()) {
            for x in [a.start, a.end] {
                if x > r.start && x < r.end && src.is_char_boundary(x) {
                    cuts.push(x);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let (s, e) = (w[0], w[1]);
            let is_active = active
                .iter()
                .any(|a| a.start <= s && e <= a.end && a.start < a.end);
            let is_err = error
                .as_ref()
                .is_some_and(|x| x.start <= s && e <= x.end.max(x.start + 1));
            let mut fmt = TextFormat::simple(
                font.clone(),
                if is_active {
                    theme.active_ink
                } else {
                    theme.color(kind)
                },
            );
            if is_active {
                fmt.background = theme.active_bg;
            }
            if is_err {
                fmt.underline = Stroke::new(1.6, theme.error);
                fmt.color = theme.error;
            }
            job.append(&src[s..e], 0.0, fmt);
        }
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(&str, Kind)> {
        tokens(src)
            .into_iter()
            .filter(|(_, k)| *k != Kind::Text)
            .map(|(r, k)| (&src[r], k))
            .collect()
    }

    #[test]
    fn chains() {
        let k = kinds("mesh(\"dawn #fff\").flow(.3) // hi");
        assert_eq!(k[0], ("mesh", Kind::Source));
        assert!(k.contains(&("dawn", Kind::Word)));
        assert!(k.contains(&("#fff", Kind::Color)));
        assert!(k.contains(&("flow", Kind::Effect)));
        assert!(k.contains(&(".3", Kind::Number)));
        assert!(k.contains(&("// hi", Kind::Comment)));
    }

    #[test]
    fn signals_and_methods() {
        let k = kinds("sine.range(0, 2).slow(4) mouse.x slider(1)");
        assert_eq!(k[0], ("sine", Kind::Signal));
        assert!(k.contains(&("range", Kind::Signal)));
        assert!(k.contains(&("x", Kind::Signal)));
        assert!(k.contains(&("slider", Kind::Func)));
    }

    #[test]
    fn number_spans_for_scrubbing() {
        let s = "julia(\"x\", -0.8, .156) \"0 1.5\" // 9";
        let n: Vec<&str> = numbers(s).into_iter().map(|r| &s[r]).collect();
        assert_eq!(n, vec!["-0.8", ".156", "0", "1.5"]);
    }

    #[test]
    fn shader_blocks_highlight_as_wgsl() {
        let s = "picture rings(k = 2) {\n    let d = length(asp(uv)) * k; // r\n    return vec4f(vec3f(d), 1.0);\n}\nrings(3).blur()\neffect e() { return pic(uv); }\nmesh().e()";
        let k = kinds(s);
        assert_eq!(k[0], ("picture", Kind::Func));
        assert_eq!(k[1], ("rings", Kind::Source), "your picture is a picture");
        assert!(k.contains(&("let", Kind::Func)));
        assert!(k.contains(&("length", Kind::Effect)));
        assert!(k.contains(&("uv", Kind::Signal)) && k.contains(&("k", Kind::Signal)));
        assert!(k.contains(&("vec4f", Kind::Word)));
        assert!(k.contains(&("1.0", Kind::Number)));
        assert!(k.contains(&("// r", Kind::Comment)));
        assert!(k.contains(&("blur", Kind::Effect)));
        assert!(k.iter().filter(|x| x.0 == "e").all(|x| x.1 == Kind::Effect), "{k:?}");
        // still a variable when it's assigned
        assert!(!kinds("picture = 1").contains(&("picture", Kind::Func)));
    }

    #[test]
    fn covers_every_byte() {
        let s = "a(\"é ✓\") — x\npicture p() { return vec4f(1e-3u); /* é */ }\neffect q(";
        let t = tokens(s);
        let mut at = 0;
        for (r, _) in t {
            assert_eq!(r.start, at);
            at = r.end;
        }
        assert_eq!(at, s.len());
    }
}
