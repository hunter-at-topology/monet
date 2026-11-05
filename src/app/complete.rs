//! Autocomplete and signature hints, as pure functions over (text, cursor).
//! Cursor positions here are *byte* offsets.
//!
//! Suggestions follow context: only what can actually go at the cursor.
//! After a dot, the chain is walked back to its start, so a picture is offered
//! effects and a signal is offered signal methods. Inside quotes, an argument
//! offers its palettes or words. Inside an argument, values come first.

use std::ops::Range;

use crate::chain::build::UArg;
use crate::chain::catalog::{self, Arg, Role};
use crate::chain::native::{self, Outline};
use crate::chain::parse::DefKind;

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    /// Text that replaces the typed prefix.
    pub insert: String,
    /// Where the cursor goes, relative to the start of `insert`.
    pub cursor: usize,
    /// One line about it.
    pub detail: String,
    /// The full signature with defaults, for the popup's footer (may be empty).
    pub sig: String,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Source,
    Effect,
    Signal,
    Func,
    Palette,
    Word,
    Var,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    /// The typed prefix (bytes) that an accepted item replaces.
    pub range: Range<usize>,
    pub items: Vec<Item>,
}

// ── scanning ─────────────────────────────────────────────────────────────────

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Start of the identifier ending at `end`.
fn ident_start(b: &[u8], end: usize) -> usize {
    let mut s = end;
    while s > 0 && is_ident(b[s - 1]) {
        s -= 1;
    }
    s
}

/// Back over spaces, tabs, newlines and `// comments` (a comment's full stop is not a dot).
fn skip_ws_back(b: &[u8], mut i: usize) -> usize {
    loop {
        while i > 0 && b[i - 1].is_ascii_whitespace() {
            i -= 1;
        }
        if i == 0 {
            return 0;
        }
        let line = b[..i].iter().rposition(|&c| c == b'\n').map_or(0, |n| n + 1);
        match comment_start(&b[line..i]) {
            Some(c) => i = line + c,
            None => return i,
        }
    }
}

/// Where `//` starts a comment in one line (outside strings).
fn comment_start(line: &[u8]) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for k in 0..line.len() {
        let c = line[k];
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if matches!(c, b'"' | b'\'' | b'`') => quote = Some(c),
            None if c == b'/' && line.get(k + 1) == Some(&b'/') => return Some(k),
            None => {}
        }
    }
    None
}

/// The `(` matching the `)` at `close`.
fn open_paren(b: &[u8], close: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = close + 1;
    let mut quote: Option<u8> = None;
    while i > 0 {
        i -= 1;
        let c = b[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            b'"' | b'\'' | b'`' => quote = Some(c),
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Is `at` inside a string on its line? Returns the string's opening quote offset.
fn string_start(src: &str, at: usize) -> Option<usize> {
    let line_start = src[..at].rfind('\n').map_or(0, |i| i + 1);
    let mut open: Option<(usize, u8)> = None;
    let b = src.as_bytes();
    let mut i = line_start;
    while i < at {
        let c = b[i];
        match open {
            Some((_, q)) if c == q => open = None,
            None if c == b'/' && b.get(i + 1) == Some(&b'/') => return None,
            None if matches!(c, b'"' | b'\'' | b'`') => open = Some((i, c)),
            _ => {}
        }
        i += 1;
    }
    open.map(|o| o.0)
}

/// What a dotted chain hangs off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recv {
    Picture,
    Signal,
    /// `mouse.` itself: x or y
    Mouse,
    /// a stack, a setting or a number: nothing chains
    Nothing,
    /// a string: a palette or a pattern
    Text,
    /// can't tell (offer both, effects first)
    Unknown,
}

/// The value the `.` at `dot` is called on, walking back along the chain.
fn receiver(src: &str, dot: usize, depth: u8) -> Recv {
    let b = src.as_bytes();
    let i = skip_ws_back(b, dot);
    if i == 0 || depth > 6 {
        return Recv::Unknown;
    }
    match b[i - 1] {
        b')' => {
            let Some(open) = open_paren(b, i - 1) else { return Recv::Unknown };
            let s = ident_start(b, open);
            if s == open {
                return Recv::Unknown; // a bracketed expression
            }
            let before = skip_ws_back(b, s);
            if before > 0 && b[before - 1] == b'.' {
                match receiver(src, before - 1, depth + 1) {
                    Recv::Mouse => Recv::Signal,
                    r => r,
                }
            } else {
                root(src, &src[s..open], depth)
            }
        }
        b'"' | b'\'' | b'`' => Recv::Signal, // a pattern string
        c if is_ident(c) => {
            let s = ident_start(b, i);
            if b[s].is_ascii_digit() {
                return Recv::Nothing;
            }
            let before = skip_ws_back(b, s);
            if before > 0 && b[before - 1] == b'.' {
                match receiver(src, before - 1, depth + 1) {
                    Recv::Mouse => Recv::Signal, // mouse.x
                    r => r,
                }
            } else {
                root(src, &src[s..i], depth)
            }
        }
        _ => Recv::Unknown,
    }
}

/// The shaders this sketch defines.
fn mine(src: &str) -> Vec<Outline> {
    native::outline(src).0
}

/// A picture: built in, or written in this sketch.
fn is_picture_name(src: &str, name: &str) -> bool {
    catalog::find(name).is_some_and(|s| s.role == Role::Source && !s.hidden)
        || mine(src).iter().any(|d| d.kind == DefKind::Picture && d.name == name)
}

/// What a chain's first name makes it.
fn root(src: &str, name: &str, depth: u8) -> Recv {
    if is_picture_name(src, name) {
        return Recv::Picture;
    }
    match name {
        "mouse" => return Recv::Mouse,
        "slider" | "time" => return Recv::Signal,
        "stack" | "speed" | "view" => return Recv::Nothing,
        _ => {}
    }
    if catalog::SIGNALS.iter().any(|s| s.0.split('.').next() == Some(name)) {
        return Recv::Signal;
    }
    match assignment(src, name) {
        Some(expr) if expr.starts_with(['"', '\'', '`']) && !expr.contains(").") => Recv::Text,
        Some(expr) => {
            let e = format!("{expr}.");
            receiver(&e, e.len() - 1, depth + 1)
        }
        None => Recv::Unknown,
    }
}

/// The expression assigned to `name` (`name = …`, chains on following lines included).
fn assignment(src: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = src.lines().collect();
    for (k, line) in lines.iter().enumerate() {
        let t = line.trim_start().trim_start_matches("$:").trim_start();
        let Some(rest) = t.strip_prefix(name) else { continue };
        let rest = rest.trim_start();
        let Some(expr) = rest.strip_prefix('=') else { continue };
        if expr.starts_with('=') {
            continue;
        }
        let mut out = expr.trim().to_string();
        for next in &lines[k + 1..] {
            if next.trim_start().starts_with('.') {
                out.push_str(next.trim());
            } else {
                break;
            }
        }
        return Some(out);
    }
    None
}

/// Names assigned before `upto`, and what each holds.
fn variables(src: &str, upto: usize) -> Vec<(String, Recv)> {
    let mut out = Vec::new();
    for line in src[..upto].lines() {
        let t = line.trim_start().trim_start_matches("$:").trim_start();
        let n = t.bytes().take_while(|&c| is_ident(c)).count();
        if n == 0 || t.as_bytes()[0].is_ascii_digit() {
            continue;
        }
        let rest = t[n..].trim_start();
        if rest.starts_with('=') && !rest.starts_with("==") {
            let name = &t[..n];
            if out.iter().all(|(v, _): &(String, Recv)| v != name) {
                out.push((name.to_string(), root(src, name, 0)));
            }
        }
    }
    out
}

/// What the argument at a position takes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
    Num,
    /// dither's first argument: a level count, or a palette
    NumOrPal,
    Pal,
    Word(&'static [&'static str]),
    /// mask's first argument: a shape, or a picture to mask by
    WordOrPic(&'static [&'static str]),
    Pic,
    Other,
}

