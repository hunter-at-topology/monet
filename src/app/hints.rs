//! The editor's floating hints: the completion list (scrolling, with the
//! selected item's full signature and description underneath) and the
//! signature card shown while you type an argument.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, RichText, Sense, Stroke, TextFormat, Ui, pos2, vec2};

use super::complete::{self, Completion, Kind, Signature};
use super::theme::*;
use super::{card, syntax_theme};
use crate::chain::{self, catalog};

const WIDTH: f32 = 380.0;
const ROW: f32 = 24.0;
const VISIBLE: usize = 8;

pub fn kind_color(k: Kind) -> Color32 {
    let t = syntax_theme();
    match k {
        Kind::Source => t.source,
        Kind::Effect => t.effect,
        Kind::Signal => t.signal,
        Kind::Func => t.func,
        Kind::Palette | Kind::Word => t.word,
        Kind::Var => INK,
    }
}

/// A palette as a strip of its colours.
pub fn swatch(p: &egui::Painter, rect: Rect, cols: &[&str]) {
    let n = cols.len().max(1);
    let w = rect.width() / n as f32;
    for (k, hex) in cols.iter().enumerate() {
        let rgb = chain::mini::parse_hex(hex.trim_start_matches('#')).unwrap_or([0.0; 3]);
        let col = Color32::from_rgb((rgb[0] * 255.0) as u8, (rgb[1] * 255.0) as u8, (rgb[2] * 255.0) as u8);
        let rr = match (k == 0, k == n - 1) {
            (true, true) => CornerRadius::same(3),
            (true, false) => CornerRadius { nw: 3, sw: 3, ne: 0, se: 0 },
            (false, true) => CornerRadius { ne: 3, se: 3, nw: 0, sw: 0 },
            _ => CornerRadius::ZERO,
        };
        p.rect_filled(Rect::from_min_size(pos2(rect.left() + w * k as f32, rect.top()), vec2(w + 0.5, rect.height())), rr, col);
    }
    p.rect_stroke(rect, CornerRadius::same(3), Stroke::new(1.0, Color32::from_black_alpha(16)), egui::StrokeKind::Inside);
}

fn one_line(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.fonts_mut(|f| f.layout_job(job))
}

/// The completion list under the caret. Returns a clicked row.
pub fn completion(ctx: &egui::Context, at: Pos2, c: &Completion, sel: usize) -> Option<usize> {
    let mut clicked = None;
    // follow the selection only when it moves, so the wheel scrolls freely
    let last: Option<usize> = ctx.data(|d| d.get_temp(Id::unique("complete-last-sel")));
    let follow = last != Some(sel);
    ctx.data_mut(|d| d.insert_temp(Id::unique("complete-last-sel"), sel));
    egui::Area::new(Id::unique("complete"))
        .order(egui::Order::Foreground)
        .fade_in(false)
        .constrain(true)
        .fixed_pos(at)
        .show(ctx, |ui| {
            card().fill(PAPER).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                ui.set_width(WIDTH);
                let rows = c.items.len().min(VISIBLE) as f32;
                egui::ScrollArea::vertical()
                    .id_salt(("complete-list", c.range.start, c.items.len(), c.items.first().map(|i| i.label.clone())))
                    .max_height(rows * ROW)
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (k, item) in c.items.iter().enumerate() {
                            let w = ui.available_width();
                            let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW), Sense::click());
                            let p = ui.painter();
                            if k == sel {
                                p.rect_filled(rect, 6.0, HOVER);
                                p.rect_filled(Rect::from_min_size(rect.left_top() + vec2(0.0, 6.0), vec2(2.0, ROW - 12.0)), 1.0, ACCENT);
                                if follow {
                                    ui.scroll_to_rect(rect, None);
                                }
                            } else if resp.hovered() {
                                p.rect_filled(rect, 6.0, HOVER.gamma_multiply(0.6));
                            }
                            let name = one_line(ui, &item.label, FontId::monospace(12.5), kind_color(item.kind), 150.0);
                            let name_w = name.size().x;
                            p.galley(pos2(rect.left() + 10.0, rect.center().y - name.size().y / 2.0), name, INK);
                            let right = rect.right() - 8.0;
                            let left = rect.left() + 10.0 + name_w.max(96.0) + 12.0;
                            if item.kind == Kind::Palette {
                                let cols: Vec<&str> = item.sig.split(' ').collect();
                                swatch(p, Rect::from_min_max(pos2(right - 84.0, rect.center().y - 5.0), pos2(right, rect.center().y + 5.0)), &cols);
                            } else {
                                let g = one_line(ui, &item.detail, FontId::proportional(11.5), FAINT, (right - left).max(20.0));
                                p.galley(pos2(right - g.size().x, rect.center().y - g.size().y / 2.0), g, FAINT);
                            }
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                clicked = Some(k);
                            }
                        }
                    });
                // the selected item in full
                if let Some(item) = c.items.get(sel) {
                    ui.add_space(4.0);
                    let (line, _) = ui.allocate_exact_size(vec2(WIDTH, 1.0), Sense::hover());
                    ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, LINE));
                    ui.add_space(5.0);
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 0)).show(ui, |ui| {
                        ui.set_max_width(WIDTH - 16.0);
                        if item.kind == Kind::Palette {
                            ui.label(RichText::new(format!("\"{}\"", item.label)).font(FontId::monospace(11.5)).color(kind_color(item.kind)));
                            let (r, _) = ui.allocate_exact_size(vec2(WIDTH - 16.0, 12.0), Sense::hover());
                            let cols: Vec<&str> = item.sig.split(' ').collect();
                            swatch(ui.painter(), r, &cols);
                            ui.label(RichText::new(&item.sig).font(FontId::monospace(10.5)).color(FAINT));
                        } else {
                            if !item.sig.is_empty() {
                                ui.label(RichText::new(&item.sig).font(FontId::monospace(11.5)).color(kind_color(item.kind)));
                            }
                            ui.label(RichText::new(&item.detail).font(FontId::proportional(12.0)).color(MUTED));
                        }
                        ui.add_space(2.0);
                        ui.label(RichText::new("↑↓ choose · tab or return to insert · esc to close").font(FontId::proportional(10.5)).color(FAINT));
                    });
                    ui.add_space(2.0);
                }
            });
        });
    clicked
}

