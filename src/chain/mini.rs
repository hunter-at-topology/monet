//! Mini-notation: a little rhythm language for values that change over time.
//!
//! ```text
//! "0 .5 1"        a sequence — each value gets an equal slice of a cycle
//! "<dawn ember>"  one per cycle, alternating
//! "[0 1] 2"       brackets nest: [0 1] shares the first half
//! "0*2 1"         * repeats faster, / stretches slower
//! "1@3 0"         @ gives a step more weight (3/4 vs 1/4 of the cycle)
//! "1!3 0"         ! replicates (same as "1 1 1 0")
//! "1 ~"           ~ is a rest (0 for numbers)
//! ```
//!
//! Spans are absolute byte ranges into the sketch, so the editor can light up
//! the step that is playing.

use std::ops::Range;

use super::Error;

#[derive(Clone, Debug, PartialEq)]
pub enum Atom {
    Num(f64),
    Color([f32; 3]),
    Word(String),
    Rest,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Atom(Atom),
    Seq(Vec<(Node, f64)>),
    Alt(Vec<Node>),
    Fast(Box<Node>, f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit<'a> {
    pub atom: &'a Atom,
    pub span: Range<usize>,
    pub begin: f64,
    pub end: f64,
}

pub fn parse_hex(hex: &str) -> Option<[f32; 3]> {
    let v = u32::from_str_radix(hex, 16).ok()?;
    let rgb = match hex.len() {
        6 => [(v >> 16) & 255, (v >> 8) & 255, v & 255],
        3 => [((v >> 8) & 15) * 17, ((v >> 4) & 15) * 17, (v & 15) * 17],
        _ => return None,
    };
    Some(rgb.map(|x| x as f32 / 255.0))
}

struct Parser<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
    base: usize,
}