fn slot(src: &str, at: usize) -> Option<Slot> {
    let (name, k, dotted) = call_at(src, at)?;
    if let Some(d) = mine(src).into_iter().find(|d| d.name == name) {
        let (_, default) = d.params.get(k)?;
        return Some(match native::param_kind(default.as_deref(), |n| is_picture_name(src, n)) {
            UArg::Num => Slot::Num,
            UArg::Pal => Slot::Pal,
            UArg::Pic => Slot::Pic,
        });
    }
    let spec = if dotted { catalog::effect(&name) } else { catalog::find(&name).filter(|s| !s.hidden) };
    if let Some(s) = spec {
        if s.role == Role::Source && dotted {
            return None;
        }
        if name == "dither" && k == 0 {
            return Some(Slot::NumOrPal);
        }
        if name == "mask" && k == 0 {
            return Some(Slot::WordOrPic(catalog::MASKS));
        }
        return Some(match &s.args.get(k)?.1 {
            Arg::Num(_) => Slot::Num,
            Arg::Pal(_) => Slot::Pal,
            Arg::Word(_, o) => Slot::Word(o),
            Arg::Pic(_) => Slot::Pic,
            Arg::Color(_) | Arg::Path => Slot::Other,
        });
    }
    Some(match name.as_str() {
        "blend" => Slot::Word(catalog::BLENDS),
        "stack" => Slot::Pic,
        _ => Slot::Num,
    })
}

// ── items ────────────────────────────────────────────────────────────────────

fn call(name: &str, has_args: bool) -> (String, usize) {
    if has_args { (format!("{name}()"), name.len() + 1) } else { (format!("{name}()"), name.len() + 2) }
}

fn sources() -> Vec<Item> {
    catalog::SPECS
        .iter()
        .filter(|s| !s.hidden && s.role == Role::Source)
        .map(|s| {
            let (insert, cursor) = call(s.name, !s.args.is_empty());
            Item { label: s.name.into(), insert, cursor, detail: s.doc.into(), sig: catalog::signature_defaults(s.name, s), kind: Kind::Source }
        })
        .collect()
}

fn effects() -> Vec<Item> {
    let mut v: Vec<Item> = catalog::effects()
        .map(|(n, s)| {
            let (insert, cursor) = call(n, !s.args.is_empty());
            Item { label: n.into(), insert, cursor, detail: s.doc.into(), sig: catalog::signature_defaults(n, s), kind: Kind::Effect }
        })
        .collect();
    v.push(Item {
        label: "blend".into(),
        insert: "blend(\"\")".into(),
        cursor: 7,
        detail: "how this layer mixes with the ones below".into(),
        sig: format!("blend(mode = \"normal\")   {}", catalog::BLENDS.join(" ")),
        kind: Kind::Effect,
    });
    v
}

/// The sketch's own pictures or effects, offered like built-ins.
fn user_items(src: &str, kind: DefKind) -> Vec<Item> {
    mine(src)
        .into_iter()
        .filter(|d| d.kind == kind)
        .map(|d| {
            let (insert, cursor) = call(&d.name, !d.params.is_empty());
            let k = if kind == DefKind::Picture { Kind::Source } else { Kind::Effect };
            Item { label: d.name.clone(), insert, cursor, detail: d.summary(), sig: d.signature(), kind: k }
        })
        .collect()
}

