//! Evaluate a parsed sketch into layers. Every number becomes a uniform slot
//! (driven by a CPU signal), every palette a palette slot, so editing values,
//! scrubbing and patterns never touch the shader — only structure does.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use super::catalog::{self, Arg, Role, Spec};
use super::mini::{self, Atom};
use super::parse::{Def, DefKind, Expr, Kind, Stmt};
use super::signal::{Glide, Sig, Wave};
use super::{Error, suggest};

pub const MAX_SCALARS: usize = 256;
pub const MAX_PALETTES: usize = 16;
pub const PAL_STOPS: usize = 8;

/// An OKLab colour.
pub type Lab = [f32; 3];

#[derive(Clone, Debug)]
pub enum PalSig {
    Fixed(Vec<Lab>),
    Pat(Rc<mini::Node>, Rc<HashMap<String, Vec<Lab>>>),
}

#[derive(Clone, Debug)]
pub enum ArgIR {
    Scalar(usize),
    Pal(usize),
    Word(usize),
    Image(usize),
    Pic(Box<Layer>),
}

#[derive(Clone, Debug)]
pub struct Inst {
    pub spec: &'static Spec,
    pub args: Vec<ArgIR>,
    pub span: Range<usize>,
    /// A shader written in the sketch; `spec` is then its stand-in.
    pub user: Option<Rc<UserFn>>,
}

/// What a shader parameter takes, decided by its default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UArg {
    /// `amount = .5` (or no default): an `f32`, fed by numbers, patterns and signals
    Num,
    /// `colors = "dawn"`: a palette, read with `pal(colors, t)` (a `u32` in WGSL)
    Pal,
    /// `other = clouds()`: a picture, called like a function: `other(uv)`
    Pic,
}

#[derive(Clone, Debug)]
pub struct UParam {
    pub name: String,
    pub kind: UArg,
    pub default: Option<Expr>,
}

/// A `picture` or `effect` written in native WGSL inside the sketch.
#[derive(Debug)]
pub struct UserFn {
    pub kind: DefKind,
    pub name: String,
    pub name_span: Range<usize>,
    pub params: Vec<UParam>,
    pub body: String,
    pub body_start: usize,
}

/// A `wgsl { … }` block of helper functions.
#[derive(Clone, Debug)]
pub struct Helper {
    pub code: String,
    pub start: usize,
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub source: Inst,
    pub stages: Vec<Inst>,
    pub blend: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slider {
    /// Byte span of the value literal (rewritten when the slider moves).
    pub span: Range<usize>,
    pub value: f64,
    pub lo: f64,
    pub hi: f64,
    pub label: String,
}

#[derive(Clone, Debug)]
enum Val {
    Num(f64, Option<usize>),
    Sig(Sig),
    Str(String, usize),
    Layer(Box<Layer>),
    Layers(Vec<Layer>),
    Unit,
}

pub struct Built {
    pub layers: Vec<Layer>,
    pub scalars: Vec<Sig>,
    pub palettes: Vec<PalSig>,
    pub sliders: Vec<Slider>,
    pub images: Vec<String>,
    pub speed: f64,
    pub view: Option<[f32; 3]>,
    /// Shaders written in the sketch, in order, used or not.
    pub defs: Vec<Rc<UserFn>>,
    pub helpers: Vec<Helper>,
}

pub fn srgb_to_oklab(c: [f32; 3]) -> Lab {
    let lin = |x: f32| {
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b] = c.map(lin);
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

fn hex_lab(h: &str) -> Lab {
    srgb_to_oklab(mini::parse_hex(h.trim_start_matches('#')).unwrap_or([0.0; 3]))
}

pub fn named_palette(name: &str) -> Option<Vec<Lab>> {
    catalog::palette(name).map(|cols| cols.iter().map(|c| hex_lab(c)).collect())
}

struct B<'a> {
    env: HashMap<String, Val>,
    defs: HashMap<String, Rc<UserFn>>,
    user_pals: HashMap<String, Vec<Lab>>,
    out: Built,
    _src: &'a str,
}

fn type_name(v: &Val) -> &'static str {
    match v {
        Val::Num(..) => "a number",
        Val::Sig(_) => "a signal",
        Val::Str(..) => "a pattern string",
        Val::Layer(_) => "a picture",
        Val::Layers(_) => "a stack of pictures",
        Val::Unit => "a setting",
    }
}

fn wave(name: &str) -> Option<Wave> {
    Some(match name {
        "sine" => Wave::Sine,
        "cosine" => Wave::Cosine,
        "saw" => Wave::Saw,
        "isaw" => Wave::Isaw,
        "tri" => Wave::Tri,
        "square" => Wave::Square,
        "rand" => Wave::Rand,
        "perlin" => Wave::Perlin,
        _ => return None,
    })
}

