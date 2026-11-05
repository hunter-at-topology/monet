//! Values that change over time, evaluated on the CPU once per frame and
//! uploaded as uniforms. Changing any of them never recompiles a shader.

use std::ops::Range;
use std::rc::Rc;

use super::mini::{Atom, Node};

/// The clock and inputs a signal is evaluated against.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    /// Cycles since start (the musical clock: `speed` cycles per second).
    pub cycle: f64,
    pub mouse: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Cosine,
    Saw,
    Isaw,
    Tri,
    Square,
    Rand,
    Perlin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glide {
    /// Short eased crossfade at each step change (default).
    Soft,
    /// Interpolate across the whole step towards the next value.
    Smooth,
    /// Jump.
    Sharp,
}

#[derive(Clone, Debug)]
pub enum Sig {
    Const(f64),
    Pat(Rc<Node>, Glide),
    Wave(Wave),
    Mouse(usize),
    /// Seconds-ish time (cycles × 2, so it is seconds at the default speed).
    Time,
    Bin(char, Box<Sig>, Box<Sig>),
    Neg(Box<Sig>),
    /// Time warp: query the inner signal at `cycle * k + shift`.
    Warp(Box<Sig>, f64, f64),
    /// Sample-and-hold `n` times per cycle.
    Segment(Box<Sig>, f64),
    Range(Box<Sig>, Box<Sig>, Box<Sig>),
}

fn hash(n: f64) -> f64 {
    let x = (n * 127.1 + 311.7).sin() * 43_758.545_312_3;
    x - x.floor()
}

fn ease(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn atom_num(a: &Atom) -> f64 {
    match a {
        Atom::Num(n) => *n,
        _ => 0.0,
    }
}

impl Sig {
    pub fn constant(&self) -> Option<f64> {
        match self {
            Sig::Const(v) => Some(*v),
            _ => None,
        }
    }

    /// Evaluate at `clock`, recording the spans of pattern steps that are
    /// playing into `active` (for highlighting).
    pub fn eval(&self, k: &Clock, active: &mut Vec<Range<usize>>) -> f64 {
        let t = k.cycle;
        match self {
            Sig::Const(v) => *v,
            Sig::Time => t * 2.0,
            Sig::Mouse(i) => k.mouse[*i] as f64,
            Sig::Wave(w) => match w {
                Wave::Sine => 0.5 + 0.5 * (t * std::f64::consts::TAU).sin(),
                Wave::Cosine => 0.5 + 0.5 * (t * std::f64::consts::TAU).cos(),
                Wave::Saw => t - t.floor(),
                Wave::Isaw => 1.0 - (t - t.floor()),
                Wave::Tri => 1.0 - (2.0 * (t - t.floor()) - 1.0).abs(),
                Wave::Square => {
                    if t - t.floor() < 0.5 {
                        0.0
                    } else {
                        1.0
                    }
                }
                Wave::Rand => {
                    // a new random value each cycle, eased in
                    let c = t.floor();
                    let f = ((t - c) / 0.25).min(1.0);
                    hash(c - 1.0) + (hash(c) - hash(c - 1.0)) * ease(f)
                }
                Wave::Perlin => {
                    let c = t.floor();
                    let f = t - c;
                    hash(c) + (hash(c + 1.0) - hash(c)) * ease(f)
                }
            },
            Sig::Pat(node, glide) => {
                let h = node.query(t);
                active.push(h.span.clone());
                let cur = atom_num(h.atom);
                match glide {
                    Glide::Sharp => cur,
                    Glide::Soft => {
                        let len = h.end - h.begin;
                        let window = (len * 0.35).min(0.12);
                        let since = t - h.begin;
                        if since >= window || window <= 0.0 || h.begin <= 0.0 {
                            return cur;
                        }
                        let prev = atom_num(node.query(h.begin - 1e-7).atom);
                        prev + (cur - prev) * ease(since / window)
                    }
                    Glide::Smooth => {
                        let next = atom_num(node.query(h.end + 1e-7).atom);
                        let f = (t - h.begin) / (h.end - h.begin).max(1e-9);
                        cur + (next - cur) * ease(f)
                    }
                }
            }
            Sig::Bin(op, a, b) => {
                let (a, b) = (a.eval(k, active), b.eval(k, active));
                match op {
                    '+' => a + b,
                    '-' => a - b,
                    '*' => a * b,
                    '/' => {
                        if b == 0.0 {
                            0.0
                        } else {
                            a / b
                        }
                    }
                    _ => a,
                }
            }
            Sig::Neg(a) => -a.eval(k, active),
            Sig::Warp(inner, mul, shift) => {
                let k2 = Clock {
                    cycle: t * mul + shift,
                    ..*k
                };
                inner.eval(&k2, active)
            }
            Sig::Segment(inner, n) => {
                let k2 = Clock {
                    cycle: (t * n).floor() / n,
                    ..*k
                };
                inner.eval(&k2, active)
            }
            Sig::Range(inner, lo, hi) => {
                let v = inner.eval(k, active);
                let (lo, hi) = (lo.eval(k, active), hi.eval(k, active));
                lo + (hi - lo) * v
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::mini;

    fn at(s: &Sig, t: f64) -> f64 {
        s.eval(
            &Clock {
                cycle: t,
                mouse: [0.25, 0.75],
            },
            &mut Vec::new(),
        )
    }

    #[test]
    fn waves_are_unit_range() {
        let s = Sig::Wave(Wave::Sine);
        assert!((at(&s, 0.25) - 1.0).abs() < 1e-9);
        assert!((at(&Sig::Wave(Wave::Saw), 2.5) - 0.5).abs() < 1e-9);
        for w in [Wave::Rand, Wave::Perlin, Wave::Tri] {
            for i in 0..50 {
                let v = at(&Sig::Wave(w), i as f64 * 0.137);
                assert!((0.0..=1.0).contains(&v), "{w:?} {v}");
            }
        }
    }

    #[test]
    fn range_slow_and_maths() {
        let s = Sig::Range(
            Box::new(Sig::Wave(Wave::Saw)),
            Box::new(Sig::Const(2.0)),
            Box::new(Sig::Const(4.0)),
        );
        assert!((at(&s, 0.5) - 3.0).abs() < 1e-9);
        let slow = Sig::Warp(Box::new(Sig::Wave(Wave::Saw)), 0.5, 0.0);
        assert!((at(&slow, 1.0) - 0.5).abs() < 1e-9);
        let m = Sig::Bin('*', Box::new(Sig::Mouse(1)), Box::new(Sig::Const(2.0)));
        assert!((at(&m, 0.0) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn patterns_glide_and_highlight() {
        let node = Rc::new(mini::parse("0 1", 100).unwrap());
        let soft = Sig::Pat(node.clone(), Glide::Soft);
        assert_eq!(at(&soft, 0.4), 0.0);
        let mid = at(&soft, 0.53);
        assert!(mid > 0.0 && mid < 1.0, "{mid}");
        assert_eq!(at(&soft, 0.9), 1.0);
        let sharp = Sig::Pat(node.clone(), Glide::Sharp);
        assert_eq!(at(&sharp, 0.51), 1.0);
        let mut active = Vec::new();
        soft.eval(
            &Clock {
                cycle: 0.9,
                mouse: [0.0; 2],
            },
            &mut active,
        );
        assert_eq!(active, vec![102..103]);
    }
}
