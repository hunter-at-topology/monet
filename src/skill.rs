//! The agent skill: everything an AI agent needs to write monet sketches, as one
//! Markdown document (Claude skill format). Generated from the catalog, so it can
//! never fall out of date. Copy it from the reference panel (⌘K), fetch it from
//! monet.hunterharloff.com/skill.md, or print it with `monet skill`.

use std::fmt::Write;

use crate::chain::catalog::{self, Arg, Role};
use crate::chain::native;
use crate::presets;

pub const SITE: &str = "https://monet.hunterharloff.com";

/// Sketches shown in full as examples (all presets, so they always compile).
const EXAMPLES: &[&str] = &["kali", "lilies", "poster", "pool", "pulse", "shader"];

fn words_of(a: &Arg) -> Option<&'static [&'static str]> {
    match a {
        Arg::Word(_, opts) => Some(opts),
        _ => None,
    }
}

fn entry(out: &mut String, name: &str, s: &catalog::Spec) {
    let mut line = format!("- `{}`: {}", catalog::signature_defaults(name, s), s.doc);
    for (arg, a) in s.args {
        if let Some(opts) = words_of(a) {
            let _ = write!(line, ". `{arg}` is one of {}", opts.iter().map(|o| format!("\"{o}\"")).collect::<Vec<_>>().join(", "));
        }
    }
    let _ = writeln!(out, "{line}");
}