impl Parser<'_> {
    fn err(&self, at: Range<usize>, msg: impl Into<String>) -> Error {
        Error::new(self.base + at.start..self.base + at.end, msg)
    }
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn number(&mut self) -> Option<f64> {
        let start = self.i;
        let mut j = self.i;
        if j < self.s.len() && self.s[j] == b'-' {
            j += 1;
        }
        let digits = j;
        while j < self.s.len() && (self.s[j].is_ascii_digit() || self.s[j] == b'.') {
            j += 1;
        }
        if j == digits {
            return None;
        }
        let v = self.src[start..j].parse().ok()?;
        self.i = j;
        Some(v)
    }

    fn seq(&mut self, close: Option<u8>) -> Result<Vec<(Node, f64)>, Error> {
        let mut steps: Vec<(Node, f64)> = Vec::new();
        loop {
            self.ws();
            let Some(&c) = self.s.get(self.i) else {
                if let Some(cl) = close {
                    return Err(self.err(self.i..self.i, format!("missing `{}`", cl as char)));
                }
                return Ok(steps);
            };
            if Some(c) == close {
                self.i += 1;
                return Ok(steps);
            }
            match c {
                b']' | b'>' => {
                    return Err(self.err(self.i..self.i + 1, format!("unexpected `{}`", c as char)));
                }
                b',' => {
                    return Err(self.err(
                        self.i..self.i + 1,
                        "`,` (chords) isn't supported. Values are one at a time",
                    ));
                }
                b'!' => {
                    // bare `!` repeats the previous step
                    let at = self.i;
                    self.i += 1;
                    let Some(last) = steps.last().cloned() else {
                        return Err(self.err(at..at + 1, "`!` needs a step before it"));
                    };
                    steps.push(last);
                    continue;
                }
                _ => {}
            }
            let mut node = self.term()?;
            let mut weight = 1.0;
            let mut copies = 1usize;
            while let Some(&m) = self.s.get(self.i) {
                if !matches!(m, b'*' | b'/' | b'@' | b'!') {
                    break;
                }
                let at = self.i;
                self.i += 1;
                let n = self.number();
                match (m, n) {
                    (b'*', Some(k)) if k > 0.0 => node = wrap_fast(node, k),
                    (b'/', Some(k)) if k > 0.0 => node = wrap_fast(node, 1.0 / k),
                    (b'@', Some(k)) if k > 0.0 => weight = k,
                    (b'!', Some(k)) if k >= 1.0 => copies = k as usize,
                    (b'!', None) => copies += 1,
                    _ => {
                        return Err(self.err(
                            at..self.i,
                            format!("`{}` needs a positive number after it", m as char),
                        ));
                    }
                }
            }
            for _ in 0..copies.min(64) {
                steps.push((node.clone(), weight));
            }
        }
    }

    fn term(&mut self) -> Result<Node, Error> {
        let start = self.i;
        let c = self.s[self.i];
        let kind = match c {
            b'[' => {
                self.i += 1;
                let steps = self.seq(Some(b']'))?;
                if steps.is_empty() {
                    return Err(self.err(start..self.i, "empty `[]`"));
                }
                NodeKind::Seq(steps)
            }
            b'<' => {
                self.i += 1;
                let steps = self.seq(Some(b'>'))?;
                if steps.is_empty() {
                    return Err(self.err(start..self.i, "empty `<>`"));
                }
                NodeKind::Alt(steps.into_iter().map(|(n, _)| n).collect())
            }
            b'~' => {
                self.i += 1;
                NodeKind::Atom(Atom::Rest)
            }
            b'#' => {
                self.i += 1;
                while self.i < self.s.len() && self.s[self.i].is_ascii_hexdigit() {
                    self.i += 1;
                }
                let hex = &self.src[start + 1..self.i];
                let rgb = parse_hex(hex).ok_or_else(|| {
                    self.err(
                        start..self.i,
                        format!("`#{hex}` isn't a colour. Use #rrggbb or #rgb"),
                    )
                })?;
                NodeKind::Atom(Atom::Color(rgb))
            }
            _ if c.is_ascii_digit() || c == b'-' || c == b'.' => match self.number() {
                Some(v) => NodeKind::Atom(Atom::Num(v)),
                None => {
                    return Err(self.err(start..start + 1, format!("unexpected `{}`", c as char)));
                }
            },
            _ if c.is_ascii_alphabetic() || c == b'_' => {
                while self.i < self.s.len()
                    && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_')
                {
                    self.i += 1;
                }
                NodeKind::Atom(Atom::Word(self.src[start..self.i].to_string()))
            }
            _ => {
                let ch = self.src[start..].chars().next().unwrap_or('?');
                return Err(self.err(
                    start..start + ch.len_utf8(),
                    format!("unexpected `{ch}` in pattern"),
                ));
            }
        };
        Ok(Node {
            kind,
            span: self.base + start..self.base + self.i,
        })
    }
}

fn wrap_fast(node: Node, k: f64) -> Node {
    let span = node.span.clone();
    Node {
        kind: NodeKind::Fast(Box::new(node), k),
        span,
    }
}

/// Parse a pattern string. `base` is the absolute offset of its first byte.
pub fn parse(src: &str, base: usize) -> Result<Node, Error> {
    let mut p = Parser {
        s: src.as_bytes(),
        src,
        i: 0,
        base,
    };
    let steps = p.seq(None)?;
    if steps.is_empty() {
        return Err(Error::new(
            base.saturating_sub(1)..base + 1,
            "empty pattern",
        ));
    }
    let span = base..base + src.len();
    if steps.len() == 1 && steps[0].1 == 1.0 {
        return Ok(steps.into_iter().next().unwrap().0);
    }
    Ok(Node {
        kind: NodeKind::Seq(steps),
        span,
    })
}

