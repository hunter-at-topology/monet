//! Layers → a small render graph of fullscreen passes.
//!
//! Domain and colour effects fuse into one shader as nested functions:
//! `dither(pixelate(warp(mesh)))(uv)`. Effects that need neighbouring pixels
//! (blur, bloom, chroma…) cut the chain: everything before renders to a
//! texture, and the rest of the chain samples it. All layers' final segments
//! share one composite pass. Pipelines are cached by shader text, so editing
//! one layer only recompiles the passes that changed.

use std::fmt::Write;

use super::Error;
use super::build::{ArgIR, Helper, Inst, Layer};
use super::catalog::{Role, Tex};

pub const LIBRARY: &str = include_str!("../library.wgsl");
pub const MAX_INPUTS: usize = 8;
pub const VS_ENTRY: &str = "vs";
pub const FS_ENTRY: &str = "fs";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TexRef {
    /// An intermediate buffer (index into `Graph::bufs`).
    Buf(usize),
    /// Last frame's value of a feedback buffer.
    HistPrev(usize),
    /// This frame's value of a feedback buffer.
    HistNext(usize),
    Image(usize),
    Screen,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    pub label: String,
    /// The fragment shader, without the library (see `Pass::source`).
    pub body: String,
    pub inputs: Vec<TexRef>,
    pub output: TexRef,
}

impl Pass {
    /// The full shader: only the library items this pass reaches, then the body.
    pub fn source(&self) -> String {
        format!("{}\n// ── generated ──\n{}", shake(&self.body), self.body)
    }
}

/// One top-level item of the library (a function, struct, binding, const…).
struct Item {
    name: String,
    text: String,
    deps: Vec<usize>,
    always: bool,
}

fn idents(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| {
            w.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        })
}

fn library_items() -> &'static [Item] {
    static ITEMS: std::sync::OnceLock<Vec<Item>> = std::sync::OnceLock::new();
    ITEMS.get_or_init(|| {
        let mut items: Vec<Item> = Vec::new();
        let mut cur = String::new();
        let mut depth = 0i32;
        for line in LIBRARY.lines() {
            let t = line.trim_start();
            if depth == 0 && cur.trim().is_empty() && (t.starts_with("//") || t.is_empty()) {
                continue; // comments between items are dropped
            }
            cur.push_str(line);
            cur.push('\n');
            let code = line.split("//").next().unwrap_or("");
            depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
            let attr_only = t.starts_with('@') && !t.contains("var") && !t.contains("fn ");
            if depth == 0
                && !attr_only
                && (code.trim_end().ends_with('}') || code.trim_end().ends_with(';'))
            {
                let name = {
                    let head = cur
                        .lines()
                        .find(|l| {
                            !l.trim_start().starts_with("//") && !l.trim_start().starts_with('@')
                                || l.contains(" var")
                        })
                        .unwrap_or("");
                    let mut words = idents(head).filter(|w| {
                        !matches!(
                            *w,
                            "group"
                                | "binding"
                                | "var"
                                | "uniform"
                                | "fn"
                                | "struct"
                                | "const"
                                | "private"
                        )
                    });
                    words.next().unwrap_or("").to_string()
                };
                let always = cur.contains("@vertex")
                    || cur.contains("@group")
                    || cur.starts_with("struct U");
                items.push(Item {
                    name,
                    text: std::mem::take(&mut cur),
                    deps: Vec::new(),
                    always,
                });
            }
        }
        let index: std::collections::HashMap<String, usize> = items
            .iter()
            .enumerate()
            .map(|(i, it)| (it.name.clone(), i))
            .collect();
        for (i, item) in items.iter_mut().enumerate() {
            let mut deps: Vec<usize> = idents(&item.text)
                .filter_map(|w| index.get(w).copied())
                .filter(|&j| j != i)
                .collect();
            deps.sort_unstable();
            deps.dedup();
            item.deps = deps;
        }
        items
    })
}

