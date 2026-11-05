//! The window: a code pane on the left, the living canvas on the right.
//!
//! The editor *is* the REPL: every keystroke re-evaluates the
//! sketch. Values (numbers, patterns, palettes, sliders, scrubbing) apply in
//! the same frame because they're uniforms; only a change of structure (which
//! pictures and effects) compiles shaders, on a background thread, while the
//! last good picture keeps playing.

mod complete;
#[cfg(test)]
mod complete_qa;
mod editing;
mod highlight;
mod hints;
mod reference;
mod theme;

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use web_time::Instant;

use eframe::egui::{
    self, Align2, CornerRadius, FontId, Id, Key, KeyboardShortcut, Modifiers, Pos2, Rect, RichText,
    Sense, Stroke, Ui, Vec2, pos2, vec2,
};
use eframe::egui_wgpu::wgpu;

use crate::chain::{self, Program};
use crate::engine::{Compiler, Diag, Engine, Uniforms};
use crate::presets;
use theme::*;

/// The editor in a browser tab, on `canvas` (see `crate::web`).
#[cfg(target_arch = "wasm32")]
pub async fn run_web(canvas: web_sys::HtmlCanvasElement) -> Result<(), String> {
    let shared = crate::web::shared_sketch();
    let mut options = eframe::WebOptions::default();
    // `?gl` forces WebGL2 (to try the fallback); otherwise WebGPU where the browser has it
    if crate::web::query_has("gl") {
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        setup.instance_descriptor.backends = wgpu::Backends::GL;
        options.wgpu_options.wgpu_setup = eframe::egui_wgpu::WgpuSetup::CreateNew(setup);
    }
    eframe::WebRunner::new()
        .start(
            canvas,
            options,
            Box::new(move |cc| {
                let file = shared.map(|src| (PathBuf::from("shared.monet"), src));
                Ok(Box::new(App::new(cc, file)?))
            }),
        )
        .await
        .map_err(|e| format!("{e:?}"))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run(path: Option<PathBuf>) -> Result<(), String> {
    let file = match &path {
        Some(p) => Some((
            p.clone(),
            std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?,
        )),
        None => None,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("monet")
            .with_inner_size([1480.0, 920.0])
            .with_min_inner_size([720.0, 420.0])
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "monet",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, file)?))),
    )
    .map_err(|e| e.to_string())
}

// ── documents ────────────────────────────────────────────────────────────────

struct Doc {
    name: String,
    src: String,
    /// Built-in source (presets) for "revert".
    original: Option<&'static str>,
    path: Option<PathBuf>,
    saved: Option<String>,
    mtime: Option<SystemTime>,
    /// centre x, centre y, zoom — for pannable (fractal) sketches
    view: [f32; 3],
    home: Option<[f32; 3]>,
}

impl Doc {
    fn dirty(&self) -> bool {
        match (&self.saved, self.original) {
            (Some(s), _) => *s != self.src,
            (None, Some(o)) => o != self.src,
            _ => true,
        }
    }

    fn dir(&self) -> PathBuf {
        self.path
            .as_ref()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
    }
}

/// An ⌥-drag on a number in the editor.
struct Scrub {
    range: Range<usize>,
    orig: String,
    value: f64,
}

// ── app ──────────────────────────────────────────────────────────────────────

struct App {
    engine: Engine,
    compiler: Compiler,
    texture: Option<egui::TextureId>,

    docs: Vec<Doc>,
    active: usize,

    /// The program on screen, and the source it came from.
    program: Option<(Program, String)>,
    /// A program waiting for its pipelines (generation, program, source).
    pending: Option<(u64, Program, String)>,
    /// The graph most recently sent to the compiler.
    submitted: Option<chain::Graph>,
    diag: Option<Diag>,
    eval_ms: f32,
    pipe_ms: f32,
    /// Pattern steps playing right now (byte spans into the program's source).
    playing_steps: Vec<Range<usize>>,

    // clock
    cycle: f64,
    seconds: f64,
    playing: bool,
    last_tick: Instant,
    frame: u64,
    fps: f32,

    // canvas
    scale: u32,
    mouse: [f32; 2],
    focus: bool,
    last_pointer_move: Instant,
    params_open: bool,

    // ui
    menu_open: bool,
    ref_open: bool,
    /// The reference's search box, and whether to focus it (on ⌘K).
    ref_query: String,
    ref_focus: bool,
    toast: Option<(String, Instant)>,
    last_file_poll: Instant,
    goto: Option<usize>,
    cursor: usize,
    scrub: Option<Scrub>,
    /// Open autocomplete: suggestions, selected row, and the cursor (bytes) it was made for.
    completion: Option<(complete::Completion, usize, usize)>,
    /// Did the last frame actually draw? (Still pictures skip unchanged frames.)
    drew: bool,
    /// Demo hook: focus the editor with the caret at the end on the first frame.
    focus_end: bool,
    /// Reset the UI zoom on the first frame: once, to undo a stale saved zoom.
    reset_zoom: bool,
    /// `MONET_SHOT=path.png`: capture the window after warm-up, then quit.
    shot: Option<PathBuf>,
}

const EDITOR_ID: &str = "monet-editor";
/// The sketch a first visit opens on.
const DEFAULT_SKETCH: &str = "kali";
/// Below this width (points), the editor goes under the picture instead of beside it.
const NARROW: f32 = 720.0;
/// Marks that the one-time reset of the saved UI zoom has run (a promo recording
/// once left 1.5× saved); ⌘+ / ⌘− / ⌘0 zoom choices after it are kept.
const ZOOM_RESET: &str = "ui/zoom-reset-1";
/// Storage namespace; bump when the stored format or preset language changes.
const STORE: &str = "v4";

impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        file: Option<(PathBuf, String)>,
    ) -> Result<Self, String> {
        let rs = cc
            .wgpu_render_state
            .as_ref()
            .ok_or("monet needs the wgpu renderer")?;
        #[cfg(target_arch = "wasm32")]
        {
            let info = rs.adapter.get_info();
            web_sys::console::info_1(&format!("monet: drawing with {:?} on {}", info.backend, info.name).into());
        }
        let engine = Engine::new(rs.device.clone(), rs.queue.clone());
        let ctx = cc.egui_ctx.clone();
        let compiler = Compiler::spawn(engine.factory(), Arc::new(move || ctx.request_repaint()));
        theme::install(&cc.egui_ctx);

        let storage = cc.storage;
        let get = |k: &str| storage.and_then(|s| s.get_string(k));

        let mut docs: Vec<Doc> = Vec::new();
        if let Some((path, src)) = file {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("sketch")
                .to_string();
            let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            docs.push(Doc {
                name,
                saved: Some(src.clone()),
                src,
                original: None,
                path: Some(path),
                mtime,
                view: [0.0, 0.0, 1.0],
                home: None,
            });
        }
        for p in presets::PRESETS {
            let src = get(&format!("{STORE}/doc/{}", p.name))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| p.source.to_string());
            docs.push(Doc {
                name: p.name.to_string(),
                src,
                original: Some(p.source),
                path: None,
                saved: None,
                mtime: None,
                view: [0.0, 0.0, 1.0],
                home: None,
            });
        }
        let active = if docs[0].path.is_some() {
            0
        } else {
            get(&format!("{STORE}/active"))
                .and_then(|n| docs.iter().position(|d| d.name == n))
                // first visit: open on kali
                .or_else(|| docs.iter().position(|d| d.name == DEFAULT_SKETCH))
                .unwrap_or(0)
        };

        let mut app = Self {
            engine,
            compiler,
            texture: None,
            docs,
            active,
            program: None,
            pending: None,
            submitted: None,
            diag: None,
            eval_ms: 0.0,
            pipe_ms: 0.0,
            playing_steps: Vec::new(),
            cycle: 0.0,
            seconds: 0.0,
            playing: true,
            last_tick: Instant::now(),
            frame: 0,
            fps: 60.0,
            scale: 1,
            mouse: [0.5, 0.5],
            focus: std::env::var_os("MONET_FOCUS").is_some(),
            last_pointer_move: Instant::now(),
            params_open: true,
            menu_open: false,
            ref_open: std::env::var_os("MONET_REF").is_some(),
            ref_query: std::env::var("MONET_REF")
                .ok()
                .filter(|q| q != "1")
                .unwrap_or_default(),
            ref_focus: false,
            toast: None,
            last_file_poll: Instant::now(),
            goto: None,
            cursor: 0,
            scrub: None,
            completion: None,
            drew: true,
            focus_end: false,
            reset_zoom: get(ZOOM_RESET).is_none(),
            shot: std::env::var_os("MONET_SHOT").map(PathBuf::from),
        };
        // MONET_SKETCH=name opens a specific sketch (scripting, demos).
        let active = std::env::var("MONET_SKETCH")
            .ok()
            .and_then(|n| app.docs.iter().position(|d| d.name == n))
            .unwrap_or(active);
        app.switch_to(active);
        // MONET_TYPE="…" appends text to the sketch, as if typed (demos, screenshots).
        if let Ok(extra) = std::env::var("MONET_TYPE") {
            app.doc_mut().src.push_str(&extra.replace("\\n", "\n"));
            app.evaluate();
            let src = app.doc().src.clone();
            app.completion = complete::complete(&src, src.len()).map(|c| (c, 0, src.len()));
            app.focus_end = true;
        }
        Ok(app)
    }

    fn doc(&self) -> &Doc {
        &self.docs[self.active]
    }

    fn doc_mut(&mut self) -> &mut Doc {
        &mut self.docs[self.active]
    }

    fn switch_to(&mut self, i: usize) {
        self.active = i;
        self.diag = None;
        self.scrub = None;
        self.engine.reset_history();
        self.evaluate();
    }

    /// Re-evaluate the active sketch. Microseconds; never blocks on the GPU.
    fn evaluate(&mut self) {
        let src = self.doc().src.clone();
        let t = Instant::now();
        let result = chain::compile(&src);
        self.eval_ms = t.elapsed().as_secs_f32() * 1000.0;
        let p = match result {
            Ok(p) => p,
            Err(e) => {
                self.diag = Some(e.into());
                return;
            }
        };
        self.diag = None;
        let doc = &mut self.docs[self.active];
        if p.home != doc.home {
            doc.home = p.home;
            doc.view = p.home.unwrap_or([0.0, 0.0, 1.0]);
        }
        if self.engine.graph() == Some(&p.graph) {
            // Same shaders: swap values in this very frame.
            self.program = Some((p, src));
            self.pending = None;
            self.submitted = None;
        } else if self.submitted.as_ref() == Some(&p.graph)
            && let Some((generation, ..)) = self.pending.take()
        {
            self.pending = Some((generation, p, src));
        } else {
            let dir = self.doc().dir();
            let generation = self.compiler.submit(&p.graph, &dir);
            self.submitted = Some(p.graph.clone());
            self.pending = Some((generation, p, src));
        }
    }

    fn poll_compiler(&mut self) {
        let Some(done) = self.compiler.poll() else {
            return;
        };
        let Some((generation, ..)) = &self.pending else {
            return;
        };
        if done.generation != *generation {
            return;
        }
        let (_, program, src) = self.pending.take().expect("pending");
        self.pipe_ms = done.millis;
        match done.result {
            Ok(g) => {
                self.engine.set_graph(g);
                self.program = Some((program, src));
            }
            Err(d) => {
                self.diag = Some(d);
                self.submitted = None;
            }
        }
    }

    fn compiling(&self) -> bool {
        self.pending.is_some()
    }

    /// Hot reload: pick up edits made to the backing file by another editor.
    fn poll_file(&mut self) {
        if self.last_file_poll.elapsed().as_millis() < 250 {
            return;
        }
        self.last_file_poll = Instant::now();
        let doc = &self.docs[self.active];
        let Some(path) = doc.path.clone() else { return };
        let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
            return;
        };
        if Some(mtime) == doc.mtime {
            return;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let doc = self.doc_mut();
        doc.mtime = Some(mtime);
        if text != doc.src {
            doc.src = text.clone();
            doc.saved = Some(text);
            self.evaluate();
            self.toast("reloaded from disk");
        }
    }

    fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), Instant::now()));
    }

    /// ⌘S in the browser: sketches already keep themselves in this browser, so
    /// saving downloads the file.
    #[cfg(target_arch = "wasm32")]
    fn save(&mut self) {
        let doc = &self.docs[self.active];
        let name = format!("{}.monet", doc.name);
        let src = doc.src.clone();
        match crate::web::download(&name, src.as_bytes(), "text/plain") {
            Ok(()) => {
                self.doc_mut().saved = Some(src);
                self.toast(format!("downloaded {name}"));
            }
            Err(e) => self.toast(format!("save failed: {e}")),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save(&mut self) {
        let doc = &self.docs[self.active];
        let path = doc.path.clone().unwrap_or_else(|| {
            let cwd = std::env::current_dir().unwrap_or_default();
            let mut p = cwd.join(format!("{}.monet", doc.name));
            let mut n = 2;
            while p.exists() {
                p = cwd.join(format!("{}-{n}.monet", doc.name));
                n += 1;
            }
            p
        });
        match std::fs::write(&path, &doc.src) {
            Ok(()) => {
                let src = doc.src.clone();
                let doc = self.doc_mut();
                doc.saved = Some(src);
                doc.mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                doc.path = Some(path.clone());
                self.toast(format!("saved {}", short_path(&path)));
            }
            Err(e) => self.toast(format!("save failed: {e}")),
        }
    }

    /// ⌘E in the browser: read the canvas back (asynchronously) and download it.
    #[cfg(target_arch = "wasm32")]
    fn export(&mut self) {
        let name = format!("monet-{}.png", self.doc().name);
        let scale = self.scale;
        let started = self.engine.read_target_then(move |img| {
            let _ = crate::web::download(&name, &img.upscale(scale).png_bytes(), "image/png");
        });
        if started {
            self.toast("exporting png");
        }
    }

    /// Copy a link that opens this sketch, for sharing.
    #[cfg(target_arch = "wasm32")]
    fn share(&mut self, ctx: &egui::Context) {
        let url = crate::web::share_url(&self.doc().src);
        ctx.copy_text(url);
        self.toast("link copied: it opens this sketch");
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn export(&mut self) {
        let Some(img) = self.engine.read_target() else {
            return;
        };
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let path = std::env::current_dir()
            .unwrap_or_default()
            .join(format!("monet-{}-{stamp}.png", self.doc().name));
        match img.upscale(self.scale).save_png(&path) {
            Ok(()) => self.toast(format!("exported {}", short_path(&path))),
            Err(e) => self.toast(format!("export failed: {e}")),
        }
    }

    fn uses_view(&self) -> bool {
        self.program.as_ref().is_some_and(|(p, _)| p.uses_view())
    }

    fn restart(&mut self) {
        self.cycle = 0.0;
        self.seconds = 0.0;
        self.frame = 0;
        self.engine.reset_history();
    }

    /// The live program's sliders, when they still point into the current text.
    fn sliders(&self) -> Vec<chain::Slider> {
        match &self.program {
            Some((p, src)) if *src == self.doc().src => p.sliders.clone(),
            _ => Vec::new(),
        }
    }

    // ── input ────────────────────────────────────────────────────────────────

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
        let hit = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if hit(cmd(Key::Enter)) {
            self.focus = !self.focus;
        }
        if self.completion.is_none() && hit(KeyboardShortcut::new(Modifiers::NONE, Key::Escape)) {
            if self.ref_open {
                self.ref_open = false;
            } else if self.focus {
                self.focus = false;
            }
        }
        if hit(cmd(Key::S)) {
            self.save();
        }
        if hit(cmd(Key::K)) {
            self.ref_open = !self.ref_open;
            self.ref_focus = self.ref_open;
        }
        if hit(cmd(Key::E)) {
            self.export();
        }
        #[cfg(target_arch = "wasm32")]
        if hit(KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::C)) {
            self.share(ctx);
        }
        if hit(cmd(Key::Period))
            || (self.focus && hit(KeyboardShortcut::new(Modifiers::NONE, Key::Space)))
        {
            self.playing = !self.playing;
        }
        if hit(cmd(Key::R)) {
            self.restart();
        }
        if hit(cmd(Key::Num0)) {
            let d = self.doc_mut();
            d.view = d.home.unwrap_or([0.0, 0.0, 1.0]);
        }
        for (k, s) in [
            (Key::Num1, 1),
            (Key::Num2, 2),
            (Key::Num3, 4),
            (Key::Num4, 8),
        ] {
            if hit(cmd(k)) {
                self.scale = s;
            }
        }
        if hit(cmd(Key::OpenBracket)) {
            self.switch_to((self.active + self.docs.len() - 1) % self.docs.len());
        }
        if hit(cmd(Key::CloseBracket)) {
            self.switch_to((self.active + 1) % self.docs.len());
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64().min(0.25);
        self.last_tick = now;
        if self.playing {
            let speed = self.program.as_ref().map_or(0.5, |(p, _)| p.speed);
            self.seconds += dt;
            self.cycle += dt * speed;
            self.frame += 1;
            if dt > 0.0 {
                self.fps += ((1.0 / dt) as f32 - self.fps) * 0.08;
            }
        }
    }

    // ── editor pane ──────────────────────────────────────────────────────────

    fn editor_pane(&mut self, ui: &mut Ui) {
        // room for the macOS window buttons, which only the desktop app's side pane has
        let left = if cfg!(target_arch = "wasm32") || ui.ctx().content_rect().width() < NARROW {
            16
        } else {
            80
        };
        egui::Panel::top("header")
            .exact_size(46.0)
            .frame(egui::Frame::new().fill(PAPER).inner_margin(egui::Margin {
                left,
                right: 16,
                top: 0,
                bottom: 0,
            }))
            .show(ui, |ui| self.header(ui));
        egui::Panel::bottom("status")
            .exact_size(32.0)
            .frame(
                egui::Frame::new()
                    .fill(PAPER)
                    .inner_margin(egui::Margin::symmetric(16, 0)),
            )
            .show(ui, |ui| self.status(ui));
        if self.ref_open {
            egui::Panel::bottom("reference")
                .resizable(true)
                .default_size(380.0)
                .size_range(180.0..=900.0)
                .frame(
                    egui::Frame::new()
                        .fill(GLASS)
                        .stroke(Stroke::new(1.0, LINE))
                        .inner_margin(egui::Margin {
                            left: 10,
                            right: 12,
                            top: 0,
                            bottom: 0,
                        }),
                )
                .show(ui, |ui| self.reference(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(PAPER))
            .show(ui, |ui| self.code(ui));
    }

    fn header(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            ui.label(
                RichText::new("monet")
                    .font(FontId::proportional(15.0))
                    .color(INK)
                    .strong(),
            );
            ui.add_space(14.0);
            let doc = self.doc();
            let title = format!("{}{}", doc.name, if doc.dirty() { " •" } else { "" });
            let r = ui.add(
                egui::Label::new(
                    RichText::new(title)
                        .font(FontId::proportional(14.0))
                        .color(MUTED),
                )
                .sense(Sense::click()),
            );
            let (cr, chev) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::click());
            let c = cr.center();
            let ink = if r.hovered() || chev.hovered() {
                INK
            } else {
                FAINT
            };
            ui.painter().line(
                vec![
                    c + vec2(-3.5, -1.5),
                    c + vec2(0.0, 2.0),
                    c + vec2(3.5, -1.5),
                ],
                Stroke::new(1.3, ink),
            );
            let r = r
                .union(chev)
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if r.clicked() {
                self.menu_open = !self.menu_open;
            }
            if self.menu_open {
                self.sketch_menu(ui, r.rect.left_bottom() + vec2(-10.0, 8.0));
            }
            if self.doc().original.is_some_and(|o| o != self.doc().src) && self.doc().path.is_none()
            {
                ui.add_space(6.0);
                if text_button(ui, "revert", FAINT).clicked() {
                    let o = self.doc().original.unwrap().to_string();
                    self.doc_mut().src = o;
                    self.evaluate();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let fps = if !self.playing {
                    "paused".into()
                } else if self.engine.is_still() && !self.drew {
                    "still · 0 gpu".into()
                } else {
                    format!("{:.0} fps", self.fps.round())
                };
                ui.label(
                    RichText::new(fps)
                        .font(FontId::monospace(11.5))
                        .color(FAINT),
                );
            });
        });
    }

    fn sketch_menu(&mut self, ui: &mut Ui, at: Pos2) {
        let mut pick = None;
        let area = egui::Area::new(Id::unique("sketch-menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                // short windows (laptops, browsers) scroll the list instead of cutting it off
                let room = (ui.ctx().content_rect().bottom() - at.y - 40.0).max(160.0);
                card().show(ui, |ui| {
                    ui.set_min_width(190.0);
                    ui.set_max_width(220.0);
                    egui::ScrollArea::vertical()
                        .max_height(room)
                        .auto_shrink([true, true])
                        .show(ui, |ui| {
                    let files: Vec<usize> = (0..self.docs.len())
                        .filter(|&i| self.docs[i].path.is_some())
                        .collect();
                    if !files.is_empty() {
                        section(ui, "files");
                        for i in files {
                            if menu_item(ui, &self.docs[i].name, i == self.active).clicked() {
                                pick = Some(i);
                            }
                        }
                    }
                    for group in ["monet", "paintings", "covers", "fractals", ""] {
                        if !group.is_empty() {
                            section(ui, group);
                        } else {
                            ui.add_space(4.0);
                        }
                        for p in presets::PRESETS.iter().filter(|p| p.group == group) {
                            if let Some(i) = self
                                .docs
                                .iter()
                                .position(|d| d.name == p.name && d.path.is_none())
                                && menu_item(ui, p.name, i == self.active).clicked()
                            {
                                pick = Some(i);
                            }
                        }
                    }
                        });
                });
            });
        if let Some(i) = pick {
            self.menu_open = false;
            self.switch_to(i);
        } else if ui.ctx().input(|i| i.pointer.any_pressed())
            && !area.response.contains_pointer()
            && !ui
                .ctx()
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| ui.max_rect().contains(p))
        {
            self.menu_open = false;
        }
    }

    fn status(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            let toast = self
                .toast
                .as_ref()
                .filter(|(_, t)| t.elapsed().as_secs_f32() < 2.5)
                .map(|(s, _)| s.clone());
            let src = &self.doc().src;
            let (dot, text, color) = if let Some(t) = toast {
                (Some(ACCENT), t, MUTED)
            } else if let Some(d) = &self.diag {
                let at = d.span.as_ref().map(|s| {
                    let (l, c) = chain::Error::new(s.clone(), "").line_col(src);
                    format!("{l}:{c}  ")
                });
                (
                    Some(ERR),
                    format!("{}{}", at.unwrap_or_default(), d.message),
                    ERR,
                )
            } else if self.compiling() {
                (Some(AMBER), "compiling…".to_string(), MUTED)
            } else if self.program.is_some() {
                // all good: nothing to say
                (None, String::new(), FAINT)
            } else {
                (Some(AMBER), "starting…".to_string(), MUTED)
            };
            if let Some(dot) = dot {
                let (r, p) = ui.allocate_painter(vec2(8.0, 8.0), Sense::hover());
                p.circle_filled(r.rect.center(), 3.0, dot);
                ui.add_space(4.0);
            }
            let span = self.diag.as_ref().and_then(|d| d.span.clone());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if text_button(ui, "reference  ⌘K", if self.ref_open { INK } else { FAINT })
                    .clicked()
                {
                    self.ref_open = !self.ref_open;
                }
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(text)
                                .font(FontId::monospace(11.5))
                                .color(color),
                        )
                        .truncate()
                        .sense(Sense::click()),
                    );
                    if let Some(s) = span
                        && resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                    {
                        self.goto = Some(s.start);
                    }
                });
            });
            if self
                .toast
                .as_ref()
                .is_some_and(|(_, t)| t.elapsed().as_secs_f32() < 2.6)
            {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(200));
            }
        });
    }

    fn code(&mut self, ui: &mut Ui) {
        let id = Id::unique(EDITOR_ID);
        let error = self.diag.as_ref().and_then(|d| d.span.clone());
        let steps = match &self.program {
            Some((_, src)) if *src == self.doc().src => self.playing_steps.clone(),
            _ => Vec::new(),
        };
        let font = FontId::monospace(13.0);
        let syntax = syntax_theme();
        let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
            let job = highlight::layout(buf.as_str(), font.clone(), &syntax, &steps, error.clone());
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let gutter = 46.0;
        let before = self.doc().src.clone();
        let mut changed = false;
        let alt = ui.input(|i| i.modifiers.alt);

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let avail = ui.available_size();
                ui.horizontal_top(|ui| {
                    ui.add_space(gutter);
                    let bg = ui.painter().add(egui::Shape::Noop);
                    let doc = &mut self.docs[self.active];
                    if std::mem::take(&mut self.focus_end) {
                        let mut state =
                            egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                        let n = doc.src.chars().count();
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(
                                egui::text::CCursor::new(n),
                            )));
                        state.store(ui.ctx(), id);
                        ui.memory_mut(|m| m.request_focus(id));
                    }
                    let focused = ui.memory(|m| m.has_focus(id));
                    if !focused {
                        self.completion = None;
                    }
                    // Autocomplete keys go first, before the text edit sees them.
                    let mut accept: Option<usize> = None;
                    if let Some((c, sel, _)) = &mut self.completion {
                        let n = c.items.len();
                        let (tab, enter, up, down, esc) = ui.input_mut(|i| {
                            (
                                i.consume_key(Modifiers::NONE, Key::Tab),
                                i.consume_key(Modifiers::NONE, Key::Enter),
                                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                                i.consume_key(Modifiers::NONE, Key::Escape),
                            )
                        });
                        let (pgup, pgdn) = ui.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::PageUp), i.consume_key(Modifiers::NONE, Key::PageDown)));
                        if pgup {
                            *sel = sel.saturating_sub(7);
                        }
                        if pgdn {
                            *sel = (*sel + 7).min(n - 1);
                        }
                        if up {
                            *sel = (*sel + n - 1) % n;
                        }
                        if down {
                            *sel = (*sel + 1) % n;
                        }
                        if tab || enter {
                            accept = Some(*sel);
                        }
                        if esc {
                            self.completion = None;
                        }
                    }
                    let ectx = ui.ctx().clone();
                    let apply = |doc: &mut Doc,
                                 completion: &mut Option<(complete::Completion, usize, usize)>,
                                 k: usize| {
                        if let Some((c, _, _)) = completion.take()
                            && let Some(item) = c.items.get(k)
                        {
                            let at = complete::accept(&mut doc.src, &c, item);
                            let mut state =
                                egui::text_edit::TextEditState::load(&ectx, id).unwrap_or_default();
                            let ci = editing::byte_to_char(&doc.src, at);
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(ci),
                                )));
                            state.store(&ectx, id);
                            Some(at)
                        } else {
                            None
                        }
                    };
                    if let Some(k) = accept
                        && let Some(at) = apply(doc, &mut self.completion, k)
                    {
                        changed = true;
                        // keep going: `blend("` lists its modes, `mouse.` its axes
                        self.completion = complete::complete(&doc.src, at).map(|c| (c, 0, at));
                    }
                    let out = egui::TextEdit::multiline(&mut doc.src)
                        .id(id)
                        .code_editor()
                        .frame(egui::Frame::NONE)
                        .lock_focus(true)
                        .desired_width(f32::INFINITY)
                        .min_size(vec2(avail.x - gutter, avail.y))
                        .margin(egui::Margin {
                            left: 0,
                            right: 24,
                            top: 14,
                            bottom: 120,
                        })
                        .layouter(&mut layouter)
                        .show(ui);
                    changed |= out.response.changed();
                    let galley = out.galley.clone();
                    let origin = out.galley_pos;
                    let rect_of = |src: &str, r: &Range<usize>| -> Rect {
                        let a = galley.pos_from_cursor(egui::text::CCursor::new(
                            editing::byte_to_char(src, r.start),
                        ));
                        let b = galley.pos_from_cursor(egui::text::CCursor::new(
                            editing::byte_to_char(src, r.end),
                        ));
                        Rect::from_min_max(a.min, pos2(b.max.x.max(a.max.x), a.max.y))
                            .translate(origin.to_vec2())
                    };

                    // line numbers + error stripe
                    let painter = ui.painter().clone();
                    let err_line = error
                        .as_ref()
                        .map(|e| chain::Error::new(e.clone(), "").line_col(&doc.src).0);
                    let mut line = 1usize;
                    let mut at_line_start = true;
                    let x = origin.x - 14.0;
                    for row in &galley.rows {
                        if at_line_start {
                            let y = origin.y + row.min_y();
                            let is_err = err_line == Some(line);
                            if is_err {
                                painter.set(
                                    bg,
                                    egui::Shape::rect_filled(
                                        Rect::from_min_max(
                                            pos2(origin.x - gutter, y - 1.0),
                                            pos2(
                                                ui.max_rect().right() + 4000.0,
                                                origin.y + row.max_y() + 1.0,
                                            ),
                                        ),
                                        0.0,
                                        ERR_BG,
                                    ),
                                );
                            }
                            painter.text(
                                pos2(x, origin.y + (row.min_y() + row.max_y()) * 0.5),
                                Align2::RIGHT_CENTER,
                                line.to_string(),
                                FontId::monospace(11.0),
                                if is_err { ERR } else { GUTTER },
                            );
                        }
                        at_line_start = row.ends_with_newline;
                        if at_line_start {
                            line += 1;
                        }
                    }

                    // colour swatches under every #hex
                    for (r, k) in highlight::tokens(&doc.src) {
                        if k == highlight::Kind::Color
                            && let Some(rgb) = chain::mini::parse_hex(&doc.src[r.start + 1..r.end])
                        {
                            let rect = rect_of(&doc.src, &r);
                            let c = egui::Color32::from_rgb(
                                (rgb[0] * 255.0) as u8,
                                (rgb[1] * 255.0) as u8,
                                (rgb[2] * 255.0) as u8,
                            );
                            painter.rect_filled(
                                Rect::from_min_max(
                                    pos2(rect.left(), rect.bottom() - 0.5),
                                    pos2(rect.right(), rect.bottom() + 2.5),
                                ),
                                1.5,
                                c,
                            );
                        }
                    }

                    // ⌥-drag scrubs numbers; hovering a name shows its docs
                    let hover = ui
                        .input(|i| i.pointer.hover_pos())
                        .filter(|p| out.response.rect.contains(*p));
                    let hovered_byte = hover.map(|p| {
                        let cc = galley.cursor_from_pos(p - origin);
                        editing::char_to_byte(&doc.src, cc.index.0)
                    });
                    if let Some(s) = &mut self.scrub {
                        let resp = ui.interact(
                            rect_of(&doc.src, &s.range).expand(4.0),
                            Id::unique("scrub"),
                            Sense::drag(),
                        );
                        if resp.dragged() {
                            let dx = ui.input(|i| i.pointer.delta().x) as f64;
                            let fine = if ui.input(|i| i.modifiers.shift) {
                                0.1
                            } else {
                                1.0
                            };
                            s.value += dx * editing::scrub_step(&s.orig) * fine;
                            s.range =
                                editing::replace_number(&mut doc.src, s.range.clone(), s.value);
                            changed = true;
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        } else {
                            self.scrub = None;
                        }
                    } else if alt
                        && let Some(b) = hovered_byte
                        && let Some(r) = highlight::numbers(&doc.src)
                            .into_iter()
                            .find(|r| r.start <= b && b <= r.end)
                    {
                        let rect = rect_of(&doc.src, &r);
                        painter.hline(
                            rect.x_range(),
                            rect.bottom() + 1.0,
                            Stroke::new(1.0, ACCENT),
                        );
                        let resp =
                            ui.interact(rect.expand(4.0), Id::unique("scrub"), Sense::drag());
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        if resp.drag_started() || resp.dragged() {
                            let orig = doc.src[r.clone()].to_string();
                            let value = orig.parse().unwrap_or(0.0);
                            self.scrub = Some(Scrub {
                                range: r,
                                orig,
                                value,
                            });
                        }
                    } else if let Some(b) = hovered_byte
                        && !out.response.dragged()
                        && let Some(tip) = hints::docs_at(&doc.src, b)
                    {
                        egui::Tooltip::always_open(
                            ui.ctx().clone(),
                            ui.layer_id(),
                            Id::unique("doc-tip"),
                            egui::PopupAnchor::Pointer,
                        )
                        .gap(12.0)
                        .show(|ui| {
                            ui.set_max_width(380.0);
                            ui.label(
                                RichText::new(tip.0)
                                    .font(FontId::monospace(12.0))
                                    .color(INK),
                            );
                            if let Some(cols) = tip.2 {
                                let (r, _) = ui.allocate_exact_size(vec2(180.0, 12.0), Sense::hover());
                                hints::swatch(ui.painter(), r, cols);
                            }
                            ui.label(
                                RichText::new(tip.1)
                                    .font(FontId::proportional(12.0))
                                    .color(MUTED),
                            );
                        });
                    }

                    // autocomplete: refresh as you type; close when the cursor wanders
                    let cursor_b = egui::text_edit::TextEditState::load(ui.ctx(), id)
                        .and_then(|st| st.cursor.char_range())
                        .map(|r| editing::char_to_byte(&doc.src, r.primary.index.0));
                    if let Some(b) = cursor_b {
                        if out.response.changed() && focused {
                            // words, dots, and the quotes or `<` that open palettes and options
                            let typed_word = doc.src[..b].ends_with(|c: char| {
                                c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '"' | '<')
                            });
                            self.completion = if typed_word {
                                complete::complete(&doc.src, b).map(|c| (c, 0, b))
                            } else {
                                None
                            };
                        } else if self.completion.as_ref().is_some_and(|c| c.2 != b) {
                            self.completion = None;
                        }
                    }
                    if let (Some(b), true) = (cursor_b, focused) {
                        let caret = galley
                            .pos_from_cursor(egui::text::CCursor::new(editing::byte_to_char(
                                &doc.src, b,
                            )))
                            .translate(origin.to_vec2());
                        let mut clicked: Option<usize> = None;
                        if let Some((c, sel, _)) = &self.completion {
                            clicked = hints::completion(ui.ctx(), caret.left_bottom() + vec2(-8.0, 6.0), c, *sel);
                        } else if let Some(sig) = complete::signature(&doc.src, b) {
                            hints::signature(ui.ctx(), caret.left_top() + vec2(-8.0, -6.0), &sig);
                        }
                        if let Some(k) = clicked {
                            if let Some((_, sel, _)) = &mut self.completion {
                                *sel = k;
                            }
                            if let Some(at) = apply(doc, &mut self.completion, k) {
                                changed = true;
                                self.completion = complete::complete(&doc.src, at).map(|c| (c, 0, at));
                            }
                            ui.memory_mut(|m| m.request_focus(id));
                        }
                    }

                    // post-edit niceties: tabs → spaces, auto-indent
                    if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), id) {
                        if let Some(range) = state.cursor.char_range() {
                            self.cursor = range.primary.index.0;
                        }
                        if out.response.changed()
                            && let Some(range) = state.cursor.char_range()
                        {
                            let mut c = range.primary.index.0;
                            let typed_newline = doc.src.chars().count()
                                == before.chars().count() + 1
                                && doc.src.chars().nth(c.wrapping_sub(1)) == Some('\n');
                            c = editing::expand_tabs(&mut doc.src, c);
                            if typed_newline {
                                c = editing::auto_indent(&mut doc.src, c);
                            }
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(c),
                                )));
                            state.store(ui.ctx(), id);
                        }
                    }

                    // ⌘/ toggles comments; clicking an error jumps to it
                    if out.response.has_focus()
                        && ui.ctx().input_mut(|i| {
                            i.consume_shortcut(&KeyboardShortcut::new(
                                Modifiers::COMMAND,
                                Key::Slash,
                            ))
                        })
                        && let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), id)
                        && let Some(r) = state.cursor.char_range()
                    {
                        let (a, b) = editing::toggle_comment(
                            &mut doc.src,
                            r.primary.index.0,
                            r.secondary.index.0,
                        );
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(a),
                                egui::text::CCursor::new(b),
                            )));
                        state.store(ui.ctx(), id);
                        changed = true;
                    }
                    if let Some(b) = self.goto.take()
                        && let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), id)
                    {
                        let c = editing::byte_to_char(&doc.src, b);
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(
                                egui::text::CCursor::new(c),
                            )));
                        state.store(ui.ctx(), id);
                        ui.memory_mut(|m| m.request_focus(id));
                    }
                });
            });
        if changed || self.doc().src != before {
            self.evaluate();
        }
    }

    /// Insert something from the reference card at the editor's cursor.
    fn insert(&mut self, ctx: &egui::Context, what: Insert) {
        let id = Id::unique(EDITOR_ID);
        let cursor = self.cursor.min(self.doc().src.chars().count());
        let doc = &mut self.docs[self.active];
        let c = match what {
            Insert::Layer(s) => editing::insert_layer(&mut doc.src, cursor, &format!("{s}()")),
            Insert::Effect(s) => editing::insert_effect(&mut doc.src, cursor, &format!("{s}()")),
            Insert::Text(s) => editing::insert_at(&mut doc.src, cursor, &s),
        };
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(c),
            )));
        state.store(ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
        self.cursor = c;
        self.evaluate();
    }

    fn reference(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        match reference::show(ui, &mut self.ref_query, &mut self.ref_focus) {
            Some(reference::Action::Insert(what)) => self.insert(&ctx, what),
            Some(reference::Action::Close) => self.ref_open = false,
            None => {}
        }
    }

    // ── canvas ───────────────────────────────────────────────────────────────

    fn canvas(&mut self, ui: &mut Ui, frame: &eframe::Frame) {
        let rect = ui.max_rect();
        let resp = ui.interact(rect, Id::unique("canvas"), Sense::click_and_drag());
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();

        let w = ((rect.width() * ppp) / self.scale as f32).round().max(1.0) as u32;
        let h = ((rect.height() * ppp) / self.scale as f32).round().max(1.0) as u32;
        if (self.engine.ensure_target(w, h) || self.texture.is_none())
            && let (Some(rs), Some(t)) = (frame.wgpu_render_state(), self.engine.target())
        {
            let mut r = rs.renderer.write();
            match self.texture {
                Some(id) => r.update_egui_texture_from_wgpu_texture(
                    &rs.device,
                    &t.view,
                    wgpu::FilterMode::Nearest,
                    id,
                ),
                None => {
                    self.texture = Some(r.register_native_texture(
                        &rs.device,
                        &t.view,
                        wgpu::FilterMode::Nearest,
                    ))
                }
            }
        }

        self.interact(&ctx, &resp, rect);

        if let Some((program, _)) = &self.program {
            let clock = chain::Clock {
                cycle: self.cycle,
                mouse: self.mouse,
            };
            let f = program.frame(&clock);
            let v = self.doc().view;
            self.drew = self.engine.render(
                &Uniforms {
                    res: [w as f32, h as f32],
                    mouse: self.mouse,
                    time: (self.cycle * 2.0) as f32,
                    dt: 1.0 / self.fps.max(1.0),
                    frame: self.frame as f32,
                    cycle: self.cycle as f32,
                    view: [v[0], v[1], v[2], 0.0],
                },
                &f,
            );
            self.playing_steps = f.active;
        }

        let radius = if self.focus { 0 } else { 14 };
        if !self.focus {
            ui.painter().add(
                egui::epaint::Shadow {
                    offset: [0, 10],
                    blur: 34,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(26),
                }
                .as_shape(rect, radius),
            );
        }
        if let Some(id) = self.texture
            && self.engine.has_graph()
        {
            egui::Image::from_texture(egui::load::SizedTexture::new(id, rect.size()))
                .corner_radius(radius)
                .paint_at(ui, rect);
        } else {
            ui.painter().rect_filled(rect, radius, PAPER);
        }

        let recently = self.last_pointer_move.elapsed().as_secs_f32() < 2.2;
        let show_chrome = !self.focus || recently || !self.playing;
        if show_chrome {
            self.transport(&ctx, rect);
            if !self.sliders().is_empty() || self.uses_view() {
                self.params_card(&ctx, rect);
            }
        }
        if self.focus && recently {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn interact(&mut self, ctx: &egui::Context, resp: &egui::Response, rect: Rect) {
        // moving the mouse, or a tap or touch (no movement), brings the controls back
        if resp.hovered() && ctx.input(|i| i.pointer.delta() != Vec2::ZERO)
            || resp.is_pointer_button_down_on()
            || resp.clicked()
            || (resp.contains_pointer() && ctx.input(|i| i.any_touches()))
        {
            self.last_pointer_move = Instant::now();
        }
        let aspect = rect.width() / rect.height();
        let to_uv = |p: Pos2| {
            [
                (p.x - rect.left()) / rect.width(),
                1.0 - (p.y - rect.top()) / rect.height(),
            ]
        };
        let to_centered = |uv: [f32; 2]| [(uv[0] - 0.5) * aspect, uv[1] - 0.5];
        let plane = self.uses_view();
        let shift = ctx.input(|i| i.modifiers.shift);
        let pan = plane != shift;

        if resp.double_clicked() && plane {
            let d = self.doc_mut();
            d.view = d.home.unwrap_or([0.0, 0.0, 1.0]);
        }
        // the mouse follows the pointer over the canvas (lens, water ripples…)
        if let Some(p) = ctx
            .input(|i| i.pointer.hover_pos())
            .filter(|p| rect.contains(*p))
            && !(pan && resp.dragged())
        {
            self.mouse = to_uv(p);
        }
        if pan && resp.dragged() {
            let d = resp.drag_delta();
            let view = &mut self.docs[self.active].view;
            view[0] -= d.x / rect.height() / view[2];
            view[1] += d.y / rect.height() / view[2];
        }
        if plane && resp.hovered() {
            let (scroll, pinch) = ctx.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (scroll * 0.004).exp() * pinch;
            if factor != 1.0
                && let Some(p) = ctx.input(|i| i.pointer.hover_pos())
            {
                let c = to_centered(to_uv(p));
                let view = &mut self.docs[self.active].view;
                let world = [c[0] / view[2] + view[0], c[1] / view[2] + view[1]];
                view[2] = (view[2] * factor).clamp(1e-3, 2e5);
                view[0] = world[0] - c[0] / view[2];
                view[1] = world[1] - c[1] / view[2];
            }
        }
    }

    fn transport(&mut self, ctx: &egui::Context, rect: Rect) {
        egui::Area::new(Id::unique("transport"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(rect.center_bottom() - vec2(0.0, 16.0))
            .show(ctx, |ui| {
                pill().show(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        let compact = rect.width() < 460.0;
                        ui.spacing_mut().item_spacing.x = if compact { 10.0 } else { 12.0 };
                        if text_button(ui, if self.playing { "pause" } else { "play" }, INK)
                            .clicked()
                        {
                            self.playing = !self.playing;
                        }
                        // the cycle: whole cycles · a dot per quarter
                        let c = self.cycle.max(0.0);
                        if !compact {
                        let beat = ((c.fract() * 4.0) as usize).min(3);
                        let dots: String =
                            (0..4).map(|i| if i == beat { '●' } else { '·' }).collect();
                        let t = RichText::new(format!("{:>3} {dots}", c.floor() as u64))
                            .font(FontId::monospace(12.0))
                            .color(MUTED);
                        if ui
                            .add(egui::Label::new(t).sense(Sense::click()))
                            .on_hover_text(format!(
                                "cycle {:.2} · {:.1}s · click to restart  ⌘R",
                                c, self.seconds
                            ))
                            .clicked()
                        {
                            self.restart();
                        }
                        }
                        // phones: no room for pixel sizes (⌘1–⌘4 has no keyboard there anyway)
                        if !compact {
                        divider(ui);
                        for s in [1u32, 2, 4, 8] {
                            let c = if self.scale == s { INK } else { FAINT };
                            if text_button(ui, &format!("{s}×"), c)
                                .on_hover_text("pixel size  ⌘1–⌘4")
                                .clicked()
                            {
                                self.scale = s;
                            }
                        }
                        }
                        divider(ui);
                        if text_button(ui, "png", MUTED)
                            .on_hover_text("export  ⌘E")
                            .clicked()
                        {
                            self.export();
                        }
                        #[cfg(target_arch = "wasm32")]
                        if text_button(ui, "share", MUTED)
                            .on_hover_text("copy a link to this sketch  ⌘⇧C")
                            .clicked()
                        {
                            self.share(ui.ctx());
                        }
                        let f = if self.focus { "editor" } else { "focus" };
                        if text_button(ui, f, MUTED)
                            .on_hover_text("toggle editor  ⌘↵")
                            .clicked()
                        {
                            self.focus = !self.focus;
                        }
                    });
                });
            });
    }

    fn params_card(&mut self, ctx: &egui::Context, rect: Rect) {
        let plane = self.uses_view();
        let sliders = self.sliders();
        let mut set: Option<(Range<usize>, f64)> = None;
        egui::Area::new(Id::unique("params"))
            .order(egui::Order::Foreground)
            .pivot(Align2::RIGHT_TOP)
            .fixed_pos(rect.right_top() + vec2(-16.0, 16.0))
            .show(ctx, |ui| {
                card().show(ui, |ui| {
                    // fits a phone: never wider than most of the picture
                    let w = 250.0f32.min(rect.width() - 64.0).max(120.0);
                    ui.set_width(w);
                    ui.spacing_mut().slider_width = w - 28.0;
                    ui.horizontal(|ui| {
                        // a fractal with no sliders shows only where you are: call it that
                        let title = if sliders.is_empty() { "view" } else { "sliders" };
                        let label = if self.params_open {
                            title.to_string()
                        } else {
                            format!("{title}  ›")
                        };
                        let label = label.as_str();
                        if text_button(ui, label, MUTED).clicked() {
                            self.params_open = !self.params_open;
                        }
                        if self.params_open && plane {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if text_button(ui, "home", FAINT).clicked() {
                                        let d = &mut self.docs[self.active];
                                        d.view = d.home.unwrap_or([0.0, 0.0, 1.0]);
                                    }
                                },
                            );
                        }
                    });
                    if !self.params_open {
                        return;
                    }
                    ui.add_space(4.0);
                    let src = self.doc().src.clone();
                    for (k, s) in sliders.iter().enumerate() {
                        let lit = &src[s.span.clone()];
                        let mut v = lit.parse::<f64>().unwrap_or(s.value);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&s.label)
                                    .font(FontId::monospace(11.5))
                                    .color(INK),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(lit)
                                            .font(FontId::monospace(11.5))
                                            .color(MUTED),
                                    );
                                },
                            );
                        });
                        let r = ui.push_id(k, |ui| {
                            ui.add(
                                egui::Slider::new(&mut v, s.lo..=s.hi)
                                    .show_value(false)
                                    .trailing_fill(true)
                                    .clamping(egui::SliderClamping::Always),
                            )
                        });
                        if r.inner.changed() {
                            set = Some((s.span.clone(), v));
                        }
                        ui.add_space(2.0);
                    }
                    if plane {
                        ui.add_space(2.0);
                        let v = self.doc().view;
                        ui.label(
                            RichText::new(format!(
                                "{:+.6} {:+.6}i  ×{}",
                                v[0],
                                v[1],
                                fmt_zoom(v[2])
                            ))
                            .font(FontId::monospace(10.5))
                            .color(FAINT),
                        )
                        .on_hover_text("scroll to zoom · drag to pan · double-click to reset");
                        // say how, since nothing on screen suggests it
                        let how = if cfg!(target_arch = "wasm32") && ui.ctx().content_rect().width() < NARROW {
                            "pinch to zoom · drag to pan"
                        } else {
                            "scroll to zoom · drag to pan"
                        };
                        ui.label(RichText::new(how).font(FontId::proportional(11.0)).color(FAINT));
                    }
                });
            });
        if let Some((span, v)) = set {
            // Sliders write back into the code, so the code is always the truth.
            let lit = self.doc().src[span.clone()].to_string();
            let decimals = editing::decimals(&lit).max(if v.abs() < 10.0 { 2 } else { 0 });
            let text = format!("{v:.decimals$}");
            self.doc_mut().src.replace_range(span, &text);
            self.evaluate();
        }
    }
}

