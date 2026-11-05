//! QA for autocomplete, over every preset and a set of real sketches.
//!
//! 1. **Typing replay:** type each document character by character. Wherever the popup
//!    would open, the name the author went on to write must be offered (and near the top).
//! 2. **Validity:** at every name in every document, ask for completions, put each
//!    suggestion in place of the name, and compile. A suggestion the compiler rejects as
//!    the wrong kind of thing (an effect on a number, a signal method on a picture, a
//!    word an argument doesn't take…) is a bug.

use super::complete::{self, Kind};
use crate::chain::{self, catalog};
use crate::presets;

/// Real sketches people wrote, beyond the presets.
const SKETCHES: &[&str] = &[
    "silk(\"candy\", .6)\n  .hue(sine.range(-20, 20).slow(8))\n  .relief(.8)\n  .grain(.03)\n  .blur(2)\n",
    "waves(\"harbour\").blur().canvas(4).brush(0.5).blur(0.2).dither(2, 7).noise().mask(\"circle\", 0.2, 0)\n",
    "view(-0.7453, 0.1127, 180)\n\nlilies(\"mono\", 2, 2).dither(2, 5).noise(2, 0, 1)\n",
    "water(\"sunlit\", slider(3.00, 0, 3), .8)\n",
    "lilies(\"dawn\", 1.3).canvas(10)\n\ncells().blend(\"screen\")\n",
    "w = sine.range(0, 1).slow(4)\np = mesh(\"dawn\")\n  .flow(w)\np.zoom(1.2)\n",
    "speed(1)\ngradient(\"dusk\", 75).displace(clouds(\"mono\", 3), .4)\nstripes(\"citrus\").mask(spot(.3).blur(.4))\n",
    "mesh(\"<dawn ember>\").noise(.2, 0, 2, 1, \"uniform\").blend(\"multiply\")\n",
    "stack(mesh(\"dawn\"), stars())\nmandelbrot(\"fire\", 300).zoom(saw.range(1, 2)).mask(\"ring\", .3)\n",
    "mesh(\"dawn\").zoom(mouse.x.range(1, 3)).rotate(time.mul(10))\n",
    // shaders written in the sketch
    "// soft rings\npicture breathe(colors = \"dawn\", rings = 8, over = clouds()) {\n    let d = length(asp(uv));\n    return vec4f(pal(colors, fract(d * rings - u.time)), 1.0);\n}\neffect smear(amount = .02) {\n    return pic(uv + amount);\n}\nwgsl {\n    fn sd_box(p: vec2f) -> f32 { return length(p); }\n}\nbreathe(\"ember\", sine.range(4, 9))\n  .smear(.01)\n  .grain()\nmesh(\"dawn\").displace(breathe(), .2).smear()\n",
];

fn documents() -> Vec<(String, String)> {
    let mut docs: Vec<(String, String)> = presets::PRESETS.iter().map(|p| (p.name.to_string(), p.source.to_string())).collect();
    docs.extend(SKETCHES.iter().enumerate().map(|(k, s)| (format!("sketch{k}"), s.to_string())));
    docs
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Byte ranges of names in code (not comments, not numbers), with whether each sits in a string.
fn names(src: &str) -> Vec<(usize, usize, bool)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        if quote.is_none() && c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
                i += 1;
                continue;
            }
        } else if c == b'"' {
            quote = Some(c);
            i += 1;
            continue;
        }
        // shader definitions and their WGSL are checked elsewhere
        let line_start = i == 0 || b[i - 1] == b'\n';
        if quote.is_none() && line_start && ["picture ", "effect ", "wgsl "].iter().any(|k| src[i..].starts_with(k)) {
            match src[i..].find('{') {
                Some(o) => i = crate::chain::lex::block_end(b, i + o).map_or(b.len(), |e| e + 1),
                None => i = b.len(),
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            // skip hex colours and numbers glued to letters
            if s > 0 && (b[s - 1] == b'#' || b[s - 1].is_ascii_digit()) {
                continue;
            }
            out.push((s, i, quote.is_some()));
            continue;
        }
        i += 1;
    }
    out
}

/// Is this a name autocomplete should know about?
fn known(name: &str, src: &str) -> bool {
    catalog::find(name).is_some_and(|s| !s.hidden)
        || catalog::palette(name).is_some()
        || catalog::word_doc(name).is_some()
        || catalog::SIGNALS.iter().any(|s| s.0.split('.').next() == Some(name) || s.0.ends_with(&format!(".{name}")))
        || catalog::SIGNAL_METHODS.iter().any(|s| s.0 == name)
        || catalog::FUNCTIONS.iter().any(|s| s.0 == name)
        || name == "blend"
        || src.lines().any(|l| l.trim_start().starts_with(&format!("{name} =")))
        || crate::chain::native::outline(src).0.iter().any(|d| d.name == name)
}