/// The library, minus everything `body` can't reach.
pub fn shake(body: &str) -> String {
    let items = library_items();
    let index: std::collections::HashMap<&str, usize> = items
        .iter()
        .enumerate()
        .map(|(i, it)| (it.name.as_str(), i))
        .collect();
    let mut keep = vec![false; items.len()];
    let mut stack: Vec<usize> = idents(body).filter_map(|w| index.get(w).copied()).collect();
    stack.extend(
        items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.always)
            .map(|(i, _)| i),
    );
    while let Some(i) = stack.pop() {
        if !std::mem::replace(&mut keep[i], true) {
            stack.extend(items[i].deps.iter().copied());
        }
    }
    items
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(it, _)| it.text.as_str())
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod shake_tests {
    use super::*;

    #[test]
    fn library_splits_into_named_items() {
        let items = library_items();
        for name in [
            "U",
            "vs",
            "frag_uv",
            "tx",
            "noise",
            "fbm",
            "src_mesh",
            "blend_layer",
            "finish",
            "PI",
        ] {
            assert!(items.iter().any(|i| i.name == name), "missing {name}");
        }
        // Every item is reproduced exactly once.
        let total: usize = items.iter().map(|i| i.text.matches("fn ").count()).sum();
        assert_eq!(
            total,
            LIBRARY
                .lines()
                .filter(|l| l.trim_start().starts_with("fn "))
                .count()
        );
    }

    #[test]
    fn shaking_keeps_what_is_reached() {
        let s = shake("fn fs() { let x = src_mesh(uv, 0u, 1.0); }");
        assert!(
            s.contains("fn src_mesh") && s.contains("fn noise(") && s.contains("fn oklab_to_srgb")
        );
        assert!(!s.contains("fn src_water") && !s.contains("fn kuwahara"));
        assert!(s.contains("struct U") && s.contains("fn vs("));
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub passes: Vec<Pass>,
    /// Resolution divisor of each intermediate buffer.
    pub bufs: Vec<u32>,
    pub hists: usize,
    pub images: Vec<String>,
}

fn placeholder(r: TexRef) -> String {
    match r {
        TexRef::Buf(i) => format!("@B{i}@"),
        TexRef::HistPrev(i) => format!("@P{i}@"),
        TexRef::HistNext(i) => format!("@N{i}@"),
        TexRef::Image(i) => format!("@I{i}@"),
        TexRef::Screen => unreachable!(),
    }
}

#[derive(Default)]
struct Seg {
    fns: String,
    inputs: Vec<TexRef>,
    last: String,
}

impl Seg {
    fn tex(&mut self, r: TexRef) -> String {
        if !self.inputs.contains(&r) {
            self.inputs.push(r);
        }
        placeholder(r)
    }
    fn push(&mut self, name: String, expr: &str) {
        let _ = writeln!(
            self.fns,
            "fn {name}(uv: vec2f) -> vec4f {{ return {expr}; }}"
        );
        self.last = name;
    }
}

fn scalar(i: usize) -> String {
    format!("u.s[{}].{}", i / 4, ["x", "y", "z", "w"][i % 4])
}

fn arg_expr(a: &ArgIR, seg: &mut Seg, pic: Option<&String>) -> String {
    match a {
        ArgIR::Pic(_) => pic.cloned().unwrap_or_default(),
        ArgIR::Scalar(i) => scalar(*i),
        ArgIR::Pal(p) => format!("{}u", p * super::build::PAL_STOPS),
        ArgIR::Word(w) => format!("{w}"),
        ArgIR::Image(i) => seg.tex(TexRef::Image(*i)),
    }
}

fn fill(
    template: &str,
    prev: &str,
    inst: &Inst,
    seg: &mut Seg,
    pics: &[(usize, String)],
) -> String {
    let mut s = template.replace("{prev}", prev);
    for (i, a) in inst.args.iter().enumerate() {
        let pic = pics.iter().find(|p| p.0 == i).map(|p| &p.1);
        let e = arg_expr(a, seg, pic);
        s = s.replace(&format!("{{{i}}}"), &e);
    }
    s
}

struct G {
    graph: Graph,
    subs: usize,
    /// The sketch's `wgsl { … }` helpers, ready to paste into passes that run sketch code.
    helpers: String,
}

impl G {
    fn buf(&mut self, div: u32) -> usize {
        self.graph.bufs.push(div);
        self.graph.bufs.len() - 1
    }

    fn assemble(
        &self,
        mut seg: Seg,
        main: &str,
        output: TexRef,
        label: String,
        at: &Inst,
    ) -> Result<Pass, Error> {
        if seg.inputs.len() > MAX_INPUTS {
            return Err(Error::new(
                at.span.clone(),
                format!(
                    "too many textures feed one pass (max {MAX_INPUTS}). Use fewer blur, bloom or image layers"
                ),
            ));
        }
        let div = match output {
            TexRef::Buf(i) => self.graph.bufs[i],
            _ => 1,
        };
        let mut body = std::mem::take(&mut seg.fns);
        if body.contains(super::native::USER_MARK) {
            body.insert_str(0, &self.helpers);
        }
        let _ = write!(
            body,
            "@fragment\nfn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {{\n    let uv = frag_uv(pos, {div}.0);\n{main}\n}}\n"
        );
        for (k, r) in seg.inputs.iter().enumerate() {
            body = body.replace(&placeholder(*r), &format!("t{k}"));
        }
        Ok(Pass {
            label,
            body,
            inputs: seg.inputs,
            output,
        })
    }

    /// Render a segment to a new full-resolution buffer; returns the buffer.
    fn flush(&mut self, seg: Seg, label: String, at: &Inst) -> Result<usize, Error> {
        let a = self.buf(1);
        let main = format!("    return {}(uv);", seg.last);
        let p = self.assemble(seg, &main, TexRef::Buf(a), label, at)?;
        self.graph.passes.push(p);
        Ok(a)
    }

    fn simple_pass(
        &mut self,
        label: String,
        input: &[TexRef],
        expr: impl Fn(&[String]) -> String,
        div: u32,
        at: &Inst,
    ) -> Result<usize, Error> {
        let out = self.buf(div);
        let mut seg = Seg::default();
        let names: Vec<String> = input.iter().map(|r| seg.tex(*r)).collect();
        let main = format!("    return {};", expr(&names));
        let p = self.assemble(seg, &main, TexRef::Buf(out), label, at)?;
        self.graph.passes.push(p);
        Ok(out)
    }

    fn blur2(
        &mut self,
        l: &str,
        src: usize,
        radius: &str,
        div: u32,
        at: &Inst,
    ) -> Result<usize, Error> {
        let r = radius.to_string();
        let h = self.simple_pass(
            format!("layer {l} blur →"),
            &[TexRef::Buf(src)],
            |t| format!("blur_dir({}, uv, vec2f(1.0, 0.0), {r})", t[0]),
            div,
            at,
        )?;
        let r = radius.to_string();
        self.simple_pass(
            format!("layer {l} blur ↓"),
            &[TexRef::Buf(h)],
            |t| format!("blur_dir({}, uv, vec2f(0.0, 1.0), {r})", t[0]),
            div,
            at,
        )
    }

    /// Generate picture arguments into `seg`; returns (arg index, function name).
    fn pictures(
        &mut self,
        prefix: &str,
        inst: &Inst,
        seg: &mut Seg,
    ) -> Result<Vec<(usize, String)>, Error> {
        let mut out = Vec::new();
        for (i, a) in inst.args.iter().enumerate() {
            if let ArgIR::Pic(l) = a {
                self.subs += 1;
                let sub = self.layer(&format!("{prefix}p{}", self.subs), l)?;
                seg.fns.push_str(&sub.fns);
                for r in sub.inputs {
                    seg.tex(r);
                }
                out.push((i, sub.last));
            }
        }
        Ok(out)
    }

    /// A shader written in the sketch: its function, then the call that uses it.
    fn user(
        &mut self,
        name: &str,
        prev: Option<&str>,
        inst: &Inst,
        seg: &mut Seg,
    ) -> Result<String, Error> {
        let f = inst.user.as_ref().expect("a sketch shader");
        let pics = self.pictures(name, inst, seg)?;
        let fname = format!("{name}_f");
        let mut renames: Vec<(&str, String)> = Vec::new();
        let mut call_args = String::new();
        for (i, (p, a)) in f.params.iter().zip(&inst.args).enumerate() {
            match a {
                ArgIR::Pic(_) => {
                    let sub = pics.iter().find(|x| x.0 == i).map(|x| x.1.clone()).unwrap_or_default();
                    renames.push((p.name.as_str(), sub));
                }
                a => {
                    call_args.push_str(", ");
                    call_args.push_str(&arg_expr(a, seg, None));
                }
            }
        }
        if let Some(prev) = prev {
            renames.push(("pic", prev.to_string()));
        }
        super::native::emit(&mut seg.fns, &fname, f, &renames);
        Ok(format!("{fname}(uv{call_args})"))
    }

    fn layer(&mut self, l: &str, layer: &Layer) -> Result<Seg, Error> {
        let mut seg = Seg::default();
        let expr = if layer.source.user.is_some() {
            self.user(&format!("{l}s0"), None, &layer.source, &mut seg)?
        } else {
            let pics = self.pictures(l, &layer.source, &mut seg)?;
            fill(layer.source.spec.wgsl, "", &layer.source, &mut seg, &pics)
        };
        seg.push(format!("{l}s0"), &expr);
        for (k, st) in layer.stages.iter().enumerate() {
            let name = format!("{l}s{}", k + 1);
            match st.spec.role {
                Role::Source => unreachable!(),
                Role::Color if st.user.is_some() => {
                    let prev = seg.last.clone();
                    let expr = self.user(&name, Some(&prev), st, &mut seg)?;
                    seg.push(name, &expr);
                }
                Role::Domain | Role::Color => {
                    let prev = seg.last.clone();
                    let pics = self.pictures(l, st, &mut seg)?;
                    let expr = fill(st.spec.wgsl, &prev, st, &mut seg, &pics);
                    seg.push(name, &expr);
                }
                Role::Tex(kind) => {
                    let label = format!("layer {l} → {}", st.spec.name);
                    let a = self.flush(std::mem::take(&mut seg), label, st)?;
                    let s = |i: usize| match &st.args[i] {
                        ArgIR::Scalar(k) => scalar(*k),
                        _ => unreachable!(),
                    };
                    let base = match kind {
                        Tex::Blur => {
                            let b = self.blur2(l, a, &format!("{} * 0.03", s(0)), 2, st)?;
                            format!("tx({}, uv)", seg.tex(TexRef::Buf(b)))
                        }
                        Tex::Bloom | Tex::Glow => {
                            let thr = if kind == Tex::Bloom {
                                s(1)
                            } else {
                                "0.0".into()
                            };
                            let radius = if kind == Tex::Bloom {
                                "0.045".into()
                            } else {
                                format!("(0.02 + {} * 0.1)", s(1))
                            };
                            let bright = self.simple_pass(
                                format!("layer {l} bright"),
                                &[TexRef::Buf(a)],
                                |t| format!("bright({}, uv, {thr})", t[0]),
                                4,
                                st,
                            )?;
                            let b = self.blur2(l, bright, &radius, 4, st)?;
                            let f = if kind == Tex::Bloom {
                                "bloom_mix"
                            } else {
                                "glow_mix"
                            };
                            format!(
                                "{f}(tx({}, uv), tx({}, uv), {})",
                                seg.tex(TexRef::Buf(a)),
                                seg.tex(TexRef::Buf(b)),
                                s(0)
                            )
                        }
                        Tex::Chroma => format!("chroma({}, uv, {})", seg.tex(TexRef::Buf(a)), s(0)),
                        Tex::Paint => {
                            format!("kuwahara({}, uv, {})", seg.tex(TexRef::Buf(a)), s(0))
                        }
                        Tex::Relief => format!(
                            "relief({}, uv, {}, {})",
                            seg.tex(TexRef::Buf(a)),
                            s(0),
                            s(1)
                        ),
                        Tex::Edges => format!("edges({}, uv, {})", seg.tex(TexRef::Buf(a)), s(0)),
                        Tex::Brush => format!(
                            "brush({}, uv, {}, {}, {})",
                            seg.tex(TexRef::Buf(a)),
                            s(0),
                            s(1),
                            s(2)
                        ),
                        Tex::ZoomBlur => format!(
                            "zoomblur({}, uv, {}, {}, {})",
                            seg.tex(TexRef::Buf(a)),
                            s(0),
                            s(1),
                            s(2)
                        ),
                        Tex::Trail => {
                            let h = self.graph.hists;
                            self.graph.hists += 1;
                            let mut ts = Seg::default();
                            let (ta, tp) = (ts.tex(TexRef::Buf(a)), ts.tex(TexRef::HistPrev(h)));
                            let main = format!(
                                "    return trail_mix(tx({ta}, uv), tx({tp}, uv), {});",
                                s(0)
                            );
                            let p = self.assemble(
                                ts,
                                &main,
                                TexRef::HistNext(h),
                                format!("layer {l} trail"),
                                st,
                            )?;
                            self.graph.passes.push(p);
                            format!("tx({}, uv)", seg.tex(TexRef::HistNext(h)))
                        }
                    };
                    seg.push(name, &base);
                }
            }
        }
        Ok(seg)
    }
}

pub fn generate(layers: &[Layer], images: &[String], helpers: &[Helper]) -> Result<Graph, Error> {
    let mut g = G {
        graph: Graph {
            images: images.to_vec(),
            ..Default::default()
        },
        subs: 0,
        helpers: super::native::helpers(helpers),
    };
    let mut fin = Seg::default();
    let mut main = String::from("    var acc = vec3f(0.0);\n");
    for (l, layer) in layers.iter().enumerate() {
        let seg = g.layer(&format!("l{l}"), layer)?;
        fin.fns.push_str(&seg.fns);
        for r in seg.inputs {
            if !fin.inputs.contains(&r) {
                fin.inputs.push(r);
            }
        }
        let _ = writeln!(
            main,
            "    acc = blend_layer(acc, {}(uv), {});",
            seg.last, layer.blend
        );
    }
    if layers.is_empty() {
        main.push_str("    acc = vec3f(0.957, 0.937, 0.902);\n");
    }
    main.push_str("    return vec4f(finish(acc, pos.xy), 1.0);");
    let at = layers
        .last()
        .map(|l| l.stages.last().unwrap_or(&l.source).clone());
    let at = at.unwrap_or_else(|| Inst {
        spec: &super::catalog::SPECS[0],
        args: vec![],
        span: 0..0,
        user: None,
    });
    let p = g.assemble(fin, &main, TexRef::Screen, "composite".into(), &at)?;
    g.graph.passes.push(p);
    Ok(g.graph)
}