enum Insert {
    Layer(&'static str),
    Effect(&'static str),
    Text(String),
}


impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if std::mem::take(&mut self.reset_zoom) {
            ctx.set_zoom_factor(1.0);
        }
        self.shortcuts(&ctx);
        self.poll_compiler();
        self.poll_file();
        self.tick();

        let screen = ctx.content_rect();
        if !self.focus && screen.width() < NARROW {
            // phones: the picture on top, the code below
            egui::Panel::bottom("editor-pane-narrow")
                .resizable(true)
                .default_size(screen.height() * 0.5)
                .size_range(140.0..=screen.height() * 0.8)
                .frame(
                    egui::Frame::new()
                        .fill(PAPER)
                        .stroke(Stroke::new(1.0, LINE)),
                )
                .show(ui, |ui| self.editor_pane(ui));
        } else if !self.focus {
            let width = screen.width();
            // "editor-pane": a fresh id drops a width saved under the old 1.5× zoom
            egui::Panel::left("editor-pane")
                .resizable(true)
                .default_size((width * 0.4).clamp(380.0, 640.0))
                .size_range(320.0..=width * 0.7)
                .frame(
                    egui::Frame::new()
                        .fill(PAPER)
                        .stroke(Stroke::new(1.0, LINE)),
                )
                .show(ui, |ui| self.editor_pane(ui));
        }
        let margin = if self.focus { 0 } else if screen.width() < NARROW { 8 } else { 18 };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(SURROUND)
                    .inner_margin(egui::Margin::same(margin)),
            )
            .show(ui, |ui| self.canvas(ui, frame));

