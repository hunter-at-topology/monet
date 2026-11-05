//! The monet language — chains for pictures.
//!
//! ```text
//! mesh("dawn")                 // a picture
//!   .flow(.3)                  // effects chain with `.`
//!   .dither("riviera", 4)
//! glints().blend("add")        // each line is a layer, bottom first
//! ```
//!
//! Numbers can be patterns ("0 .5 1", "<1 2>") or signals (sine.range(0, 2)),
//! and are evaluated on the CPU every frame — the shader only changes when the
//! *structure* (which pictures and effects) does.

pub mod build;
pub mod catalog;
pub mod codegen;
pub mod lex;
pub mod mini;
pub mod native;
pub mod parse;
pub mod signal;

use std::ops::Range;

pub use build::{PalSig, Slider};
pub use codegen::{Graph, Pass, TexRef};
pub use signal::{Clock, Sig};

#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    /// Byte range in the sketch.
    pub span: Range<usize>,
    pub msg: String,
}

impl Error {
    pub fn new(span: Range<usize>, msg: impl Into<String>) -> Self {
        Self {
            span,
            msg: msg.into(),
        }
    }

    /// 1-based line and column of the start of the span.
    pub fn line_col(&self, src: &str) -> (usize, usize) {
        let at = self.span.start.min(src.len());
        let before = &src[..at];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        (line, col)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

/// A compiled sketch: a render graph plus the signals that feed it.
#[derive(Debug)]
pub struct Program {
    pub graph: Graph,
    pub scalars: Vec<Sig>,
    pub palettes: Vec<PalSig>,
    pub sliders: Vec<Slider>,
    /// Cycles per second.
    pub speed: f64,
    /// Where pannable (fractal) sources start; `None` if there are none.
    pub home: Option<[f32; 3]>,
    pub layers: usize,
}

/// Uniform values for one frame.
pub struct Frame {
    pub scalars: [f32; build::MAX_SCALARS],
    /// OKLab stops, 8 per palette; `.w` holds the stop count.
    pub colors: [[f32; 4]; build::MAX_PALETTES * build::PAL_STOPS],
    /// Byte spans of pattern steps playing right now.
    pub active: Vec<Range<usize>>,
}

pub fn compile(src: &str) -> Result<Program, Error> {
    let stmts = parse::parse(src)?;
    let b = build::build(src, &stmts)?;
    native::check(&b.defs, &b.helpers, src)?;
    let graph = codegen::generate(&b.layers, &b.images, &b.helpers)?;
    let home = b
        .view
        .or_else(|| b.layers.iter().find_map(|l| l.source.spec.home));
    Ok(Program {
        graph,
        scalars: b.scalars,
        palettes: b.palettes,
        sliders: b.sliders,
        speed: b.speed,
        home,
        layers: b.layers.len(),
    })
}

fn ease(x: f64) -> f32 {
    let x = x.clamp(0.0, 1.0);
    (x * x * (3.0 - 2.0 * x)) as f32
}

fn resample(p: &[build::Lab], n: usize) -> Vec<build::Lab> {
    (0..n)
        .map(|i| {
            if p.len() == 1 {
                return p[0];
            }
            let x = i as f32 / (n - 1) as f32 * (p.len() - 1) as f32;
            let k = (x.floor() as usize).min(p.len() - 2);
            let f = x - k as f32;
            std::array::from_fn(|c| p[k][c] + (p[k + 1][c] - p[k][c]) * f)
        })
        .collect()
}

impl Program {
    pub fn uses_view(&self) -> bool {
        self.home.is_some()
    }

    pub fn frame(&self, clock: &Clock) -> Frame {
        let mut f = Frame {
            scalars: [0.0; build::MAX_SCALARS],
            colors: [[0.0; 4]; build::MAX_PALETTES * build::PAL_STOPS],
            active: Vec::new(),
        };
        for (i, s) in self.scalars.iter().enumerate() {
            f.scalars[i] = s.eval(clock, &mut f.active) as f32;
        }
        for (i, p) in self.palettes.iter().enumerate() {
            let stops: Vec<build::Lab> = match p {
                PalSig::Fixed(v) => v.clone(),
                PalSig::Pat(node, table) => {
                    let lookup = |a: &mini::Atom| -> Vec<build::Lab> {
                        match a {
                            mini::Atom::Word(w) => table.get(w).cloned().unwrap_or_default(),
                            mini::Atom::Color(c) => vec![build::srgb_to_oklab(*c)],
                            _ => vec![],
                        }
                    };
                    let h = node.query(clock.cycle);
                    f.active.push(h.span.clone());
                    let cur = lookup(h.atom);
                    let window = ((h.end - h.begin) * 0.5).min(0.25);
                    let since = clock.cycle - h.begin;
                    let prev = lookup(node.query(h.begin - 1e-7).atom);
                    if since < window
                        && h.begin > 0.0
                        && !prev.is_empty()
                        && prev != cur
                        && !cur.is_empty()
                    {
                        let n = build::PAL_STOPS;
                        let (a, b) = (resample(&prev, n), resample(&cur, n));
                        let t = ease(since / window);
                        a.iter()
                            .zip(&b)
                            .map(|(x, y)| std::array::from_fn(|c| x[c] + (y[c] - x[c]) * t))
                            .collect()
                    } else {
                        cur
                    }
                }
            };
            let n = stops.len().clamp(1, build::PAL_STOPS);
            for k in 0..build::PAL_STOPS {
                let c = stops
                    .get(k.min(stops.len().saturating_sub(1)))
                    .copied()
                    .unwrap_or([0.5, 0.0, 0.0]);
                f.colors[i * build::PAL_STOPS + k] = [c[0], c[1], c[2], n as f32];
            }
        }
        f
    }
}

/// Closest name by edit distance, if it's close enough to be a typo.
pub fn suggest<'a>(word: &str, names: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    /// Optimal string alignment distance (a swap of neighbours costs 1).
    fn dist(a: &str, b: &str) -> usize {
        let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
        let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
        for (i, row) in d.iter_mut().enumerate() {
            row[0] = i;
        }
        for (j, cell) in d[0].iter_mut().enumerate() {
            *cell = j;
        }
        for i in 1..=a.len() {
            for j in 1..=b.len() {
                let cost = usize::from(a[i - 1] != b[j - 1]);
                d[i][j] = (d[i - 1][j] + 1)
                    .min(d[i][j - 1] + 1)
                    .min(d[i - 1][j - 1] + cost);
                if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                    d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
                }
            }
        }
        d[a.len()][b.len()]
    }
    names
        .filter(|n| *n != word) // never "did you mean" the very word
        .map(|n| (dist(word, n), n))
        .filter(|(d, n)| *d <= (word.len().max(n.len()) / 3).max(1))
        .min_by_key(|(d, _)| *d)
        .map(|(_, n)| n)
}

#[cfg(test)]
mod tests;