/// The whole skill, as Markdown.
pub fn text() -> String {
    let mut o = String::new();
    let w = &mut o;
    let _ = writeln!(
        w,
        "---
name: monet
description: Write sketches for monet, a live-coding shader REPL ({SITE}). Use when asked to make a monet sketch, shader art, a generative design or an animated picture in monet, or to explain or fix monet code.
---

# monet

monet is a live-coding REPL for moving pictures. A sketch is a few lines of a small chain language; every keystroke re-renders on the GPU. It runs in the browser at {SITE} (and as a desktop app).

## How to hand a sketch over

Give the sketch in a code block, then a link that opens it:

    {SITE}/#s=<the sketch as UTF-8, URL-safe base64 without padding>

To make the link (any base64url encoder works; a trailing newline is fine), run:

    python3 -c \"import base64,sys; print('{SITE}/#s=' + base64.urlsafe_b64encode(sys.stdin.read().encode()).decode().rstrip('='))\" < sketch.monet

Check every name against the lists below: only these pictures, effects, signals and palettes exist. A wrong name is underlined with a \"Did you mean\" suggestion, and the last good picture keeps playing.

## The language

```monet
// a comment
mesh(\"dawn\")                 // a picture: each line is a layer, bottom first
  .flow(.25)                 // effects chain with `.`, one per line if you like
  .dither(\"riviera\", 4)
glints().blend(\"add\")        // a second layer, added on top
```

- **Arguments** are positional and all optional; defaults are shown below. `mesh()` alone already looks good.
- **Numbers** can be plain (`.5`, `-2`), a signal (`sine.range(1, 3).slow(4)`), a pattern string (`\"0 .5 1\"`), a slider (`slider(.5, 0, 1)`), or arithmetic (`sine * 2 + 1`).
- **Palettes** are a name (`\"dawn\"`), your own colours (`\"#f4a7b9 #a8c8f0\"`, up to 8), or a pattern that crossfades (`\"<dawn ember>\"`).
- **Variables:** `w = sine.range(0, 1)`, `p = mesh(\"dawn\").flow()`, `ink = \"#1d3fbb #f4efe6\"`; then use `w`, `p`, `ink`.
- **Pictures as arguments:** some effects take a picture, with its own chain: `displace(clouds(), .2)`, `mask(spot(.3).blur())`.
- **Effects belong to their layer.** `.grain()` at the end of the second line textures that layer only; there is no whole-scene effect (a chain can't follow `stack()`). To finish everything, put the texture on each layer, or build the scene as one layer: a picture can take others as arguments (`displace`, `mask`) or be written in WGSL.
- **Layers cover what's below** unless they're transparent by nature (`stars`, `glints`, `orbits`, `rays`, `spot` and friends) or you give them `.blend(\"screen\")`, `.blend(\"multiply\")`, `.opacity(.5)` or a `.mask(…)`.
- `stack(a, b, …)` groups layers; `speed(cps)` sets cycles per second for the whole sketch (default 0.5). Signals, patterns and `.slow()` all count in cycles, so `speed()` scales them all.
- **Fractals** (`mandelbrot`, `julia`, `kali`, …) live on a plane you can drag and scroll. `view(x, y, zoom)` sets where it starts: the centre point on the plane and a zoom (bigger is closer). It's one setting for the sketch, on a line of its own, anywhere. Leave it out to start at the fractal's home view (listed below).
- Effects in the *Light and lens* group each add a render pass: use a few, not ten.
- Sketch line order: settings (`speed`, `view`), then variables and shaders, then layers bottom to top. Only layers draw.
"
    );

    let _ = writeln!(w, "## Pictures\n\nStart a line with one of these.\n");
    for s in catalog::SPECS.iter().filter(|s| s.role == Role::Source && !s.hidden) {
        entry(w, s.name, s);
        if let Some([x, y, z]) = s.home {
            let _ = writeln!(w, "  - home view: `view({}, {}, {})`", catalog::fmt_num(x as f64), catalog::fmt_num(y as f64), catalog::fmt_num(z as f64));
        }
    }
    for (title, pick) in [
        ("Shape and motion", (|r: Role| r == Role::Domain) as fn(Role) -> bool),
        ("Colour and texture", |r: Role| r == Role::Color),
        ("Light and lens", |r: Role| matches!(r, Role::Tex(_))),
    ] {
        let _ = writeln!(w, "\n## Effects: {title}\n");
        for (n, s) in catalog::effects().filter(|(_, s)| pick(s.role)) {
            entry(w, n, s);
        }
        if title == "Light and lens" {
            let _ = writeln!(
                w,
                "- `blend(mode = \"normal\")`: how this layer mixes with the ones below. `mode` is one of {}",
                catalog::BLENDS.iter().map(|b| format!("\"{b}\"")).collect::<Vec<_>>().join(", ")
            );
        }
    }
    let _ = writeln!(
        w,
        "\n`mask` also takes a picture instead of a shape: `mask(spot(.3), 1)`. `dither` takes a level count, or a palette to dither into: `dither(\"riviera\", 4)`.\n"
    );

    let _ = writeln!(w, "## Signals\n\nAll run 0 to 1, once per cycle, and go anywhere a number does.\n");
    for (n, d) in catalog::SIGNALS {
        let _ = writeln!(w, "- `{n}`: {d}");
    }
    let _ = writeln!(w, "\nChain them:\n");
    for (_, d) in catalog::SIGNAL_METHODS {
        // "range(lo, hi): rescale…" → "- `.range(lo, hi)`: rescale…"
        let _ = writeln!(w, "- `.{}", d.replacen("): ", ")`: ", 1));
    }
    let _ = writeln!(w, "\n## Functions\n");
    for (_, d) in catalog::FUNCTIONS {
        let _ = writeln!(w, "- `{}", d.replacen("): ", ")`: ", 1));
    }
    let _ = writeln!(
        w,
        "
## Patterns

Any number or palette can be a pattern string, played once per cycle:

- `\"0 .5 1\"`: a sequence, each step gets a slice of the cycle
- `\"<1 2 3>\"`: one step per cycle
- `\"[0 1] 2\"`: nest steps inside one slice
- `\"0*2\"`: play a step faster
- `\"1@3 0\"`: give a step more weight
- `\"<dawn ember>\"`: palettes too; they crossfade

## Palettes
"
    );
    for (n, cols) in catalog::PALETTES {
        let _ = writeln!(w, "- `\"{n}\"`: {}", cols.join(" "));
    }

    let _ = writeln!(
        w,
        "
## Writing your own shaders (native WGSL)

When nothing in the catalog fits, define a picture or an effect in WGSL inside the sketch. It then works like a built-in: it chains, takes signals and sliders, and can be used before or after its definition.

```monet
// rings that breathe (the comment above becomes its docs)
picture breathe(colors = \"dawn\", rings = 8, speed = .5) {{
    let d = length(asp(uv));
    return vec4f(pal(colors, fract(d * rings - u.time * speed)), 1.0);
}}

// an effect reads the picture so far with pic(p), at any position
effect pane(amount = .04, count = 16) {{
    let x = fract(uv.x * count);
    return pic(uv + vec2f((x - 0.5) * amount, 0.0));
}}

breathe(\"<dawn ember>\", sine.range(4, 9).slow(4)).pane().grain(.03)
```

- A `picture` returns the colour at `uv` (0 to 1 across the frame, y up) as a `vec4f`. An `effect` returns the new colour and reads what's below with `pic(p)`.
- A parameter's default sets its type: a number (or none, meaning 0) is an `f32`; a palette string is a `u32` read with `pal(colors, t)` or `palc(colors, i)`; a picture like `other = clouds()` is called as `other(uv)`.
- Number parameters are always `f32`, palettes `u32`. Return `vec4f(rgb, 1.0)`, not a `vec3f`. Write float literals as `1.0`, not `1`, where the type matters; `let` is a constant, `var` a variable.
- Animate with `u.cycle` (follows `speed()` and patterns) or `u.time`; both pause with the sketch.
- The frame is `u`: {}.
- Helper functions go in a `wgsl {{ … }}` block; every shader can call them.
- Don't name a shader after a built-in picture, effect, signal or function (every name in this document is taken: `glow`, `ripple`, `wave`…). That's an error: \"`glow` is a built-in name. Pick another\". Don't name parameters `uv`, `pic`, `u` or a WGSL keyword.

In scope inside a shader:
",
        native::UNIFORMS.iter().map(|(n, d)| format!("`u.{n}` ({d})")).collect::<Vec<_>>().join(", ")
    );
    for (_, sig, d) in native::SHADER_NAMES.iter().filter(|(n, ..)| !matches!(*n, "let" | "var" | "return" | "for" | "if")) {
        let _ = writeln!(w, "- `{sig}`: {d}");
    }

    let _ = writeln!(
        w,
        "
## Making it beautiful

- Start from a picture with a palette that suits the mood, then add two or three effects. Restraint reads as taste.
- Movement: animate one or two numbers with slow signals (`sine.range(a, b).slow(8)`), not everything at once.
- Texture finishes a picture: `.grain(.03)`, `.canvas(.5)`, `.stipple(\"cobalt\")`, or `.brush()` to paint it.
- Layers: a quiet base plus one accent on top with `.blend(\"screen\")` or `\"add\"` usually beats many layers.
- Print looks: `stipple` maps dark to the first colour and light to the last, so a palette is inks then paper. Two inks on paper: `.stipple(\"#1d3fbb #e2553f #f4efe6\")` (or the built-in `\"riso\"`). One ink: `\"cobalt\"`. Feed it a calm monochrome picture (palette `\"mono\"`) for the cleanest print.

## Examples
"
    );
    for name in EXAMPLES {
        if let Some(p) = presets::find(name) {
            let _ = writeln!(w, "```monet\n{}```\n", p.source);
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skill_knows_every_name() {
        let s = text();
        for spec in catalog::SPECS.iter().filter(|s| !s.hidden) {
            assert!(s.contains(&format!("`{}(", catalog::written(spec))), "{} missing", spec.name);
        }
        for (n, _) in catalog::PALETTES {
            assert!(s.contains(&format!("`\"{n}\"`")), "palette {n} missing");
        }
        for (n, _) in catalog::SIGNAL_METHODS {
            assert!(s.contains(&format!("`.{n}(")), "method {n} missing: {s}");
        }
        assert!(s.starts_with("---\nname: monet\n"));
        assert!(!s.contains('—'), "no em dashes");
    }

    #[test]
    fn every_sketch_in_the_skill_compiles() {
        let s = text();
        let blocks: Vec<&str> = s.split("```monet\n").skip(1).map(|b| b.split("```").next().unwrap()).collect();
        assert!(blocks.len() >= 6);
        for b in blocks {
            if let Err(e) = crate::chain::compile(b) {
                panic!("{b}\n→ {} at {:?}", e.msg, e.span);
            }
        }
    }

    #[test]
    fn the_share_recipe_matches_the_app() {
        // the python one-liner in the skill and crate::share agree
        let src = "mesh(\"dawn\")\n";
        let link = format!("{SITE}/#s={}", crate::share::encode(src.as_bytes()));
        assert_eq!(link, format!("{SITE}/#s=bWVzaCgiZGF3biIpCg"));
    }
}