/// Is this string a plain list of hex colours ("#a #b #c")?
fn hex_list(s: &str) -> Option<Vec<Lab>> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() || !parts.iter().all(|p| p.starts_with('#')) {
        return None;
    }
    parts
        .iter()
        .map(|p| mini::parse_hex(&p[1..]).map(srgb_to_oklab))
        .collect()
}

impl B<'_> {
    fn scalar(&mut self, s: Sig, span: &Range<usize>) -> Result<ArgIR, Error> {
        if self.out.scalars.len() >= MAX_SCALARS {
            return Err(Error::new(
                span.clone(),
                format!("too many values (max {MAX_SCALARS})"),
            ));
        }
        self.out.scalars.push(s);
        Ok(ArgIR::Scalar(self.out.scalars.len() - 1))
    }

    fn palette_slot(&mut self, p: PalSig, span: &Range<usize>) -> Result<ArgIR, Error> {
        if self.out.palettes.len() >= MAX_PALETTES {
            return Err(Error::new(
                span.clone(),
                format!("too many palettes in one sketch (max {MAX_PALETTES})"),
            ));
        }
        self.out.palettes.push(p);
        Ok(ArgIR::Pal(self.out.palettes.len() - 1))
    }

    fn resolve_pal_word(&self, w: &str, span: Range<usize>) -> Result<Vec<Lab>, Error> {
        if let Some(p) = self.user_pals.get(w) {
            return Ok(p.clone());
        }
        if let Some(p) = named_palette(w) {
            return Ok(p);
        }
        let mut names: Vec<String> = catalog::PALETTES.iter().map(|p| p.0.to_string()).collect();
        names.extend(self.user_pals.keys().cloned());
        let hint = match suggest(w, names.iter().map(String::as_str)) {
            Some(s) => format!(". Did you mean \"{s}\"?"),
            None => ". Try dawn, riviera, lagoon, ember, monet… or \"#hex #hex\"".into(),
        };
        Err(Error::new(span, format!("no palette called `{w}`{hint}")))
    }

    fn pal_sig(&self, text: &str, base: usize, span: &Range<usize>) -> Result<PalSig, Error> {
        if let Some(cols) = hex_list(text) {
            if cols.len() > PAL_STOPS {
                return Err(Error::new(
                    span.clone(),
                    format!("a palette holds up to {PAL_STOPS} colours"),
                ));
            }
            return Ok(PalSig::Fixed(cols));
        }
        let node = mini::parse(text, base)?;
        let mut table = HashMap::new();
        for (atom, sp) in node.atoms() {
            match atom {
                Atom::Word(w) => {
                    let p = self.resolve_pal_word(w, sp)?;
                    table.insert(w.clone(), p);
                }
                Atom::Color(_) => {}
                Atom::Num(_) | Atom::Rest => {
                    return Err(Error::new(sp, "expected a palette name or #colour here"));
                }
            }
        }
        if node.is_constant() {
            if let NodeAtom(Atom::Word(w)) = node_atom(&node) {
                return Ok(PalSig::Fixed(table[w].clone()));
            }
            if let NodeAtom(Atom::Color(c)) = node_atom(&node) {
                return Ok(PalSig::Fixed(vec![srgb_to_oklab(*c)]));
            }
        }
        Ok(PalSig::Pat(Rc::new(node), Rc::new(table)))
    }

    fn as_sig(&mut self, v: Val, span: &Range<usize>, what: &str) -> Result<Sig, Error> {
        match v {
            Val::Num(n, _) => Ok(Sig::Const(n)),
            Val::Sig(s) => Ok(s),
            Val::Str(text, base) => {
                let node = mini::parse(&text, base)?;
                for (a, sp) in node.atoms() {
                    if !matches!(a, Atom::Num(_) | Atom::Rest) {
                        return Err(Error::new(
                            sp,
                            format!("{what} needs numbers here, like \"0 .5 1\""),
                        ));
                    }
                }
                if let (true, NodeAtom(Atom::Num(n))) = (node.is_constant(), node_atom(&node)) {
                    return Ok(Sig::Const(*n));
                }
                Ok(Sig::Pat(Rc::new(node), Glide::Soft))
            }
            other => Err(Error::new(
                span.clone(),
                format!("{what} needs a number, but this is {}", type_name(&other)),
            )),
        }
    }

    fn eval(&mut self, e: &Expr) -> Result<Val, Error> {
        match &e.kind {
            Kind::Num(n) => Ok(Val::Num(*n, None)),
            Kind::Str(s, base) => Ok(Val::Str(s.clone(), *base)),
            Kind::Ident(name) => self.ident(name, &e.span),
            Kind::Neg(a) => {
                let v = self.eval(a)?;
                Ok(match v {
                    Val::Num(n, _) => Val::Num(-n, None),
                    v => Val::Sig(Sig::Neg(Box::new(self.as_sig(v, &a.span, "`-`")?))),
                })
            }
            Kind::Bin(op, a, b) => {
                let (va, vb) = (self.eval(a)?, self.eval(b)?);
                if let (Val::Num(x, _), Val::Num(y, _)) = (&va, &vb) {
                    let r = match op {
                        '+' => x + y,
                        '-' => x - y,
                        '*' => x * y,
                        _ if *y == 0.0 => 0.0,
                        _ => x / y,
                    };
                    return Ok(Val::Num(r, None));
                }
                let what = format!("`{op}`");
                let sa = self.as_sig(va, &a.span, &what)?;
                let sb = self.as_sig(vb, &b.span, &what)?;
                Ok(Val::Sig(Sig::Bin(*op, Box::new(sa), Box::new(sb))))
            }
            Kind::Call {
                name,
                name_span,
                args,
            } => self.call(name, name_span, args, &e.span),
            Kind::Method {
                recv,
                name,
                name_span,
                args,
                ..
            } => self.method(recv, name, name_span, args, &e.span),
        }
    }

    fn ident(&mut self, name: &str, span: &Range<usize>) -> Result<Val, Error> {
        if let Some(v) = self.env.get(name) {
            return Ok(v.clone());
        }
        if let Some(w) = wave(name) {
            return Ok(Val::Sig(Sig::Wave(w)));
        }
        match name {
            "time" => return Ok(Val::Sig(Sig::Time)),
            "mouse" => return Err(Error::new(span.clone(), "use `mouse.x` or `mouse.y`")),
            _ => {}
        }
        if let Some(f) = self.defs.get(name).cloned() {
            return self.user_layer(&f, &[], span, span);
        }
        if let Some(spec) = catalog::find(name).filter(|s| s.role == Role::Source && !s.hidden) {
            return Ok(Val::Layer(Box::new(Layer {
                source: self.inst(spec, &[], span)?,
                stages: vec![],
                blend: 0,
            })));
        }
        Err(self.unknown(name, span))
    }

    fn unknown(&self, name: &str, span: &Range<usize>) -> Error {
        let mut names: Vec<String> = catalog::all_names().map(String::from).collect();
        names.extend(self.env.keys().cloned());
        names.extend(self.defs.keys().cloned());
        let hint = suggest(name, names.iter().map(String::as_str))
            .map(|s| format!(". Did you mean `{s}`?"))
            .unwrap_or_default();
        Error::new(span.clone(), format!("unknown name `{name}`{hint}"))
    }

    fn const_arg(
        &mut self,
        args: &[Expr],
        i: usize,
        default: f64,
        what: &str,
    ) -> Result<f64, Error> {
        let Some(a) = args.get(i) else {
            return Ok(default);
        };
        match self.eval(a)? {
            Val::Num(n, _) => Ok(n),
            v => Err(Error::new(
                a.span.clone(),
                format!("{what} needs a plain number, not {}", type_name(&v)),
            )),
        }
    }

    fn call(
        &mut self,
        name: &str,
        name_span: &Range<usize>,
        args: &[Expr],
        span: &Range<usize>,
    ) -> Result<Val, Error> {
        match name {
            "slider" => {
                let Some(first) = args.first() else {
                    return Err(Error::new(
                        span.clone(),
                        "slider(value, lo, hi) needs a value",
                    ));
                };
                let Kind::Num(value) = first.kind else {
                    return Err(Error::new(
                        first.span.clone(),
                        "slider's first value must be a plain number, because the slider rewrites it",
                    ));
                };
                let lo = self.const_arg(args, 1, 0.0, "slider")?;
                let hi = self.const_arg(
                    args,
                    2,
                    if value > 1.0 { value * 2.0 } else { 1.0 },
                    "slider",
                )?;
                self.out.sliders.push(Slider {
                    span: first.span.clone(),
                    value,
                    lo,
                    hi,
                    label: String::new(),
                });
                return Ok(Val::Num(value, Some(self.out.sliders.len() - 1)));
            }
            "speed" | "setcps" => {
                let v = self.const_arg(args, 0, 0.5, "speed")?;
                if v <= 0.0 {
                    return Err(Error::new(span.clone(), "speed must be above 0"));
                }
                self.out.speed = v;
                return Ok(Val::Unit);
            }
            "view" => {
                let x = self.const_arg(args, 0, 0.0, "view")?;
                let y = self.const_arg(args, 1, 0.0, "view")?;
                let z = self.const_arg(args, 2, 1.0, "view")?;
                self.out.view = Some([x as f32, y as f32, z.max(1e-6) as f32]);
                return Ok(Val::Unit);
            }
            "stack" => {
                let mut layers = Vec::new();
                for a in args {
                    match self.eval(a)? {
                        Val::Layer(l) => layers.push(*l),
                        Val::Layers(ls) => layers.extend(ls),
                        v => {
                            return Err(Error::new(
                                a.span.clone(),
                                format!("stack needs pictures, but this is {}", type_name(&v)),
                            ));
                        }
                    }
                }
                return Ok(Val::Layers(layers));
            }
            _ => {}
        }
        if let Some(f) = self.defs.get(name).cloned() {
            return self.user_layer(&f, args, name_span, span);
        }
        if let Some(spec) = catalog::find(name).filter(|s| !s.hidden) {
            if spec.role != Role::Source {
                return Err(Error::new(
                    name_span.clone(),
                    if name == "noise" {
                        // the flowing-noise picture was called noise() before .noise() existed
                        "`noise` is an effect now: add grain with mesh().noise(). The flowing-noise picture is clouds()".to_string()
                    } else {
                        format!("`{name}` is an effect. Chain it onto a picture: mesh().{name}()")
                    },
                ));
            }
            let source = self.inst(spec, args, span)?;
            return Ok(Val::Layer(Box::new(Layer {
                source,
                stages: vec![],
                blend: 0,
            })));
        }
        if wave(name).is_some() {
            return Err(Error::new(
                name_span.clone(),
                format!("`{name}` is a signal. Use it without brackets"),
            ));
        }
        Err(self.unknown(name, name_span))
    }

    fn method(
        &mut self,
        recv: &Expr,
        name: &str,
        name_span: &Range<usize>,
        args: &[Expr],
        span: &Range<usize>,
    ) -> Result<Val, Error> {
        if let Kind::Ident(r) = &recv.kind
            && r == "mouse"
            && !self.env.contains_key("mouse")
        {
            return match name {
                "x" => Ok(Val::Sig(Sig::Mouse(0))),
                "y" => Ok(Val::Sig(Sig::Mouse(1))),
                _ => Err(Error::new(name_span.clone(), "mouse has `.x` and `.y`")),
            };
        }
        let rv = self.eval(recv)?;
        match rv {
            Val::Layer(mut layer) => {
                if name == "blend" {
                    let Some(a) = args.first() else {
                        return Err(Error::new(
                            span.clone(),
                            "blend needs a mode, like blend(\"screen\")",
                        ));
                    };
                    layer.blend = self.word(a, catalog::BLENDS)?;
                    return Ok(Val::Layer(layer));
                }
                if let Some(f) = self.defs.get(name).cloned() {
                    if f.kind == DefKind::Picture {
                        return Err(Error::new(
                            name_span.clone(),
                            format!("`{name}` is a picture, not an effect. Put it on its own line to layer it"),
                        ));
                    }
                    let stage = self.user_inst(&f, args, &(name_span.start..span.end))?;
                    layer.stages.push(stage);
                    return Ok(Val::Layer(layer));
                }
                let Some(spec) = catalog::effect(name) else {
                    if catalog::SIGNAL_METHODS.iter().any(|m| m.0 == name) {
                        // `.div(2)` on a picture: a real method, for numbers and signals
                        let instead = match name {
                            "add" | "sub" | "mul" | "div" => " To lighten or darken a picture, try .brightness() or .contrast()",
                            "slow" | "fast" | "early" | "late" => " To change a picture's pace, give its speed a pattern, or use speed()",
                            _ => "",
                        };
                        return Err(Error::new(name_span.clone(), format!("`.{name}` works on numbers and signals, not pictures.{instead}")));
                    }
                    return Err(self.unknown(name, name_span));
                };
                if spec.role == Role::Source {
                    return Err(Error::new(
                        name_span.clone(),
                        format!(
                            "`{name}` is a picture, not an effect. Put it on its own line to layer it"
                        ),
                    ));
                }
                let mut spec = spec;
                if name == "dither"
                    && let Some(a) = args.first()
                    && let Kind::Str(s, _) = &a.kind
                    && s.chars().any(|c| c.is_ascii_alphabetic() || c == '#')
                {
                    spec = catalog::find("dither_palette").expect("dither_palette");
                }
                if name == "mask"
                    && let Some(a) = args.first()
                    && !matches!(a.kind, Kind::Str(..))
                {
                    spec = catalog::find("mask_picture").expect("mask_picture");
                }
                // still noise (motion left at 0) compiles without a clock
                let mut args = args.to_vec();
                if spec.name == "noise"
                    && args.len() <= spec.args.len()
                    && args
                        .get(3)
                        .is_none_or(|a| matches!(a.kind, Kind::Num(v) if v <= 0.0))
                {
                    spec = catalog::find("noise_still").expect("noise_still");
                    if args.len() > 3 {
                        args.remove(3);
                    }
                }
                let stage = self.inst(spec, &args, &(name_span.start..span.end))?;
                layer.stages.push(stage);
                Ok(Val::Layer(layer))
            }
            Val::Layers(_) => Err(Error::new(
                name_span.clone(),
                format!("`.{name}` can't apply to a whole stack yet. Add it to each layer inside"),
            )),
            Val::Unit => Err(Error::new(name_span.clone(), "settings can't be chained")),
            v => self.signal_method(v, recv, name, name_span, args),
        }
    }

    fn signal_method(
        &mut self,
        v: Val,
        recv: &Expr,
        name: &str,
        name_span: &Range<usize>,
        args: &[Expr],
    ) -> Result<Val, Error> {
        let sig = self.as_sig(v, &recv.span, &format!("`.{name}`"))?;
        let arg = |b: &mut Self, i: usize, d: f64| -> Result<Sig, Error> {
            match args.get(i) {
                Some(a) => {
                    let v = b.eval(a)?;
                    b.as_sig(v, &a.span, &format!("`{name}`"))
                }
                None => Ok(Sig::Const(d)),
            }
        };
        let num = |b: &mut Self, i: usize, d: f64| -> Result<f64, Error> {
            let s = arg(b, i, d)?;
            s.constant().ok_or_else(|| {
                Error::new(
                    args[i].span.clone(),
                    format!("`{name}` needs a plain number here"),
                )
            })
        };
        let s = match name {
            "range" => Sig::Range(
                Box::new(sig),
                Box::new(arg(self, 0, 0.0)?),
                Box::new(arg(self, 1, 1.0)?),
            ),
            "slow" => Sig::Warp(Box::new(sig), 1.0 / num(self, 0, 2.0)?.max(1e-6), 0.0),
            "fast" => Sig::Warp(Box::new(sig), num(self, 0, 2.0)?, 0.0),
            "early" => Sig::Warp(Box::new(sig), 1.0, num(self, 0, 0.25)?),
            "late" => Sig::Warp(Box::new(sig), 1.0, -num(self, 0, 0.25)?),
            "segment" => Sig::Segment(Box::new(sig), num(self, 0, 4.0)?.max(1e-6)),
            "add" | "sub" | "mul" | "div" => {
                let op = match name {
                    "add" => '+',
                    "sub" => '-',
                    "mul" => '*',
                    _ => '/',
                };
                Sig::Bin(
                    op,
                    Box::new(sig),
                    Box::new(arg(
                        self,
                        0,
                        if op == '*' || op == '/' { 1.0 } else { 0.0 },
                    )?),
                )
            }
            "smooth" | "sharp" => {
                let g = if name == "smooth" {
                    Glide::Smooth
                } else {
                    Glide::Sharp
                };
                match sig {
                    Sig::Pat(n, _) => Sig::Pat(n, g),
                    other => other,
                }
            }
            _ => {
                if catalog::find(name).is_some() || self.defs.contains_key(name) {
                    return Err(Error::new(
                        name_span.clone(),
                        format!(
                            "`.{name}` is an effect for pictures, but this is a number or signal"
                        ),
                    ));
                }
                return Err(self.unknown(name, name_span));
            }
        };
        Ok(Val::Sig(s))
    }

    /// A shader written in the sketch, used as a picture.
    fn user_layer(
        &mut self,
        f: &Rc<UserFn>,
        args: &[Expr],
        name_span: &Range<usize>,
        span: &Range<usize>,
    ) -> Result<Val, Error> {
        if f.kind == DefKind::Effect {
            return Err(Error::new(
                name_span.clone(),
                format!("`{0}` is an effect. Chain it onto a picture: mesh().{0}()", f.name),
            ));
        }
        let source = self.user_inst(f, args, span)?;
        Ok(Val::Layer(Box::new(Layer {
            source,
            stages: vec![],
            blend: 0,
        })))
    }

    /// Evaluate a call of a sketch shader: one slot per parameter, as for built-ins.
    fn user_inst(
        &mut self,
        f: &Rc<UserFn>,
        args: &[Expr],
        span: &Range<usize>,
    ) -> Result<Inst, Error> {
        let sig = || {
            let names: Vec<&str> = f.params.iter().map(|p| p.name.as_str()).collect();
            format!("{}({})", f.name, names.join(", "))
        };
        if args.len() > f.params.len() {
            let what = match f.params.len() {
                0 => "takes no values".to_string(),
                1 => "takes 1 value".to_string(),
                n => format!("takes {n} values"),
            };
            return Err(Error::new(
                args[f.params.len()].span.clone(),
                format!("`{}` {what}: {}", f.name, sig()),
            ));
        }
        let mut out = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            let given = args.get(i).or(p.default.as_ref());
            let what = format!("`{}`'s {}", f.name, p.name);
            let ir = match (p.kind, given) {
                (UArg::Num, None) => self.scalar(Sig::Const(0.0), span)?,
                (UArg::Num, Some(a)) => {
                    let v = self.eval(a)?;
                    if let Val::Num(_, Some(k)) = v
                        && self.out.sliders[k].label.is_empty()
                    {
                        self.out.sliders[k].label = format!("{} {}", f.name, p.name);
                    }
                    let s = self.as_sig(v, &a.span, &what)?;
                    self.scalar(s, &a.span)?
                }
                (UArg::Pal, Some(a)) => match &a.kind {
                    Kind::Str(text, base) => {
                        let pal = self.pal_sig(text, *base, &a.span)?;
                        self.palette_slot(pal, &a.span)?
                    }
                    _ => match self.eval(a)? {
                        Val::Str(text, base) => {
                            let pal = self.pal_sig(&text, base, &a.span)?;
                            self.palette_slot(pal, &a.span)?
                        }
                        _ => {
                            return Err(Error::new(
                                a.span.clone(),
                                format!("{what} is a palette, like \"dawn\" or \"#f4a7b9 #a8c8f0\""),
                            ));
                        }
                    },
                },
                (UArg::Pic, Some(a)) => match self.eval(a)? {
                    Val::Layer(l) => ArgIR::Pic(l),
                    v => {
                        return Err(Error::new(
                            a.span.clone(),
                            format!(
                                "{what} is a picture, like clouds() or spot().blur(), not {}",
                                type_name(&v)
                            ),
                        ));
                    }
                },
                (UArg::Pal | UArg::Pic, None) => unreachable!("palettes and pictures have defaults"),
            };
            out.push(ir);
        }
        let stand_in = match f.kind {
            DefKind::Picture => "user_picture",
            DefKind::Effect => "user_effect",
        };
        Ok(Inst {
            spec: catalog::find(stand_in).expect("stand-in spec"),
            args: out,
            span: span.clone(),
            user: Some(f.clone()),
        })
    }

    fn word(&mut self, a: &Expr, options: &[&str]) -> Result<usize, Error> {
        let Kind::Str(w, _) = &a.kind else {
            return Err(Error::new(
                a.span.clone(),
                format!("expected one of \"{}\"", options.join("\" \"")),
            ));
        };
        let w = w.trim();
        options.iter().position(|o| *o == w).ok_or_else(|| {
            let hint = suggest(w, options.iter().copied())
                .map(|s| format!(". Did you mean \"{s}\"?"))
                .unwrap_or_default();
            Error::new(
                a.span.clone(),
                format!("`{w}` isn't one of {}{hint}", options.join(", ")),
            )
        })
    }

    fn inst(
        &mut self,
        spec: &'static Spec,
        args: &[Expr],
        span: &Range<usize>,
    ) -> Result<Inst, Error> {
        if args.len() > spec.args.len() {
            let sig = catalog::signature_as(catalog::written(spec), spec);
            let what = if spec.args.is_empty() {
                "takes no values".to_string()
            } else {
                format!(
                    "takes {} value{}",
                    spec.args.len(),
                    if spec.args.len() == 1 { "" } else { "s" }
                )
            };
            return Err(Error::new(
                args[spec.args.len()].span.clone(),
                format!("`{}` {what}: {sig}", catalog::written(spec)),
            ));
        }
        let mut out = Vec::new();
        for (i, (argname, kind)) in spec.args.iter().enumerate() {
            let given = args.get(i);
            let ir = match (kind, given) {
                (Arg::Num(d), None) => self.scalar(Sig::Const(*d), span)?,
                (Arg::Num(_), Some(a)) => {
                    let v = self.eval(a)?;
                    if let Val::Num(_, Some(k)) = v
                        && self.out.sliders[k].label.is_empty()
                    {
                        self.out.sliders[k].label = format!("{} {argname}", catalog::written(spec));
                    }
                    let s = self.as_sig(
                        v,
                        &a.span,
                        &format!("`{}`'s {argname}", catalog::written(spec)),
                    )?;
                    self.scalar(s, &a.span)?
                }
                (Arg::Pal(d) | Arg::Color(d), None) => {
                    let p = match catalog::palette(d) {
                        Some(_) => PalSig::Fixed(named_palette(d).unwrap()),
                        None => PalSig::Fixed(hex_list(d).unwrap_or_default()),
                    };
                    self.palette_slot(p, span)?
                }
                (Arg::Pal(_) | Arg::Color(_), Some(a)) => {
                    let v = self.eval(a)?;
                    let Val::Str(text, base) = v else {
                        return Err(Error::new(
                            a.span.clone(),
                            format!(
                                "`{}`'s {argname} is a palette, like \"dawn\" or \"#f4a7b9 #a8c8f0\"",
                                catalog::written(spec)
                            ),
                        ));
                    };
                    let p = self.pal_sig(&text, base, &a.span)?;
                    self.palette_slot(p, &a.span)?
                }
                (Arg::Word(d, opts), None) => {
                    ArgIR::Word(opts.iter().position(|o| o == d).unwrap_or(0))
                }
                (Arg::Word(_, opts), Some(a)) => ArgIR::Word(self.word(a, opts)?),
                (Arg::Pic(d), None) => {
                    let src = catalog::find(d).expect("default picture");
                    let source = self.inst(src, &[], span)?;
                    ArgIR::Pic(Box::new(Layer {
                        source,
                        stages: vec![],
                        blend: 0,
                    }))
                }
                (Arg::Pic(_), Some(a)) => match self.eval(a)? {
                    Val::Layer(l) => ArgIR::Pic(l),
                    v => {
                        return Err(Error::new(
                            a.span.clone(),
                            format!(
                                "`{}`'s {argname} is a picture, like clouds() or spot().trail(.9), not {}",
                                catalog::written(spec),
                                type_name(&v)
                            ),
                        ));
                    }
                },
                (Arg::Path, None) => {
                    return Err(Error::new(
                        span.clone(),
                        "image needs a file: image(\"photo.png\")",
                    ));
                }
                (Arg::Path, Some(a)) => {
                    let Kind::Str(p, _) = &a.kind else {
                        return Err(Error::new(
                            a.span.clone(),
                            "image needs a file name in quotes",
                        ));
                    };
                    self.out.images.push(p.trim().to_string());
                    ArgIR::Image(self.out.images.len() - 1)
                }
            };
            out.push(ir);
        }
        Ok(Inst {
            spec,
            args: out,
            span: span.clone(),
            user: None,
        })
    }
}

