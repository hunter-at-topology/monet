//! The reference panel (⌘K): every picture, effect, signal, function, pattern
//! and palette, searchable, each with its one-line doc. Click to insert.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Key, Rect, RichText, Sense, Stroke, Ui, pos2,
    vec2,
};

use super::theme::*;
use super::{Insert, syntax_theme};
use crate::chain::{self, catalog};

/// What the panel asks of the app this frame.
pub enum Action {
    Insert(Insert),
    Close,
}

struct Entry {
    name: &'static str,
    sig: String,
    doc: String,
    color: Color32,
    insert: Insert,
}

struct Section {
    title: &'static str,
    hint: &'static str,
    entries: Vec<Entry>,
}

const CELL_MIN: f32 = 196.0;
const CELL_H: f32 = 42.0;
const GAP: f32 = 6.0;

fn sections() -> Vec<Section> {
    let t = syntax_theme();
    let spec_entry = |name: &'static str, s: &catalog::Spec, color, insert| Entry {
        name,
        sig: catalog::signature_as(name, s),
        doc: s.doc.to_string(),
        color,
        insert,
    };
    let effects = |pred: &dyn Fn(catalog::Role) -> bool| -> Vec<Entry> {
        catalog::effects()
            .filter(|(_, s)| pred(s.role))
            .map(|(n, s)| spec_entry(n, s, t.effect, Insert::Effect(n)))
            .collect()
    };
    let mut light = effects(&|r| matches!(r, catalog::Role::Tex(_)));
    light.push(Entry {
        name: "blend",
        sig: "blend(mode)".into(),
        doc: catalog::LAYER_METHODS[0].1.into(),
        color: t.effect,
        insert: Insert::Text(".blend(\"screen\")".into()),
    });
    let signals = catalog::SIGNALS
        .iter()
        .map(|(n, d)| {
            let head = n.split('.').next().unwrap_or(n);
            Entry {
                name: n,
                sig: n.to_string(),
                doc: d.to_string(),
                color: t.signal,
                insert: Insert::Text(if head == "mouse" {
                    n.to_string()
                } else {
                    head.to_string()
                }),
            }
        })
        .chain(catalog::SIGNAL_METHODS.iter().map(|(n, d)| {
            let (sig, doc) = split_sig(n, d);
            Entry {
                name: n,
                sig: format!(".{sig}"),
                doc,
                color: t.signal,
                insert: Insert::Text(format!(".{n}()")),
            }
        }))
        .collect();
    let functions = catalog::FUNCTIONS
        .iter()
        .map(|(n, d)| {
            let (sig, doc) = split_sig(n, d);
            Entry {
                name: n,
                sig,
                doc,
                color: t.func,
                insert: Insert::Text(match *n {
                    "slider" => "slider(0.5, 0, 1)".into(),
                    "speed" => "speed(1)\n".into(),
                    "view" => "view(0, 0, 1)\n".into(),
                    _ => format!("{n}()"),
                }),
            }
        })
        .collect();
    vec![
        Section {
            title: "Pictures",
            hint: "one per line, layered bottom to top",
            entries: catalog::SPECS
                .iter()
                .filter(|s| s.role == catalog::Role::Source && !s.hidden)
                .map(|s| spec_entry(s.name, s, t.source, Insert::Layer(s.name)))
                .collect(),
        },
        Section {
            title: "Shape and motion",
            hint: "chain after a picture",
            entries: effects(&|r| r == catalog::Role::Domain),
        },
        Section {
            title: "Colour and texture",
            hint: "chain after a picture",
            entries: effects(&|r| r == catalog::Role::Color),
        },
        Section {
            title: "Light and lens",
            hint: "each one adds a pass",
            entries: light,
        },
        Section {
            title: "Signals",
            hint: "0 to 1, anywhere a number goes",
            entries: signals,
        },
        Section {
            title: "Functions",
            hint: "",
            entries: functions,
        },
        Section {
            title: "Write your own",
            hint: "native WGSL, right in the sketch",
            entries: vec![
                Entry {
                    name: "picture",
                    sig: "picture name(amount = .5) { … }".into(),
                    doc: "return the colour at uv. Parameters take numbers, patterns and signals".into(),
                    color: t.func,
                    insert: Insert::Text(
                        "picture name(colors = \"dawn\", amount = .5) {\n    let d = length(asp(uv));\n    return vec4f(pal(colors, fract(d * 4.0 - u.time * amount)), 1.0);\n}\n".into(),
                    ),
                },
                Entry {
                    name: "effect",
                    sig: "effect name(amount = .5) { … }".into(),
                    doc: "chain it with . and read the picture so far with pic(p), at any position".into(),
                    color: t.func,
                    insert: Insert::Text(
                        "effect name(amount = .02) {\n    let c = pic(uv + vec2f(sin(uv.y * 40.0 + u.time), 0.0) * amount);\n    return c;\n}\n".into(),
                    ),
                },
                Entry {
                    name: "wgsl",
                    sig: "wgsl { fn helper(…) { … } }".into(),
                    doc: "helper functions every shader in the sketch can call".into(),
                    color: t.func,
                    insert: Insert::Text("wgsl {\n    fn helper(p: vec2f) -> f32 {\n        return length(p);\n    }\n}\n".into()),
                },
                Entry {
                    name: "defaults",
                    sig: "amount = .5 · colors = \"dawn\" · other = clouds()".into(),
                    doc: "a default sets the type: a number (f32), a palette (use pal), or a picture (call other(uv))".into(),
                    color: t.func,
                    insert: Insert::Text("other = clouds()".into()),
                },
            ],
        },
        Section {
            title: "Inside a shader",
            hint: "WGSL, plus monet's library",
            entries: chain::native::SHADER_NAMES
                .iter()
                .map(|(n, sig, doc)| Entry {
                    name: n,
                    sig: sig.to_string(),
                    doc: doc.to_string(),
                    color: if matches!(*n, "uv" | "pic" | "u") { t.signal } else { t.effect },
                    insert: Insert::Text(n.to_string()),
                })
                .chain(chain::native::UNIFORMS.iter().map(|(n, doc)| Entry {
                    name: n,
                    sig: format!("u.{n}"),
                    doc: doc.to_string(),
                    color: t.signal,
                    insert: Insert::Text(format!("u.{n}")),
                }))
                .collect(),
        },
    ]
}