fn signal_methods() -> Vec<Item> {
    catalog::SIGNAL_METHODS
        .iter()
        .map(|(m, d)| {
            let (sig, detail) = split_sig(m, d);
            let takes = !sig.ends_with("()");
            let (insert, cursor) = call(m, takes);
            Item { label: m.to_string(), insert, cursor, detail, sig: format!(".{sig}"), kind: Kind::Signal }
        })
        .collect()
}

fn signals() -> Vec<Item> {
    let mut v: Vec<Item> = Vec::new();
    for (name, d) in catalog::SIGNALS {
        let head = name.split('.').next().unwrap_or(name);
        if v.iter().any(|i| i.label == head) {
            continue;
        }
        let (insert, detail) = if head == "mouse" { ("mouse.".to_string(), "pointer position: mouse.x, mouse.y (0 to 1)".to_string()) } else { (head.to_string(), d.to_string()) };
        let cursor = insert.len();
        v.push(Item { label: head.into(), insert, cursor, detail, sig: String::new(), kind: Kind::Signal });
    }
    v
}

fn functions(only: &[&str]) -> Vec<Item> {
    catalog::FUNCTIONS
        .iter()
        .filter(|(n, _)| only.contains(n))
        .map(|(n, d)| {
            let (sig, detail) = split_sig(n, d);
            let (insert, cursor) = match *n {
                "slider" => ("slider(.5, 0, 1)".to_string(), 7),
                _ => call(n, true),
            };
            Item { label: n.to_string(), insert, cursor, detail, sig, kind: Kind::Func }
        })
        .collect()
}

/// "range(lo, hi): rescale…" → ("range(lo, hi)", "rescale…").
pub fn split_sig(name: &str, doc: &str) -> (String, String) {
    match doc.split_once("): ") {
        Some((sig, rest)) if sig.starts_with(&format!("{name}(")) => (format!("{sig})"), rest.to_string()),
        _ => (format!("{name}()"), doc.to_string()),
    }
}

fn palettes(quoted: bool) -> Vec<Item> {
    catalog::PALETTES
        .iter()
        .map(|(name, cols)| {
            let insert = if quoted { format!("\"{name}\"") } else { name.to_string() };
            let cursor = insert.len();
            Item { label: name.to_string(), insert, cursor, detail: format!("palette · {} colours", cols.len()), sig: cols.join(" "), kind: Kind::Palette }
        })
        .collect()
}

fn words(opts: &[&str], quoted: bool) -> Vec<Item> {
    opts.iter()
        .map(|w| {
            let insert = if quoted { format!("\"{w}\"") } else { w.to_string() };
            let cursor = insert.len();
            let detail = catalog::word_doc(w).map_or_else(|| format!("one of {}", opts.join(", ")), str::to_string);
            Item { label: w.to_string(), insert, cursor, detail, sig: format!("one of {}", opts.join(", ")), kind: Kind::Word }
        })
        .collect()
}

fn vars(src: &str, upto: usize, want: &[Recv]) -> Vec<Item> {
    variables(src, upto)
        .into_iter()
        .filter(|(_, r)| want.contains(r) || *r == Recv::Unknown)
        .map(|(n, r)| {
            let what = match r {
                Recv::Picture => "a picture",
                Recv::Signal => "a signal",
                Recv::Text => "palette or pattern",
                _ => "a value",
            };
            let cursor = n.len();
            Item { label: n.clone(), insert: n, cursor, detail: format!("your {what}, set above"), sig: String::new(), kind: Kind::Var }
        })
        .collect()
}

// ── ranking ──────────────────────────────────────────────────────────────────

fn subsequence(hay: &str, needle: &str) -> bool {
    let mut h = hay.chars();
    needle.chars().all(|c| h.any(|x| x == c))
}

/// 0 starts with, 1 contains, 2 letters in order, 3 the description mentions it.
fn score(item: &Item, p: &str) -> Option<u8> {
    if p.is_empty() {
        return Some(0);
    }
    let label = item.label.to_ascii_lowercase();
    if label.starts_with(p) {
        Some(0)
    } else if label.contains(p) {
        Some(1)
    } else if p.len() >= 2 && subsequence(&label, p) {
        Some(2)
    } else if p.len() >= 3 && item.detail.to_ascii_lowercase().contains(p) {
        Some(3)
    } else {
        None
    }
}

/// How often each name appears as code (not in comments) in `src`.
fn name_counts(src: &str) -> std::collections::HashMap<&str, usize> {
    let mut counts = std::collections::HashMap::new();
    for line in src.lines() {
        let code = &line[..comment_start(line.as_bytes()).unwrap_or(line.len())];
        for w in code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            if w.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                *counts.entry(w).or_insert(0) += 1;
            }
        }
    }
    counts
}

/// How much the presets use each name: what people reach for most.
fn corpus() -> &'static std::collections::HashMap<String, usize> {
    static CORPUS: std::sync::OnceLock<std::collections::HashMap<String, usize>> = std::sync::OnceLock::new();
    CORPUS.get_or_init(|| {
        let mut all = std::collections::HashMap::new();
        for p in crate::presets::PRESETS {
            for (w, n) in name_counts(p.source) {
                *all.entry(w.to_string()).or_insert(0) += n;
            }
        }
        all
    })
}