        if self.playing && (self.drew || !self.engine.is_still() || self.compiling()) {
            ctx.request_repaint();
        } else {
            // A still picture: nothing to redraw until you touch something.
            // Keep a slow heartbeat for hot reload and toasts.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        self.capture(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(ZOOM_RESET, "1".into());
        // Scripted demo runs (MONET_SHOT / MONET_TYPE) never touch your saved sketches.
        if self.shot.is_some() || std::env::var_os("MONET_TYPE").is_some() {
            return;
        }
        for d in &self.docs {
            if d.path.is_none() {
                let key = format!("{STORE}/doc/{}", d.name);
                storage.set_string(
                    &key,
                    if d.dirty() {
                        d.src.clone()
                    } else {
                        String::new()
                    },
                );
            }
        }
        storage.set_string(&format!("{STORE}/active"), self.doc().name.clone());
    }
}

impl App {
    fn capture(&mut self, ctx: &egui::Context) {
        let Some(path) = self.shot.clone() else {
            return;
        };
        ctx.request_repaint();
        if self.frame != 90 {
            return;
        }
        let ctx2 = ctx.clone();
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
            egui::ScreenshotCallback::new(move |img| {
                let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                let out = crate::engine::Image {
                    width: img.size[0] as u32,
                    height: img.size[1] as u32,
                    rgba,
                };
                if let Err(e) = out.save_png(&path) {
                    eprintln!("monet: screenshot: {e}");
                }
                ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
            }),
        ));
    }
}