/// "range(lo, hi): rescale…" → ("range(lo, hi)", "rescale…"): the grid shows the
/// name and the plain description, the tooltip the signature.
fn split_sig(name: &str, doc: &str) -> (String, String) {
    match doc.split_once("): ") {
        Some((sig, rest)) if sig.starts_with(&format!("{name}(")) => {
            (format!("{sig})"), rest.to_string())
        }
        _ => (format!("{name}()"), doc.to_string()),
    }
}

const PATTERNS: &[(&str, &str)] = &[
    (
        "\"0 .5 1\"",
        "a sequence: each step gets a slice of the cycle",
    ),
    ("\"<1 2 3>\"", "one step per cycle"),
    ("\"[0 1] 2\"", "nest steps inside one slice"),
    ("\"0*2\"", "play a step faster"),
    ("\"1@3 0\"", "give a step more weight"),
    ("\"<dawn ember>\"", "palettes too: they crossfade"),
];

fn matches(q: &str, hay: &[&str]) -> bool {
    q.is_empty() || hay.iter().any(|h| h.to_lowercase().contains(q))
}

pub fn show(ui: &mut Ui, query: &mut String, focus_search: &mut bool) -> Option<Action> {
    let mut action = None;
    ui.add_space(12.0);
    // header: title, search, close
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Reference")
                .font(FontId::proportional(14.0))
                .color(INK)
                .strong(),
        );
        if ui.available_width() > 470.0 {
            ui.add_space(10.0);
            ui.label(
                RichText::new("click any name to insert it")
                    .font(FontId::proportional(11.5))
                    .color(FAINT),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if super::text_button(ui, "Close", FAINT)
                .on_hover_text("esc")
                .clicked()
            {
                action = Some(Action::Close);
            }
            ui.add_space(12.0);
            let search = egui::Frame::new()
                .fill(PAPER)
                .stroke(Stroke::new(1.0, LINE))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(9, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(query)
                                .id(Id::unique("reference-search"))
                                .hint_text(RichText::new("Search names and docs").color(FAINT))
                                .font(FontId::proportional(12.5))
                                .text_color(INK)
                                .frame(egui::Frame::NONE)
                                .desired_width(150.0),
                        )
                    })
                    .inner
                });
            let r = search.inner;
            if std::mem::take(focus_search) {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let q = query.trim().to_lowercase();
                if let Some(e) = sections()
                    .into_iter()
                    .flat_map(|s| s.entries)
                    .find(|e| matches(&q, &[e.name, &e.doc]))
                {
                    action = Some(Action::Insert(e.insert));
                }
            }
        });
    });
    ui.add_space(2.0);

    let q = query.trim().to_lowercase();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let width = ui.available_width() - 4.0;
        let mut any = false;
        for s in sections() {
            let entries: Vec<_> = s.entries.into_iter().filter(|e| matches(&q, &[e.name, &e.doc])).collect();
            if entries.is_empty() {
                continue;
            }
            any = true;
            heading(ui, s.title, s.hint);
            if let Some(i) = grid(ui, width, &entries) {
                action = Some(Action::Insert(entries.into_iter().nth(i).expect("picked").insert));
            }
        }
        let patterns: Vec<_> = PATTERNS.iter().filter(|(c, d)| matches(&q, &[c, d, "pattern"])).collect();
        if !patterns.is_empty() {
            any = true;
            heading(ui, "Patterns", "any number can be a string of steps");
            for (code, doc) in patterns {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    let (rect, _) = ui.allocate_exact_size(vec2(150.0, 22.0), Sense::hover());
                    let g = ui.fonts_mut(|f| f.layout_job(chain_job(code)));
                    ui.painter().galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, INK);
                    ui.label(RichText::new(*doc).font(FontId::proportional(12.0)).color(MUTED));
                });
            }
        }
        let palettes: Vec<_> = catalog::PALETTES.iter().filter(|(n, _)| matches(&q, &[n, "palette", "colour", "color"])).collect();
        if !palettes.is_empty() {
            any = true;
            heading(ui, "Palettes", "a name, or your own: \"#f4a7b9 #a8c8f0\"");
            if let Some(name) = swatches(ui, width, &palettes) {
                action = Some(Action::Insert(Insert::Text(format!("\"{name}\""))));
            }
        }
        if !any {
            ui.add_space(18.0);
            ui.label(RichText::new(format!("Nothing matches “{}”", query.trim())).font(FontId::proportional(12.5)).color(MUTED));
        }
        ui.add_space(16.0);
        ui.label(RichText::new("⌥-drag a number to scrub it, with ⇧ for fine steps. The playing step of a pattern lights up.").font(FontId::proportional(11.5)).color(FAINT));
        ui.add_space(12.0);
    });
    action
}