/// Best first: how well it matches, then the kind that fits, then what this sketch
/// already uses, then what the presets use most, then the alphabet.
fn rank(items: Vec<Item>, prefix: &str, prefer: Option<Kind>, src: &str) -> Vec<Item> {
    let p = prefix.to_ascii_lowercase();
    let here = name_counts(src);
    let usage = |i: &Item| {
        // the word being typed isn't "used" yet; and an effect already in the sketch is
        // rarely wanted again, so effects rank by what the presets use, not this sketch
        let mine = if i.kind == Kind::Effect { 0 } else { here.get(i.label.as_str()).copied().unwrap_or(0).saturating_sub(usize::from(i.label == prefix)) };
        mine * 4 + corpus().get(&i.label).copied().unwrap_or(0)
    };
    let mut scored: Vec<(u8, bool, std::cmp::Reverse<usize>, Item)> =
        items.into_iter().filter_map(|i| score(&i, &p).map(|s| (s, Some(i.kind) != prefer, std::cmp::Reverse(usage(&i)), i))).collect();
    scored.sort_by(|a, b| (a.0, a.1, a.2, &a.3.label).cmp(&(b.0, b.1, b.2, &b.3.label)));
    let mut out: Vec<Item> = Vec::new();
    for (.., i) in scored {
        if !out.iter().any(|o| o.label == i.label) {
            out.push(i);
        }
    }
    out
}

// ── completion ───────────────────────────────────────────────────────────────

/// What could be typed at `cursor`? `None` when nothing sensible.
pub fn complete(src: &str, cursor: usize) -> Option<Completion> {
    let cursor = cursor.min(src.len());
    let b = src.as_bytes();
    let start = ident_start(b, cursor);
    let prefix = &src[start..cursor];
    if prefix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }

    // inside a shader's WGSL
    if let Some(block) = native::block_at(src, cursor)
        && cursor > block.start
    {
        return wgsl(src, cursor, start, block);
    }

    // inside a string: the argument's palettes or words
    if let Some(q) = string_start(src, cursor) {
        let mut items = match slot(src, q) {
            Some(Slot::Pal | Slot::NumOrPal) => palettes(false),
            Some(Slot::Word(o) | Slot::WordOrPic(o)) => words(o, false),
            Some(_) => return None, // numbers, patterns, colours, paths
            None if !prefix.is_empty() => palettes(false),
            None => return None,
        };
        // right after the opening quote, accepting closes the string too
        let fresh = start == q + 1;
        let next = b.get(cursor).copied();
        for i in &mut items {
            if next == Some(b[q]) {
                i.cursor += 1; // hop over the closing quote
            } else if fresh && !next.is_some_and(is_ident) {
                i.insert.push(b[q] as char);
                i.cursor += 1;
            }
        }
        return finish(rank(items, prefix, None, src), start..cursor, prefix);
    }

    let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
    if comment_start(&b[line_start..start]).is_some() {
        return None;
    }
    // naming a shader or its parameters: nothing to suggest (defaults are values, though)
    let head = src[line_start..start].trim_start();
    if (head.starts_with("picture ") || head.starts_with("effect ")) && !src[line_start..start].trim_end().ends_with('=') {
        return None;
    }

    // `.name`: what the chain so far can take
    if start > 0 && b[start - 1] == b'.' {
        let items = match receiver(src, start - 1, 0) {
            Recv::Picture => {
                let mut v = effects();
                v.extend(user_items(src, DefKind::Effect));
                v
            }
            Recv::Signal | Recv::Text => signal_methods(),
            Recv::Mouse => ["x", "y"]
                .iter()
                .map(|a| Item { label: a.to_string(), insert: a.to_string(), cursor: 1, detail: "pointer position, 0 to 1".into(), sig: String::new(), kind: Kind::Signal })
                .collect(),
            Recv::Unknown => {
                let mut v = effects();
                v.extend(user_items(src, DefKind::Effect));
                v.extend(signal_methods());
                v
            }
            Recv::Nothing => return None,
        };
        return finish(no_double_brackets(rank(items, prefix, None, src), b, cursor), start..cursor, prefix);
    }

    if prefix.is_empty() {
        return None;
    }
    let line_head = src[line_start..start].trim();
    let (items, prefer) = if let Some(s) = slot(src, start) {
        match s {
            Slot::Pic => {
                let mut v = sources();
                v.extend(user_items(src, DefKind::Picture));
                v.extend(vars(src, line_start, &[Recv::Picture]));
                (v, Some(Kind::Source))
            }
            Slot::Num => {
                let mut v = signals();
                v.extend(functions(&["slider"]));
                v.extend(vars(src, line_start, &[Recv::Signal, Recv::Text]));
                (v, Some(Kind::Signal))
            }
            Slot::NumOrPal => {
                let mut v = palettes(true);
                v.extend(signals());
                v.extend(vars(src, line_start, &[Recv::Signal, Recv::Text]));
                (v, Some(Kind::Palette))
            }
            Slot::Pal => {
                let mut v = palettes(true);
                v.extend(vars(src, line_start, &[Recv::Text]));
                (v, Some(Kind::Var))
            }
            Slot::Word(o) => (words(o, true), None),
            Slot::WordOrPic(o) => {
                let mut v = words(o, true);
                v.extend(sources());
                v.extend(user_items(src, DefKind::Picture));
                v.extend(vars(src, line_start, &[Recv::Picture]));
                (v, Some(Kind::Word))
            }
            Slot::Other => return None,
        }
    } else if line_head.is_empty() || line_head == "$:" {
        // a new line: something that can start one
        let mut v = sources();
        v.extend(user_items(src, DefKind::Picture));
        v.extend(functions(&["stack", "speed", "view"]));
        v.extend(definers());
        v.extend(vars(src, line_start, &[Recv::Picture]));
        (v, Some(Kind::Source))
    } else if line_head.ends_with('=') {
        let mut v = sources();
        v.extend(user_items(src, DefKind::Picture));
        v.extend(signals());
        v.extend(functions(&["slider", "stack"]));
        (v, None)
    } else {
        let mut v = signals();
        v.extend(functions(&["slider"]));
        v.extend(vars(src, line_start, &[Recv::Signal]));
        (v, Some(Kind::Signal))
    };
    finish(no_double_brackets(rank(items, prefix, prefer, src), b, cursor), start..cursor, prefix)
}