impl Node {
    /// What is playing at time `t` (in cycles)?
    pub fn query(&self, t: f64) -> Hit<'_> {
        let c = t.floor();
        match &self.kind {
            NodeKind::Atom(a) => Hit {
                atom: a,
                span: self.span.clone(),
                begin: c,
                end: c + 1.0,
            },
            NodeKind::Seq(steps) => {
                let total: f64 = steps.iter().map(|s| s.1).sum();
                let f = (t - c) * total;
                let mut acc = 0.0;
                let mut pick = steps.len() - 1;
                for (i, (_, w)) in steps.iter().enumerate() {
                    if f < acc + w {
                        pick = i;
                        break;
                    }
                    acc += w;
                }
                if pick == steps.len() - 1 {
                    acc = total - steps[pick].1;
                }
                let (a, b) = (acc / total, (acc + steps[pick].1) / total);
                let local = c + ((t - c - a) / (b - a)).clamp(0.0, 0.999_999_999);
                let h = steps[pick].0.query(local);
                let map = |x: f64| c + a + (x - c) * (b - a);
                Hit {
                    begin: map(h.begin).max(c + a),
                    end: map(h.end).min(c + b),
                    ..h
                }
            }
            NodeKind::Alt(items) => {
                let n = items.len() as f64;
                let k = c.rem_euclid(n) as usize;
                let inner = (c / n).floor();
                let h = items[k].query(inner + (t - c));
                Hit {
                    begin: c + (h.begin - inner),
                    end: c + (h.end - inner),
                    ..h
                }
            }
            NodeKind::Fast(inner, k) => {
                let h = inner.query(t * k);
                Hit {
                    begin: h.begin / k,
                    end: h.end / k,
                    ..h
                }
            }
        }
    }

    /// Every atom, for up-front validation.
    pub fn atoms(&self) -> Vec<(&Atom, Range<usize>)> {
        let mut out = Vec::new();
        self.walk(&mut out);
        out
    }

    fn walk<'a>(&'a self, out: &mut Vec<(&'a Atom, Range<usize>)>) {
        match &self.kind {
            NodeKind::Atom(a) => out.push((a, self.span.clone())),
            NodeKind::Seq(s) => s.iter().for_each(|(n, _)| n.walk(out)),
            NodeKind::Alt(s) => s.iter().for_each(|n| n.walk(out)),
            NodeKind::Fast(n, _) => n.walk(out),
        }
    }

    /// True if the pattern never changes (a single value).
    pub fn is_constant(&self) -> bool {
        matches!(self.kind, NodeKind::Atom(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num_at(p: &str, t: f64) -> f64 {
        match parse(p, 0).unwrap().query(t).atom {
            Atom::Num(n) => *n,
            Atom::Rest => f64::NAN,
            a => panic!("{a:?}"),
        }
    }

    #[test]
    fn sequences_split_the_cycle() {
        assert_eq!(num_at("0 1", 0.25), 0.0);
        assert_eq!(num_at("0 1", 0.75), 1.0);
        assert_eq!(num_at("0 1", 3.75), 1.0);
        assert_eq!(num_at("0 [1 2]", 0.8), 2.0);
    }

    #[test]
    fn alternation_is_per_cycle() {
        assert_eq!(num_at("<0 1 2>", 0.5), 0.0);
        assert_eq!(num_at("<0 1 2>", 1.5), 1.0);
        assert_eq!(num_at("<0 1 2>", 5.5), 2.0);
        assert_eq!(num_at("<0 1> 9", 1.2), 1.0);
    }

    #[test]
    fn modifiers() {
        assert_eq!(num_at("[0 1]*2", 0.3), 1.0);
        assert_eq!(num_at("1@3 0", 0.7), 1.0);
        assert_eq!(num_at("1@3 0", 0.8), 0.0);
        assert_eq!(num_at("1!3 0", 0.6), 1.0);
        assert_eq!(num_at("1 ! 0", 0.4), 1.0);
        assert_eq!(num_at("0 1/2", 0.7), 1.0);
        assert!(num_at("1 ~", 0.7).is_nan());
    }

    #[test]
    fn hits_carry_spans_and_times() {
        let n = parse("0 <1 2>", 10).unwrap();
        let h = n.query(1.6);
        assert_eq!(h.atom, &Atom::Num(2.0));
        assert_eq!(h.span, 15..16);
        assert!((h.begin - 1.5).abs() < 1e-9 && (h.end - 2.0).abs() < 1e-9);
    }

    #[test]
    fn colours_and_words() {
        assert_eq!(
            parse("#fff", 0).unwrap().query(0.0).atom,
            &Atom::Color([1.0; 3])
        );
        assert_eq!(
            parse("<dawn ember>", 0).unwrap().query(1.0).atom,
            &Atom::Word("ember".into())
        );
        assert!(parse("#ff00", 0).is_err());
        assert!(parse("0, 1", 0).unwrap_err().msg.contains("chords"));
        assert!(parse("[0 1", 0).unwrap_err().msg.contains("missing `]`"));
    }
}