fn chain_job(code: &str) -> egui::text::LayoutJob {
    super::highlight::layout(code, FontId::monospace(12.0), &syntax_theme(), &[], None)
}

fn heading(ui: &mut Ui, title: &str, hint: &str) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(
            RichText::new(title)
                .font(FontId::proportional(12.5))
                .color(INK)
                .strong(),
        );
        if !hint.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(hint)
                    .font(FontId::proportional(11.5))
                    .color(FAINT),
            );
        }
    });
    ui.add_space(6.0);
}

/// A responsive grid: each cell is the name over its one-line doc. Returns the clicked index.
fn grid(ui: &mut Ui, width: f32, entries: &[Entry]) -> Option<usize> {
    let cols = (((width + GAP) / (CELL_MIN + GAP)).floor() as usize).max(1);
    let cell_w = (width - GAP * (cols - 1) as f32) / cols as f32;
    let rows = entries.len().div_ceil(cols);
    let (area, _) = ui.allocate_exact_size(
        vec2(width, rows as f32 * CELL_H + (rows - 1) as f32 * 2.0),
        Sense::hover(),
    );
    let mut picked = None;
    for (i, e) in entries.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let rect = Rect::from_min_size(
            pos2(
                area.left() + c as f32 * (cell_w + GAP),
                area.top() + r as f32 * (CELL_H + 2.0),
            ),
            vec2(cell_w, CELL_H),
        );
        let resp = ui
            .interact(
                rect,
                Id::unique(("ref-cell", e.name, e.sig.as_str())),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(8), HOVER);
        }
        let inner = rect.shrink2(vec2(8.0, 0.0));
        p.text(
            pos2(inner.left(), rect.top() + 13.0),
            Align2::LEFT_CENTER,
            e.name,
            FontId::monospace(12.5),
            e.color,
        );
        let mut job = egui::text::LayoutJob::simple_singleline(
            e.doc.clone(),
            FontId::proportional(11.5),
            MUTED,
        );
        job.wrap = egui::text::TextWrapping {
            max_width: inner.width(),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let g = ui.fonts_mut(|f| f.layout_job(job));
        p.galley(pos2(inner.left(), rect.top() + 22.0), g, MUTED);
        let resp = resp.on_hover_ui(|ui| {
            ui.set_max_width(320.0);
            ui.label(
                RichText::new(&e.sig)
                    .font(FontId::monospace(12.0))
                    .color(e.color),
            );
            if !e.doc.is_empty() {
                ui.label(
                    RichText::new(&e.doc)
                        .font(FontId::proportional(12.0))
                        .color(MUTED),
                );
            }
        });
        if resp.clicked() {
            picked = Some(i);
        }
    }
    picked
}