struct NodeAtom<'a>(&'a Atom);
fn node_atom(n: &mini::Node) -> NodeAtom<'_> {
    match &n.kind {
        mini::NodeKind::Atom(a) => NodeAtom(a),
        _ => NodeAtom(&Atom::Rest),
    }
}

/// Names the language already owns: a sketch can't define or assign them.
fn reserved(name: &str) -> bool {
    catalog::find(name).is_some_and(|s| !s.hidden)
        || wave(name).is_some()
        || matches!(
            name,
            "mouse" | "time" | "slider" | "speed" | "setcps" | "view" | "stack" | "blend"
                | "picture" | "effect" | "wgsl"
        )
}

/// Is this default a picture? (A source, a sketch picture, a chain on one, or a
/// variable holding one.)
fn is_picture(e: &Expr, pictures: &dyn Fn(&str) -> bool, lets: &HashMap<&str, &Expr>, depth: u8) -> bool {
    if depth > 8 {
        return false;
    }
    match &e.kind {
        Kind::Call { name, .. } => pictures(name),
        Kind::Ident(name) => {
            pictures(name) || lets.get(name.as_str()).is_some_and(|v| is_picture(v, pictures, lets, depth + 1))
        }
        Kind::Method { recv, .. } => is_picture(recv, pictures, lets, depth + 1),
        _ => false,
    }
}