/// Starting a line: write your own shader.
fn definers() -> Vec<Item> {
    let mk = |label: &str, insert: &str, cursor: usize, detail: &str| Item {
        label: label.into(),
        insert: insert.into(),
        cursor,
        detail: detail.into(),
        sig: String::new(),
        kind: Kind::Func,
    };
    vec![
        mk(
            "picture",
            "picture name(amount = .5) {\n    return vec4f(uv, amount, 1.0);\n}",
            8,
            "write your own picture in WGSL: return the colour at uv",
        ),
        mk(
            "effect",
            "effect name(amount = .5) {\n    let c = pic(uv);\n    return vec4f(c.rgb * amount, c.a);\n}",
            7,
            "write your own effect in WGSL: pic(p) is the picture so far",
        ),
        mk("wgsl", "wgsl {\n    \n}", 11, "helper functions your shaders can call"),
    ]
}

/// Inside a shader block: WGSL, the library, this shader's parameters and locals.
fn wgsl(src: &str, cursor: usize, start: usize, block: Range<usize>) -> Option<Completion> {
    let b = src.as_bytes();
    let prefix = &src[start..cursor];
    let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
    let code = &src[line_start..start];
    if code.contains("//") {
        return None;
    }
    if start > 0 && b[start - 1] == b'.' {
        // u.time and friends; other members (swizzles, fields) are yours
        if start >= 2 && b[start - 2] == b'u' && (start < 3 || !is_ident(b[start - 3])) {
            let items = native::UNIFORMS
                .iter()
                .map(|(n, d)| Item { label: n.to_string(), insert: n.to_string(), cursor: n.len(), detail: d.to_string(), sig: format!("u.{n}"), kind: Kind::Signal })
                .collect();
            return finish(rank(items, prefix, None, src), start..cursor, prefix);
        }
        return None;
    }
    if prefix.is_empty() {
        return None;
    }
    let (defs, blocks) = native::outline(src);
    let def = defs.iter().find(|d| d.body == block);
    let effect = def.is_some_and(|d| d.kind == DefKind::Effect);
    let mut items: Vec<Item> = native::SHADER_NAMES
        .iter()
        .filter(|(n, ..)| (*n != "pic" || effect) && !(def.is_none() && matches!(*n, "uv" | "pic")))
        .map(|(n, sig, d)| {
            let callable = sig.starts_with(&format!("{n}(")) && !matches!(*n, "for" | "if");
            let (insert, cur) = match *n {
                "let" | "var" | "return" => (format!("{n} "), n.len() + 1),
                _ if callable => (format!("{n}()"), n.len() + 1),
                _ => (n.to_string(), n.len()),
            };
            let kind = if matches!(*n, "uv" | "pic" | "u") { Kind::Signal } else { Kind::Func };
            Item { label: n.to_string(), insert, cursor: cur, detail: d.to_string(), sig: sig.to_string(), kind }
        })
        .collect();
    if let Some(d) = def {
        for (n, default) in &d.params {
            let what = match native::param_kind(default.as_deref(), |x| is_picture_name(src, x)) {
                UArg::Num => "f32",
                UArg::Pal => "a palette: pal(name, t)",
                UArg::Pic => "a picture: name(uv)",
            };
            let insert = if what.starts_with("a picture") { format!("{n}(uv)") } else { n.clone() };
            let cur = insert.len();
            items.push(Item { label: n.clone(), insert, cursor: cur, detail: format!("parameter: {what}"), sig: d.signature(), kind: Kind::Var });
        }
    }
    // locals declared above, and helper functions from wgsl blocks
    let words_after = |text: &str, kw: &str| -> Vec<String> {
        text.match_indices(kw)
            .filter(|(i, _)| *i == 0 || !is_ident(text.as_bytes()[i - 1]))
            .filter_map(|(i, _)| {
                let rest = &text[i + kw.len()..];
                let n: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                (!n.is_empty()).then_some(n)
            })
            .collect()
    };
    for kw in ["let ", "var ", "const "] {
        for n in words_after(&src[block.start..line_start], kw) {
            items.push(Item { label: n.clone(), insert: n.clone(), cursor: n.len(), detail: "set above".into(), sig: String::new(), kind: Kind::Var });
        }
    }
    let defs_blocks: Vec<&Range<usize>> = defs.iter().map(|d| &d.body).collect();
    for r in blocks.iter().filter(|r| !defs_blocks.contains(r)) {
        for n in words_after(&src[r.clone()], "fn ") {
            let insert = format!("{n}()");
            items.push(Item { label: n.clone(), insert, cursor: n.len() + 1, detail: "your helper, from a wgsl block".into(), sig: String::new(), kind: Kind::Func });
        }
    }
    finish(no_double_brackets(rank(items, prefix, Some(Kind::Var), src), b, cursor), start..cursor, prefix)
}

/// Don't double the brackets when they're already there, and when a call follows,
/// offer only things that can be called.
fn no_double_brackets(mut items: Vec<Item>, b: &[u8], cursor: usize) -> Vec<Item> {
    if b.get(cursor) == Some(&b'(') {
        items.retain(|i| i.insert.contains('('));
        for i in &mut items {
            if let Some(p) = i.insert.find('(') {
                i.insert.truncate(p);
                i.cursor = i.insert.len();
            }
        }
    }
    items
}

fn finish(items: Vec<Item>, range: Range<usize>, prefix: &str) -> Option<Completion> {
    // accepting must add something: hide when the only match is already typed
    if items.is_empty() || (items.len() == 1 && items[0].label == prefix) {
        return None;
    }
    Some(Completion { range, items })
}