/// The signature card above the caret: every argument with its default,
/// the one you're on in ink, and what it takes.
pub fn signature(ctx: &egui::Context, at: Pos2, s: &Signature) {
    let mut job = egui::text::LayoutJob::default();
    let f = FontId::monospace(12.0);
    let muted = TextFormat::simple(f.clone(), MUTED);
    let faint = TextFormat::simple(f.clone(), FAINT);
    job.append(&format!("{}(", s.name), 0.0, muted.clone());
    for (k, (name, default)) in s.args.iter().enumerate() {
        if k > 0 {
            job.append(", ", 0.0, muted.clone());
        }
        if k == s.current {
            job.append(name, 0.0, TextFormat { underline: Stroke::new(1.0, ACCENT), ..TextFormat::simple(f.clone(), INK) });
        } else {
            job.append(name, 0.0, muted.clone());
        }
        if let Some(d) = default {
            job.append(&format!(" {d}"), 0.0, faint.clone());
        }
    }
    job.append(")", 0.0, muted);
    let hint = s.args.get(s.current).map(|(n, _)| format!("{n}: {}", s.hint)).filter(|_| !s.hint.is_empty());
    egui::Area::new(Id::unique("signature"))
        .order(egui::Order::Foreground)
        .fade_in(false)
        .constrain(true)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(at)
        .interactable(false)
        .show(ctx, |ui| {
            card().fill(PAPER).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                ui.set_max_width(460.0);
                ui.label(job);
                if let Some(h) = &hint {
                    ui.label(RichText::new(h).font(FontId::proportional(11.5)).color(INK.gamma_multiply(0.85)));
                }
                if !s.doc.is_empty() {
                    ui.label(RichText::new(&s.doc).font(FontId::proportional(11.5)).color(FAINT));
                }
            });
        });
}