/// Palettes as swatch strips with their names. Returns the clicked name.
fn swatches(
    ui: &mut Ui,
    width: f32,
    palettes: &[&(&'static str, &'static [&'static str])],
) -> Option<&'static str> {
    const MIN: f32 = 132.0;
    const H: f32 = 40.0;
    let cols = (((width + GAP) / (MIN + GAP)).floor() as usize).max(1);
    let cell_w = (width - GAP * (cols - 1) as f32) / cols as f32;
    let rows = palettes.len().div_ceil(cols);
    let (area, _) = ui.allocate_exact_size(vec2(width, rows as f32 * (H + 2.0)), Sense::hover());
    let mut picked = None;
    for (i, (name, cols_hex)) in palettes.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let rect = Rect::from_min_size(
            pos2(
                area.left() + c as f32 * (cell_w + GAP),
                area.top() + r as f32 * (H + 2.0),
            ),
            vec2(cell_w, H),
        );
        let resp = ui
            .interact(rect, Id::unique(("ref-pal", *name)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("\"{name}\""));
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(8), HOVER);
        }
        let strip = Rect::from_min_size(
            pos2(rect.left() + 8.0, rect.top() + 8.0),
            vec2(rect.width() - 16.0, 10.0),
        );
        let n = cols_hex.len();
        let w = strip.width() / n as f32;
        for (k, hex) in cols_hex.iter().enumerate() {
            let rgb = chain::mini::parse_hex(&hex[1..]).unwrap_or([0.0; 3]);
            let col = Color32::from_rgb(
                (rgb[0] * 255.0) as u8,
                (rgb[1] * 255.0) as u8,
                (rgb[2] * 255.0) as u8,
            );
            let rr = match (k == 0, k == n - 1) {
                (true, true) => CornerRadius::same(3),
                (true, false) => CornerRadius {
                    nw: 3,
                    sw: 3,
                    ne: 0,
                    se: 0,
                },
                (false, true) => CornerRadius {
                    ne: 3,
                    se: 3,
                    nw: 0,
                    sw: 0,
                },
                _ => CornerRadius::ZERO,
            };
            p.rect_filled(
                Rect::from_min_size(
                    pos2(strip.left() + w * k as f32, strip.top()),
                    vec2(w + 0.5, strip.height()),
                ),
                rr,
                col,
            );
        }
        p.rect_stroke(
            strip,
            CornerRadius::same(3),
            Stroke::new(1.0, Color32::from_black_alpha(14)),
            egui::StrokeKind::Inside,
        );
        p.text(
            pos2(strip.left(), rect.top() + 29.0),
            Align2::LEFT_CENTER,
            *name,
            FontId::monospace(11.5),
            MUTED,
        );
        if resp.clicked() {
            picked = Some(*name);
        }
    }
    picked
}