/// Apply `item` to `src`; returns the new cursor (byte offset).
pub fn accept(src: &mut String, c: &Completion, item: &Item) -> usize {
    src.replace_range(c.range.clone(), &item.insert);
    c.range.start + item.cursor
}

/// The call the cursor is inside: (name, which argument, chained after a dot).
pub fn call_at(src: &str, cursor: usize) -> Option<(String, usize, bool)> {
    let b = src.as_bytes();
    let cursor = cursor.min(src.len());
    let mut depth = 0i32;
    let mut commas = 0usize;
    let mut i = cursor;
    let mut in_str: Option<u8> = None;
    // Walk back, skipping strings on the way (quotes toggle).
    while i > 0 {
        i -= 1;
        let c = b[i];
        if let Some(q) = in_str {
            if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            b'"' | b'\'' | b'`' => in_str = Some(c),
            b')' => depth += 1,
            b'(' if depth > 0 => depth -= 1,
            b'(' => {
                let s = ident_start(b, i);
                if s == i {
                    return None;
                }
                let dotted = src[..s].trim_end().ends_with('.');
                return Some((src[s..i].to_string(), commas, dotted));
            }
            b',' if depth == 0 => commas += 1,
            _ => {}
        }
    }
    None
}

/// The signature hint for the call at the cursor.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub name: String,
    /// (argument, default as typed)
    pub args: Vec<(String, Option<String>)>,
    pub current: usize,
    pub doc: String,
    /// What the current argument takes.
    pub hint: String,
}