/// Check a `picture`/`effect` definition and work out what each parameter takes.
fn define(
    d: &Def,
    pictures: &dyn Fn(&str) -> bool,
    lets: &HashMap<&str, &Expr>,
) -> Result<UserFn, Error> {
    if reserved(&d.name) {
        return Err(Error::new(
            d.name_span.clone(),
            format!("`{}` is a built-in name. Pick another", d.name),
        ));
    }
    let mut params: Vec<UParam> = Vec::new();
    for p in &d.params {
        let n = p.name.as_str();
        if matches!(n, "uv" | "pic" | "u") {
            return Err(Error::new(
                p.span.clone(),
                format!("`{n}` is already given to every shader. Name this parameter something else"),
            ));
        }
        if n.starts_with("__") || naga::keywords::wgsl::RESERVED.contains(&n) {
            return Err(Error::new(
                p.span.clone(),
                format!("`{n}` is a WGSL keyword. Name this parameter something else"),
            ));
        }
        if params.iter().any(|q| q.name == n) {
            return Err(Error::new(p.span.clone(), format!("`{n}` is already a parameter")));
        }
        let kind = match &p.default {
            None => UArg::Num,
            Some(Expr { kind: Kind::Str(text, base), .. }) => {
                // "0 .5 1" is a number pattern; anything else is a palette
                let numeric = mini::parse(text, *base).is_ok_and(|node| {
                    node.atoms().iter().all(|(a, _)| matches!(a, Atom::Num(_) | Atom::Rest))
                });
                if numeric { UArg::Num } else { UArg::Pal }
            }
            Some(e) if is_picture(e, pictures, lets, 0) => UArg::Pic,
            Some(_) => UArg::Num,
        };
        params.push(UParam {
            name: p.name.clone(),
            kind,
            default: p.default.clone(),
        });
    }
    Ok(UserFn {
        kind: d.kind,
        name: d.name.clone(),
        name_span: d.name_span.clone(),
        params,
        body: d.body.clone(),
        body_start: d.body_start,
    })
}