/// Is the name ending at `e` being assigned (`name = …`)? Typing a new name offers nothing useful.
fn assigns(src: &str, e: usize) -> bool {
    let rest = src[e..].trim_start_matches([' ', '\t']);
    rest.starts_with('=') && !rest.starts_with("==")
}

/// Errors that mean "this kind of thing can't go here", as opposed to argument trouble.
fn kind_error(msg: &str) -> bool {
    // `blend("")` is accepted with its quotes empty, so the list of modes opens next
    !(msg.contains("isn't one of") || msg.contains("blend needs a mode") || msg.contains(" takes ") || msg.contains("needs numbers") || msg.contains("must be") || msg.contains("is a palette") || msg.contains("no palette called") || msg.contains("expected a palette"))
}

#[test]
fn autocomplete_qa() {
    let mut problems: Vec<String> = Vec::new();
    let mut ranks: Vec<usize> = Vec::new();
    let line_of = |src: &str, at: usize| src[..at].matches('\n').count() + 1;

    for (doc, src) in documents() {
        let b = src.as_bytes();
        let toks = names(&src);

        // 1. typing replay: the text so far, nothing after the cursor
        for &(s, e, _) in &toks {
            let name = &src[s..e];
            if !known(name, &src) || assigns(&src, e) {
                continue;
            }
            let opens_at_start = s > 0 && matches!(b[s - 1], b'.' | b'"' | b'<' | b' ') && (b[s - 1] != b' ' || src[..s].trim_end().ends_with(['<']));
            let first = if opens_at_start { s } else { s + 1 };
            for p in first..e {
                let typed = &src[s..p];
                match complete::complete(&src[..p], p) {
                    Some(c) => match c.items.iter().position(|i| i.label == name) {
                        Some(r) => ranks.push(r),
                        None => {
                            let top: Vec<_> = c.items.iter().take(4).map(|i| i.label.as_str()).collect();
                            problems.push(format!("[replay] {doc}:{}: typing `{typed}` toward `{name}` offers {top:?}, not `{name}`", line_of(&src, s)));
                            break;
                        }
                    },
                    None if p > s => {
                        problems.push(format!("[replay] {doc}:{}: typing `{typed}` toward `{name}` opens nothing", line_of(&src, s)));
                        break;
                    }
                    None => {}
                }
            }
        }

        // 2. validity: every suggestion, compiled in place
        if chain::compile(&src).is_err() {
            continue; // only judge suggestions against documents that compile as written
        }
        for &(s, e, in_str) in &toks {
            let name = &src[s..e];
            if !known(name, &src) || assigns(&src, e) {
                continue;
            }
            for k in [0usize, 1] {
                if k == 0 && !(s > 0 && matches!(b[s - 1], b'.' | b'"')) {
                    continue;
                }
                let p = s + k;
                let trimmed = format!("{}{}", &src[..p], &src[e..]);
                let Some(c) = complete::complete(&trimmed, p) else { continue };
                for item in &c.items {
                    // what accepting would really leave: the insert, minus brackets already there
                    let ins = if in_str { item.label.clone() } else if b.get(e) == Some(&b'(') { item.insert.split('(').next().unwrap_or("").to_string() } else { item.insert.clone() };
                    let candidate = format!("{}{}{}", &src[..s], ins, &src[e..]);
                    if let Err(err) = chain::compile(&candidate) {
                        let hit = err.span.start < s + ins.len() && err.span.end > s;
                        if hit && kind_error(&err.msg) && item.kind != Kind::Var {
                            problems.push(format!(
                                "[invalid] {doc}:{}: at `{}` (typed `{}`) suggests `{}` → {}",
                                line_of(&src, s),
                                name,
                                &src[s..p],
                                item.label,
                                err.msg
                            ));
                        }
                    }
                }
            }
        }
    }
    problems.sort();
    problems.dedup();
    let mean_rank = ranks.iter().sum::<usize>() as f64 / ranks.len().max(1) as f64;
    let top1 = ranks.iter().filter(|&&r| r == 0).count() as f64 / ranks.len().max(1) as f64;
    eprintln!("autocomplete QA: {} checks, intended name first {:.0}% of the time, mean rank {:.2}", ranks.len(), top1 * 100.0, mean_rank);
    for p in problems.iter().take(80) {
        eprintln!("  {p}");
    }
    assert!(problems.is_empty(), "{} autocomplete problems (listed above)", problems.len());
}