pub fn signature(src: &str, cursor: usize) -> Option<Signature> {
    if native::block_at(src, cursor).is_some_and(|r| cursor > r.start) {
        return None; // WGSL calls are yours
    }
    let (name, arg, dotted) = call_at(src, cursor)?;
    if let Some(d) = mine(src).into_iter().find(|d| d.name == name) {
        let header = src[..cursor].rsplit('\n').next().unwrap_or("").trim_start();
        if d.params.is_empty() || header.starts_with("picture ") || header.starts_with("effect ") {
            return None;
        }
        let current = arg.min(d.params.len() - 1);
        let hint = match native::param_kind(d.params[current].1.as_deref(), |n| is_picture_name(src, n)) {
            UArg::Num => "a number, pattern or signal",
            UArg::Pal => "a palette: type \" for the list",
            UArg::Pic => "a picture, like clouds()",
        };
        return Some(Signature { name, args: d.params.clone(), current, doc: d.summary(), hint: hint.into() });
    }
    let spec = if dotted { catalog::effect(&name) } else { catalog::find(&name).filter(|s| !s.hidden) };
    type Args = Vec<(String, Option<String>)>;
    let (args, doc, hints): (Args, String, Vec<String>) = if let Some(s) = spec {
        (
            s.args.iter().map(|(n, a)| (n.to_string(), catalog::default_text(a))).collect(),
            s.doc.to_string(),
            s.args.iter().map(|(_, a)| catalog::arg_hint(a)).collect(),
        )
    } else {
        let doc_of = |list: &[(&str, &str)]| list.iter().find(|f| f.0 == name).map(|f| split_sig(&name, f.1).1).unwrap_or_default();
        let num = |n: &[(&str, &str)]| -> Vec<(String, Option<String>)> { n.iter().map(|(a, d)| (a.to_string(), (!d.is_empty()).then(|| d.to_string()))).collect() };
        let a = match name.as_str() {
            "slider" => num(&[("value", ""), ("lo", "0"), ("hi", "1")]),
            "speed" => num(&[("cps", ".5")]),
            "view" => num(&[("x", ""), ("y", ""), ("zoom", "")]),
            "range" => num(&[("lo", ""), ("hi", "")]),
            "slow" | "fast" | "segment" => num(&[("n", "")]),
            "early" | "late" => num(&[("cycles", "")]),
            "add" | "sub" | "mul" | "div" => num(&[("value", "")]),
            "stack" => num(&[("pictures…", "")]),
            "blend" => {
                return Some(Signature {
                    name: name.clone(),
                    args: vec![("mode".into(), Some("\"normal\"".into()))],
                    current: 0,
                    doc: "how this layer mixes with the ones below".into(),
                    hint: format!("one of {}", catalog::BLENDS.join(", ")),
                });
            }
            _ => return None,
        };
        let doc = [doc_of(catalog::FUNCTIONS), doc_of(catalog::SIGNAL_METHODS)].into_iter().find(|d| !d.is_empty()).unwrap_or_default();
        let hint = if name == "stack" { "pictures, separated by commas".to_string() } else { "a number, pattern or signal".to_string() };
        let n = a.len();
        (a, doc, vec![hint; n])
    };
    if args.is_empty() {
        return None;
    }
    let current = arg.min(args.len() - 1);
    Some(Signature { name, hint: hints.get(current).cloned().unwrap_or_default(), args, current, doc })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(src: &str) -> Vec<String> {
        complete(src, src.len()).map(|c| c.items.into_iter().map(|i| i.label).collect()).unwrap_or_default()
    }

    fn has(src: &str, label: &str) -> bool {
        labels(src).iter().any(|l| l == label)
    }

    #[test]
    fn a_dot_lists_everything_that_fits() {
        let all = labels("mesh().");
        let n = catalog::effects().count() + 1; // + blend
        assert_eq!(all.len(), n, "every effect, nothing cut off");
        assert!(all.contains(&"blend".to_string()));
    }

    #[test]
    fn pictures_get_effects_and_signals_get_signal_methods() {
        assert!(!has("mesh().", "add"), "no signal maths on a picture");
        assert!(!has("mesh().", "range"));
        assert!(has("sine.", "range") && !has("sine.", "blur"));
        assert!(has("sine.range(0, 1).", "slow"), "along a signal chain");
        assert!(has("mesh()\n  .flow(.3)\n  .", "grain"), "along a picture chain, across lines");
        assert!(has("mesh(\"dawn\", sine.range(1, 2)).", "zoom"), "brackets inside a call don't confuse it");
        assert!(has("\"0 1\".", "slow"), "patterns are signals");
        assert_eq!(labels("mouse."), ["x", "y"]);
        assert!(has("mouse.x.", "range"));
        assert!(labels("stack(mesh(), clouds()).").is_empty(), "nothing chains onto a stack");
        assert!(labels("zoom(1.").is_empty(), "a decimal point isn't a dot");
    }

    #[test]
    fn a_comment_ending_in_a_full_stop_is_not_a_dot() {
        let s = "// plasma, typed out.\n\nplasma(\"dusk\", 1.2, .7)\n  .";
        assert!(has(s, "ascii") && !has(s, "range"), "{:?}", labels(s));
        let s = "mesh() // a comment.\n  .";
        assert!(has(s, "blur") && !has(s, "slow"));
        let s = "mesh(\"a // not a comment\").";
        assert!(has(s, "blur"));
    }

    #[test]
    fn names_before_a_bracket_must_be_callable() {
        // renaming `s(1)` in a number argument: only `slider` can take brackets there
        let c = complete("mesh().zoom(s(1))", 13).unwrap();
        let l: Vec<_> = c.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(l, ["slider"]);
        assert_eq!(c.items[0].insert, "slider", "the brackets are already there");
    }

    #[test]
    fn variables_know_what_they_hold() {
        assert!(has("w = sine.range(0, 1)\nmesh().zoom(w.", "slow"));
        assert!(!has("w = sine.range(0, 1)\nmesh().zoom(w.", "blur"));
        assert!(has("p = mesh()\n  .flow()\np.", "grain"));
        assert!(has("wob = saw\nmesh().zoom(wo", "wob"), "variables are offered as values");
    }

    #[test]
    fn quotes_offer_the_arguments_own_words() {
        assert_eq!(labels("mesh(\"").len(), catalog::PALETTES.len(), "a quote alone opens every palette");
        assert_eq!(labels("mesh(\"da")[0], "dawn");
        assert!(labels("mesh(\"dawn").is_empty(), "already complete");
        let mut masks = catalog::MASKS.to_vec();
        masks.sort();
        let mut got = labels("mesh().mask(\"");
        got.sort();
        assert_eq!(got, masks, "every shape (most used first)");
        assert!(has("mesh().blend(\"", "screen"));
        assert!(has("mesh().noise(.1, 1, 1, 0, \"", "uniform"));
        assert!(has("mesh().dither(\"", "riviera"), "dither takes a palette too");
        assert!(labels("mesh().zoom(\"").is_empty(), "number patterns get no palettes");
        assert!(labels("mesh().tint(\"").is_empty(), "colours are typed, not listed");
    }

    #[test]
    fn accepting_after_a_quote_closes_it() {
        let mut s = "mesh(\"da".to_string();
        let c = complete(&s, s.len()).unwrap();
        let at = accept(&mut s, &c, &c.items[0].clone());
        assert_eq!(s, "mesh(\"dawn\"");
        assert_eq!(at, s.len());
        // an existing closing quote is hopped over, not doubled
        let mut s = "mesh(\"da\")".to_string();
        let c = complete(&s, 8).unwrap();
        let at = accept(&mut s, &c, &c.items[0].clone());
        assert_eq!(s, "mesh(\"dawn\")");
        assert_eq!(&s[at..], ")");
        // palette lists keep going: "<dawn em…"
        let mut s = "mesh(\"<dawn em".to_string();
        let c = complete(&s, s.len()).unwrap();
        accept(&mut s, &c, &c.items[0].clone());
        assert_eq!(s, "mesh(\"<dawn ember");
    }

    #[test]
    fn unquoted_words_get_their_quotes() {
        let mut s = "mesh(da".to_string();
        let c = complete(&s, s.len()).unwrap();
        assert_eq!(c.items[0].label, "dawn");
        accept(&mut s, &c, &c.items[0].clone());
        assert_eq!(s, "mesh(\"dawn\"");
        assert_eq!(labels("mesh().mask(ci")[0], "circle");
    }

    #[test]
    fn line_starts_offer_pictures_not_signals() {
        assert_eq!(labels("me")[0], "mesh");
        assert!(!has("si", "sine"), "a line must be a picture");
        assert!(has("st", "stack"));
        assert_eq!(labels("mesh().zoom(si")[0], "sine");
        assert!(!has("mesh().zoom(me", "mesh"), "a number argument takes no picture");
        assert_eq!(labels("mesh().displace(cl")[0], "clouds", "picture arguments offer pictures");
        assert!(labels("").is_empty());
        assert!(labels("x = 0.5").is_empty(), "no completion on numbers");
        assert!(has("x = sa", "saw"));
    }

    #[test]
    fn fuzzy_and_by_description() {
        assert_eq!(labels("mesh().kld")[0], "kaleido");
        assert!(has("mesh().film", "grain"), "found by its description");
        assert_eq!(labels("mesh().gr")[0], "grain", "prefix matches first");
    }

    #[test]
    fn nothing_in_comments() {
        assert!(labels("// me").is_empty());
    }

    #[test]
    fn accepting_places_the_cursor_in_brackets() {
        let mut s = "mesh().fl".to_string();
        let c = complete(&s, s.len()).unwrap();
        let item = c.items[0].clone();
        let at = accept(&mut s, &c, &item);
        assert_eq!(s, "mesh().flow()");
        assert_eq!(&s[at..], ")");
        let mut s = "mesh().fl(.3)".to_string();
        let c = complete(&s, 9).unwrap();
        let item = c.items[0].clone();
        accept(&mut s, &c, &item);
        assert_eq!(s, "mesh().flow(.3)");
        // no arguments: the cursor lands after the brackets
        let mut s = "mesh().inv".to_string();
        let c = complete(&s, s.len()).unwrap();
        let at = accept(&mut s, &c, &c.items[0].clone());
        assert_eq!((s.as_str(), at), ("mesh().invert()", s.len()));
    }

    #[test]
    fn items_carry_signatures_with_defaults() {
        let c = complete("mesh().flo", 10).unwrap();
        assert!(c.items[0].sig.starts_with("flow(amount = "), "{}", c.items[0].sig);
    }

    #[test]
    fn signature_follows_the_argument() {
        let s = "mesh(\"a, b\").flow(.3, ";
        let sig = signature(s, s.len()).unwrap();
        assert_eq!((sig.name.as_str(), sig.current), ("flow", 1));
        assert_eq!(sig.args[1].0, "speed");
        assert!(sig.args[1].1.is_some(), "defaults shown");
        let s = "mesh(";
        let sig = signature(s, s.len()).unwrap();
        assert_eq!(sig.args[0].0, "palette");
        assert!(sig.hint.contains("palette"));
        assert!(signature("mesh()", 6).is_none());
        let s = "julia(\"x\", sine.range(1, ";
        assert_eq!(signature(s, s.len()).unwrap().args[1].0, "hi");
        let s = "mesh().mask(";
        assert!(signature(s, s.len()).unwrap().hint.contains("circle"));
    }

    const MINE: &str = "// soft rings
picture breathe(colors = \"dawn\", rings = 8, over = clouds()) {
    let d = length(asp(uv));
    return vec4f(pal(colors, fract(d * rings - u.time)), 1.0);
}
effect smear(amount = .02) {
    return pic(uv + amount);
}
wgsl {
    fn sd_box(p: vec2f) -> f32 { return length(p); }
}
";

    #[test]
    fn your_shaders_are_offered_like_built_ins() {
        let s = format!("{MINE}bre");
        assert_eq!(labels(&s)[0], "breathe", "a picture starts a line");
        let c = complete(&s, s.len()).unwrap();
        assert_eq!(c.items[0].detail, "soft rings", "its comment is its docs");
        assert!(c.items[0].sig.starts_with("breathe(colors = \"dawn\""));
        assert!(has(&format!("{MINE}mesh().sme"), "smear"), "effects chain");
        assert!(!has(&format!("{MINE}sme"), "smear"), "an effect can't start a line");
        assert!(has(&format!("{MINE}breathe().gr"), "grain"), "your picture takes effects");
        assert!(!has(&format!("{MINE}breathe().ran"), "range"));
        assert!(has(&format!("{MINE}mesh().displace(bre"), "breathe"), "and goes where pictures go");
        // its arguments know what they take
        assert_eq!(labels(&format!("{MINE}breathe(\"emb"))[0], "ember");
        assert!(has(&format!("{MINE}breathe(\"dawn\", si"), "sine"));
        assert!(has(&format!("{MINE}breathe(\"dawn\", 4, cl"), "clouds"));
        let s = format!("{MINE}breathe(\"dawn\", ");
        let sig = signature(&s, s.len()).unwrap();
        assert_eq!((sig.args[1].0.as_str(), sig.current), ("rings", 1));
        assert!(sig.hint.contains("number"));
        assert!(has("pi", "picture") && has("ef", "effect"), "writing one is discoverable");
    }

    #[test]
    fn shader_code_completes_wgsl() {
        let at = MINE.find("let d").unwrap() + "let d = length(asp(uv));\n    ".len();
        let s = |typed: &str| format!("{}{typed}", &MINE[..at]);
        assert_eq!(labels(&s("re"))[0], "return");
        assert_eq!(labels(&s("col"))[0], "colors", "parameters first");
        assert!(has(&s("ri"), "rings") && has(&s("pa"), "pal"));
        assert!(has(&s("fb"), "fbm") && has(&s("smo"), "smoothstep"));
        assert!(has(&s("d"), "d"), "locals above");
        // helpers from wgsl blocks, wherever they are in the sketch
        let typed = format!("{}sd{}", &MINE[..at], &MINE[at..]);
        let c = complete(&typed, at + 2).unwrap();
        assert_eq!(c.items[0].label, "sd_box");
        assert!(!has(&s("pi"), "pic"), "a picture has no pic()");
        assert_eq!(labels(&s("u.ti")), ["time"]);
        assert!(labels(&s("uv.x")).is_empty(), "swizzles are yours");
        assert!(labels(&s("// fb")).is_empty());
        let at = MINE.find("return pic").unwrap();
        assert!(has(&format!("{}pi", &MINE[..at]), "pic"), "an effect reads the picture so far");
        // the header names things: nothing to suggest
        assert!(labels("picture bre").is_empty());
        assert!(labels("effect smear(amo").is_empty());
        assert!(has("effect smear(amount = si", "sine"), "defaults are values");
        let typed = s("pal(");
        assert!(signature(&typed, typed.len()).is_none(), "WGSL calls get no monet signature");
    }

    #[test]
    fn noise_is_an_effect_and_clouds_a_picture() {
        assert!(has("mesh().noi", "noise"));
        assert!(!has("noi", "noise"), "not offered to start a line");
        assert!(has("mesh().displace(cl", "clouds"));
        let sig = signature("mesh().noise(", 13).unwrap();
        assert_eq!(sig.args[0].0, "amount");
        assert!(sig.doc.contains("Add Noise"));
        assert_eq!(signature("clouds(", 7).unwrap().args[0].0, "palette");
    }
}