/// Hover docs for the name under byte `b`: (signature, description, palette colours).
pub fn docs_at(src: &str, b: usize) -> Option<(String, String, Option<&'static [&'static str]>)> {
    use super::highlight::Kind as K;
    let (r, kind) = super::highlight::tokens(src).into_iter().find(|(r, _)| r.start <= b && b < r.end)?;
    let name = &src[r.clone()];
    let dotted = src[..r.start].trim_end().ends_with('.');
    let (defs, _) = crate::chain::native::outline(src);
    // inside a shader: WGSL, the library, the frame, this shader's parameters
    if let Some(block) = crate::chain::native::block_at(src, b) {
        if src[..r.start].ends_with("u.") {
            let d = crate::chain::native::UNIFORMS.iter().find(|u| u.0 == name)?;
            return Some((format!("u.{name}"), d.1.to_string(), None));
        }
        if dotted {
            return None;
        }
        if let Some(d) = defs.iter().find(|d| d.body == block)
            && let Some((p, default)) = d.params.iter().find(|p| p.0 == name)
        {
            let what = default.as_ref().map_or_else(|| "a number (no default: 0)".to_string(), |v| format!("default {v}"));
            return Some((format!("{p} · parameter of {}", d.name), what, None));
        }
        let n = crate::chain::native::SHADER_NAMES.iter().find(|n| n.0 == name)?;
        return Some((n.1.to_string(), n.2.to_string(), None));
    }
    if let Some(d) = defs.iter().find(|d| d.name == name) {
        return Some((d.signature(), d.summary(), None));
    }
    match kind {
        K::Source | K::Effect => {
            if name == "blend" {
                return Some(("blend(mode = \"normal\")".into(), format!("how this layer mixes: {}", catalog::BLENDS.join(", ")), None));
            }
            let s = if dotted { catalog::effect(name) } else { catalog::find(name).filter(|s| !s.hidden) }?;
            Some((catalog::signature_defaults(name, s), s.doc.to_string(), None))
        }
        K::Signal => {
            let (sig, d) = catalog::SIGNALS
                .iter()
                .find(|s| s.0.split('.').next() == Some(name) || s.0 == name)
                .map(|s| (name.to_string(), s.1.to_string()))
                .or_else(|| catalog::SIGNAL_METHODS.iter().find(|s| s.0 == name).map(|s| complete::split_sig(name, s.1)))?;
            Some((sig, d, None))
        }
        K::Func => catalog::FUNCTIONS.iter().find(|f| f.0 == name).map(|f| {
            let (sig, d) = complete::split_sig(name, f.1);
            (sig, d, None)
        }),
        K::Word => {
            if let Some(cols) = catalog::palette(name) {
                return Some((format!("\"{name}\""), format!("palette of {} colours", cols.len()), Some(cols)));
            }
            // a word argument: say what else it could be
            // look the call up from the opening quote, not from inside the string
            let quote = src[..r.start].rfind(['"', '\'', '`'])?;
            let s = complete::signature(src, quote)?;
            let doc = catalog::word_doc(name).map_or(s.hint.clone(), |d| format!("{d}. {}", s.hint.replacen("one of", "One of", 1)));
            Some((format!("\"{name}\""), doc, None))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_docs() {
        let s = "mesh(\"dawn\").flow(sine)";
        assert!(docs_at(s, 1).unwrap().0.starts_with("mesh(palette = \""), "signatures show defaults");
        let dawn = docs_at(s, 7).unwrap();
        assert!(dawn.1.starts_with("palette") && dawn.2.is_some(), "palettes show their colours");
        assert!(docs_at(s, 14).unwrap().0.starts_with("flow(amount = "));
        assert_eq!(docs_at(s, 20).unwrap().0, "sine");
        assert!(docs_at(s, 4).is_none());
    }

    #[test]
    fn hover_knows_your_shaders() {
        let s = "// breathing rings\npicture breathe(colors = \"dawn\", k = 2) {\n    return vec4f(pal(colors, fract(length(uv) * k + u.time)), 1.0);\n}\nbreathe().blur()";
        let at = |w: &str| s.rfind(w).unwrap() + 1;
        let (sig, doc, _) = docs_at(s, at("breathe()")).unwrap();
        assert_eq!(sig, "breathe(colors = \"dawn\", k = 2)");
        assert_eq!(doc, "breathing rings");
        assert!(docs_at(s, at("pal(")).unwrap().1.contains("palette"));
        assert!(docs_at(s, at("time")).unwrap().1.contains("seconds"));
        assert_eq!(docs_at(s, s.find("* k").unwrap() + 2).unwrap().1, "default 2");
        assert!(docs_at(s, at("fract")).unwrap().1.contains("decimal"));
    }

    #[test]
    fn hover_knows_chained_noise_and_word_arguments() {
        let s = "mesh().noise(.2)";
        assert!(docs_at(s, 8).unwrap().1.contains("Add Noise"), "the effect, not the picture");
        assert!(docs_at("clouds()", 1).unwrap().1.contains("Clouds"));
        let s = "mesh().mask(\"ring\")";
        assert!(docs_at(s, 14).unwrap().1.contains("circle"), "a word says what else it could be");
        let s = "mesh().blend(\"screen\")";
        assert!(docs_at(s, 9).unwrap().1.contains("multiply"));
    }
}