// ── small helpers ────────────────────────────────────────────────────────────

fn fmt_zoom(z: f32) -> String {
    if z >= 1000.0 {
        format!("{:.2e}", z)
    } else {
        format!("{z:.2}")
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn short_path(p: &std::path::Path) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    p.strip_prefix(&cwd).map_or_else(
        |_| p.display().to_string(),
        |r| format!("./{}", r.display()),
    )
}

fn text_button(ui: &mut Ui, text: &str, color: egui::Color32) -> egui::Response {
    let r = ui.add(
        egui::Label::new(
            RichText::new(text)
                .font(FontId::proportional(12.5))
                .color(color),
        )
        .sense(Sense::click()),
    );
    if r.hovered() {
        ui.painter().hline(
            r.rect.x_range(),
            r.rect.bottom() + 1.0,
            Stroke::new(1.0, FAINT),
        );
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn divider(ui: &mut Ui) {
    let (r, p) = ui.allocate_painter(vec2(1.0, 14.0), Sense::hover());
    p.vline(r.rect.center().x, r.rect.y_range(), Stroke::new(1.0, LINE));
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(title)
            .font(FontId::proportional(11.0))
            .color(FAINT),
    );
}

fn menu_item(ui: &mut Ui, name: &str, active: bool) -> egui::Response {
    let color = if active { INK } else { MUTED };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if resp.hovered() || active {
        ui.painter()
            .rect_filled(rect.expand2(vec2(4.0, 0.0)), 6.0, HOVER);
    }
    ui.painter().text(
        rect.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(13.5),
        color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(GLASS)
        .corner_radius(CornerRadius::same(12))
        .stroke(Stroke::new(1.0, LINE))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: egui::Color32::from_black_alpha(22),
        })
}

fn pill() -> egui::Frame {
    card()
        .corner_radius(CornerRadius::same(255))
        .inner_margin(egui::Margin::symmetric(18, 9))
}