pub fn build(src: &str, stmts: &[Stmt]) -> Result<Built, Error> {
    let mut b = B {
        env: HashMap::new(),
        defs: HashMap::new(),
        user_pals: HashMap::new(),
        out: Built {
            layers: vec![],
            scalars: vec![],
            palettes: vec![],
            sliders: vec![],
            images: vec![],
            speed: 0.5,
            view: None,
            defs: vec![],
            helpers: vec![],
        },
        _src: src,
    };
    // Shaders first, so a sketch can use one above where it's written.
    let lets: HashMap<&str, &Expr> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::Let { name, value, .. } => Some((name.as_str(), value)),
            _ => None,
        })
        .collect();
    let picture_names: Vec<&str> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::Def(d) if d.kind == DefKind::Picture => Some(d.name.as_str()),
            _ => None,
        })
        .collect();
    let pictures = |n: &str| {
        picture_names.contains(&n) || catalog::find(n).is_some_and(|s| s.role == Role::Source && !s.hidden)
    };
    for s in stmts {
        match s {
            Stmt::Def(d) => {
                if b.defs.contains_key(&d.name) {
                    return Err(Error::new(
                        d.name_span.clone(),
                        format!("`{}` is already defined above. Pick another name", d.name),
                    ));
                }
                let f = Rc::new(define(d, &pictures, &lets)?);
                b.out.defs.push(f.clone());
                b.defs.insert(d.name.clone(), f);
            }
            Stmt::Wgsl { code, start } => b.out.helpers.push(Helper {
                code: code.clone(),
                start: *start,
            }),
            _ => {}
        }
    }
    for s in stmts {
        match s {
            Stmt::Def(_) | Stmt::Wgsl { .. } => {}
            Stmt::Let { name, value, span } => {
                if catalog::find(name).is_some()
                    || wave(name).is_some()
                    || matches!(name.as_str(), "mouse" | "time")
                {
                    return Err(Error::new(
                        span.clone(),
                        format!("`{name}` is a built-in name. Pick another"),
                    ));
                }
                if b.defs.contains_key(name) {
                    return Err(Error::new(
                        span.clone(),
                        format!("`{name}` is a shader defined in this sketch. Pick another name"),
                    ));
                }
                let v = b.eval(value)?;
                if let Val::Num(_, Some(k)) = v {
                    b.out.sliders[k].label = name.clone();
                }
                if let Val::Str(text, _) = &v
                    && let Some(p) = hex_list(text)
                {
                    b.user_pals.insert(name.clone(), p);
                }
                b.env.insert(name.clone(), v);
            }
            Stmt::Expr(e) => match b.eval(e)? {
                Val::Layer(l) => b.out.layers.push(*l),
                Val::Layers(ls) => b.out.layers.extend(ls),
                Val::Unit => {}
                v => {
                    return Err(Error::new(
                        e.span.clone(),
                        format!(
                            "this line is {}. Each line should be a picture, like mesh(\"dawn\")",
                            type_name(&v)
                        ),
                    ));
                }
            },
        }
    }
    Ok(b.out)
}
