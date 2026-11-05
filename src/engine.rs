//! GPU side: a tiny render graph executor.
//!
//! * A sketch compiles (on the main thread, in microseconds) to a `chain::Graph`
//!   of fullscreen passes. Only when the graph's *shader text* changes is it
//!   sent to a background thread to become pipelines — so numbers, patterns,
//!   sliders and scrubbing never compile anything.
//! * The worker is latest-wins, validates with naga for readable errors, builds
//!   pipelines inside error scopes, and caches them by source: editing one
//!   layer only rebuilds the passes that changed.
//! * Intermediate buffers are pooled per canvas size; feedback (`trail`)
//!   buffers ping-pong between frames.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use web_time::Instant;

use eframe::egui_wgpu::wgpu;

use crate::chain::{self, Frame, Graph, TexRef};

/// The canvas: display-referred sRGB values stored as-is (what you write is
/// what you see; egui composites in gamma space too).
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Intermediate buffers keep HDR values for bloom and glow.
pub const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

const HEADER_FLOATS: usize = 12;
pub const UNIFORM_SIZE: u64 = ((HEADER_FLOATS
    + chain::build::MAX_SCALARS
    + 4 * chain::build::MAX_PALETTES * chain::build::PAL_STOPS)
    * 4) as u64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Uniforms {
    pub res: [f32; 2],
    pub mouse: [f32; 2],
    /// "Seconds" on the musical clock (cycles × 2).
    pub time: f32,
    pub dt: f32,
    pub frame: f32,
    pub cycle: f32,
    /// centre.x, centre.y, zoom
    pub view: [f32; 4],
}

impl Default for Uniforms {
    fn default() -> Self {
        Self {
            res: [1.0, 1.0],
            mouse: [0.5, 0.5],
            time: 0.0,
            dt: 1.0 / 60.0,
            frame: 0.0,
            cycle: 0.0,
            view: [0.0, 0.0, 1.0, 0.0],
        }
    }
}

impl Uniforms {
    pub fn to_bytes(&self, f: &Frame) -> Vec<u8> {
        let head = [
            self.res[0],
            self.res[1],
            self.mouse[0],
            self.mouse[1],
            self.time,
            self.dt,
            self.frame,
            self.cycle,
            self.view[0],
            self.view[1],
            self.view[2],
            self.view[3],
        ];
        let mut out = Vec::with_capacity(UNIFORM_SIZE as usize);
        for v in head
            .iter()
            .chain(&f.scalars)
            .chain(f.colors.iter().flatten())
        {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// A compile problem. `span` is a byte range in the sketch when the error is
/// the sketch's; GPU-side errors (bugs in monet) have none.
#[derive(Clone, Debug, PartialEq)]
pub struct Diag {
    pub span: Option<std::ops::Range<usize>>,
    pub message: String,
}

impl From<chain::Error> for Diag {
    fn from(e: chain::Error) -> Self {
        Self {
            span: Some(e.span),
            message: e.msg,
        }
    }
}

impl std::fmt::Display for Diag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Strip naga's internal handles from messages.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("[") {
        out.push_str(&rest[..i]);
        match rest[i..].find(']') {
            Some(j) if rest[i + 1..i + j].chars().all(|c| c.is_ascii_digit()) => {
                rest = &rest[i + j + 1..]
            }
            _ => {
                out.push('[');
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Parse + validate WGSL with naga.
pub fn validate(src: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(src).map_err(|e| e.emit_to_string(src))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|e| {
        let mut msg = e.as_inner().to_string();
        let mut cur: Option<&dyn std::error::Error> = std::error::Error::source(e.as_inner());
        while let Some(c) = cur {
            msg = c.to_string();
            cur = c.source();
        }
        tidy(&msg)
    })?;
    Ok(())
}

#[derive(Debug)]
pub struct GpuGraph {
    pub desc: Graph,
    pipelines: Vec<wgpu::RenderPipeline>,
    images: Vec<wgpu::TextureView>,
    /// Does any pass read the clock (time, cycle, frame, dt) or the mouse?
    /// If not, a frame whose other inputs are unchanged can be skipped.
    pub reads_time: bool,
    pub reads_mouse: bool,
}

/// Everything needed to build pipelines; cheap to clone into the worker.
#[derive(Clone)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))] // images are read from disk natively
pub struct Factory {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::PipelineLayout,
    cache: Arc<Mutex<HashMap<(String, bool), wgpu::RenderPipeline>>>,
    images: Arc<Mutex<HashMap<PathBuf, (web_time::SystemTime, wgpu::TextureView)>>>,
}

impl Factory {
    fn pipeline(&self, src: &str, hdr: bool) -> Result<wgpu::RenderPipeline, Diag> {
        let key = (src.to_string(), hdr);
        if let Some(p) = self.cache.lock().unwrap().get(&key) {
            return Ok(p.clone());
        }
        validate(src).map_err(|m| Diag {
            span: None,
            message: format!("internal shader error: {m}"),
        })?;
        // Native: catch GPU-side errors synchronously. In the browser they arrive
        // asynchronously (naga has already validated the shader above).
        #[cfg(not(target_arch = "wasm32"))]
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        #[cfg(not(target_arch = "wasm32"))]
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("monet"),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("monet"),
                layout: Some(&self.layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(chain::codegen::VS_ENTRY),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(chain::codegen::FS_ENTRY),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: if hdr { HDR } else { FORMAT },
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        #[cfg(not(target_arch = "wasm32"))]
        {
            let errs = [
                pollster::block_on(internal.pop()),
                pollster::block_on(validation.pop()),
            ];
            if let Some(e) = errs.into_iter().flatten().next() {
                return Err(Diag {
                    span: None,
                    message: format!("gpu: {e}"),
                });
            }
        }
        let mut cache = self.cache.lock().unwrap();
        if cache.len() > 256 {
            cache.clear();
        }
        cache.insert(key, pipeline.clone());
        Ok(pipeline)
    }

    #[cfg(target_arch = "wasm32")]
    fn image(&self, path: &Path) -> Result<wgpu::TextureView, String> {
        Err(format!(
            "{}: image() reads files, so it needs the desktop app",
            path.display()
        ))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn image(&self, path: &Path) -> Result<wgpu::TextureView, String> {
        let modified = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if let Some((m, v)) = self.images.lock().unwrap().get(path)
            && *m == modified
        {
            return Ok(v.clone());
        }
        let img = image::open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .to_rgba8();
        let (w, h) = img.dimensions();
        let size = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            tex.as_image_copy(),
            &img,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            size,
        );
        let view = tex.create_view(&Default::default());
        self.images
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), (modified, view.clone()));
        Ok(view)
    }

    /// Turn a graph into pipelines (and load its images relative to `dir`).
    pub fn build(&self, graph: &Graph, dir: &Path) -> Result<GpuGraph, Diag> {
        // Passes are independent: build them in parallel (cold Metal compiles
        // cost tens of ms each; cached ones return instantly).
        let sources: Vec<(String, bool)> = graph
            .passes
            .iter()
            .map(|p| (p.source(), p.output != TexRef::Screen))
            .collect();
        #[cfg(not(target_arch = "wasm32"))]
        let pipelines: Vec<wgpu::RenderPipeline> = std::thread::scope(|scope| {
            let handles: Vec<_> = sources
                .iter()
                .map(|(src, hdr)| scope.spawn(move || self.pipeline(src, *hdr)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(Diag {
                            span: None,
                            message: "shader compiler panicked".into(),
                        })
                    })
                })
                .collect::<Result<Vec<_>, Diag>>()
        })?;
        // The browser compiles pipelines in its GPU process: creating them returns at once.
        #[cfg(target_arch = "wasm32")]
        let pipelines: Vec<wgpu::RenderPipeline> = sources
            .iter()
            .map(|(src, hdr)| self.pipeline(src, *hdr))
            .collect::<Result<Vec<_>, Diag>>()?;
        let mut images = Vec::new();
        for path in &graph.images {
            let full = dir.join(path);
            images.push(self.image(&full).map_err(|m| Diag {
                span: None,
                message: m,
            })?);
        }
        let reads = |pat: &[&str]| {
            sources
                .iter()
                .any(|(src, _)| pat.iter().any(|p| src.contains(p)))
        };
        let reads_time = graph.hists > 0 || reads(&["u.time", "u.cycle", "u.frame", "u.dt"]);
        let reads_mouse = reads(&["u.mouse"]);
        Ok(GpuGraph {
            desc: graph.clone(),
            pipelines,
            images,
            reads_time,
            reads_mouse,
        })
    }
}

pub struct Compiled {
    pub generation: u64,
    pub result: Result<GpuGraph, Diag>,
    pub millis: f32,
}

#[cfg(not(target_arch = "wasm32"))]
struct Job {
    generation: u64,
    graph: Graph,
    dir: PathBuf,
}

/// Background compiler. `submit` never blocks; results arrive via `poll`.
#[cfg(not(target_arch = "wasm32"))]
pub struct Compiler {
    jobs: Sender<Job>,
    done: Receiver<Compiled>,
    generation: u64,
}

/// In the browser there are no threads to spare, and none needed: WebGPU hands
/// pipeline compilation to the GPU process, so `submit` builds at once and the
/// result is picked up by the next `poll`.
#[cfg(target_arch = "wasm32")]
pub struct Compiler {
    factory: Factory,
    done: std::cell::RefCell<Option<Compiled>>,
    wake: Arc<dyn Fn() + Send + Sync>,
    generation: u64,
}

#[cfg(target_arch = "wasm32")]
impl Compiler {
    pub fn spawn(factory: Factory, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            factory,
            done: Default::default(),
            wake,
            generation: 0,
        }
    }

    pub fn submit(&mut self, graph: &Graph, dir: &Path) -> u64 {
        self.generation += 1;
        let t = Instant::now();
        let result = self.factory.build(graph, dir);
        *self.done.borrow_mut() = Some(Compiled {
            generation: self.generation,
            result,
            millis: t.elapsed().as_secs_f32() * 1000.0,
        });
        (self.wake)();
        self.generation
    }

    pub fn poll(&self) -> Option<Compiled> {
        self.done.borrow_mut().take()
    }

    pub fn latest_generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
struct Job;

#[cfg(not(target_arch = "wasm32"))]
impl Compiler {
    pub fn spawn(factory: Factory, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (done_tx, done) = channel();
        std::thread::Builder::new()
            .name("monet-compiler".into())
            .spawn(move || {
                while let Ok(mut job) = job_rx.recv() {
                    while let Ok(newer) = job_rx.try_recv() {
                        job = newer;
                    }
                    let t = Instant::now();
                    let result = factory.build(&job.graph, &job.dir);
                    let millis = t.elapsed().as_secs_f32() * 1000.0;
                    if done_tx
                        .send(Compiled {
                            generation: job.generation,
                            result,
                            millis,
                        })
                        .is_err()
                    {
                        break;
                    }
                    wake();
                }
            })
            .expect("spawn compiler thread");
        Self {
            jobs,
            done,
            generation: 0,
        }
    }

    pub fn submit(&mut self, graph: &Graph, dir: &Path) -> u64 {
        self.generation += 1;
        let _ = self.jobs.send(Job {
            generation: self.generation,
            graph: graph.clone(),
            dir: dir.to_path_buf(),
        });
        self.generation
    }

    pub fn poll(&self) -> Option<Compiled> {
        self.done.try_iter().last()
    }

    pub fn latest_generation(&self) -> u64 {
        self.generation
    }
}

pub struct Target {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

fn texture(
    device: &wgpu::Device,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

impl Target {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = texture(device, width, height, FORMAT, "canvas");
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            width,
            height,
        }
    }
}

#[derive(Default)]
struct Pool {
    key: (u32, u32, Vec<u32>, usize),
    bufs: Vec<wgpu::TextureView>,
    hists: Vec<[wgpu::TextureView; 2]>,
}

impl Pool {
    /// Returns true when buffers were (re)allocated.
    fn ensure(&mut self, device: &wgpu::Device, w: u32, h: u32, g: &Graph) -> bool {
        let key = (w, h, g.bufs.clone(), g.hists);
        if self.key == key && !(self.bufs.is_empty() && !g.bufs.is_empty()) {
            return false;
        }
        let view = |div: u32, label: &str| {
            texture(device, w.div_ceil(div), h.div_ceil(div), HDR, label)
                .create_view(&Default::default())
        };
        self.bufs = g.bufs.iter().map(|d| view(*d, "buffer")).collect();
        self.hists = (0..g.hists)
            .map(|_| [view(1, "history"), view(1, "history")])
            .collect();
        self.key = key;
        true
    }
}

pub struct Engine {
    device: wgpu::Device,
    queue: wgpu::Queue,
    factory: Factory,
    bgl: wgpu::BindGroupLayout,
    ubuf: wgpu::Buffer,
    sampler: wgpu::Sampler,
    dummy: wgpu::TextureView,
    graph: Option<GpuGraph>,
    pool: Pool,
    target: Option<Target>,
    parity: usize,
    /// Bind groups per pass and feedback parity; rebuilt only when the graph
    /// or the buffer pool changes (not every frame).
    bind_groups: Vec<[Option<wgpu::BindGroup>; 2]>,
    /// The inputs of the last frame drawn (only the ones the graph reads).
    last_key: Option<Vec<u8>>,
    /// Skip frames whose inputs haven't changed (off for benchmarks).
    pub skip_unchanged: bool,
}

impl Engine {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let mut entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ];
        for i in 0..chain::codegen::MAX_INPUTS as u32 {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2 + i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("monet"),
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("monet"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let dummy = texture(&device, 1, 1, HDR, "dummy").create_view(&Default::default());
        let factory = Factory {
            device: device.clone(),
            queue: queue.clone(),
            layout,
            cache: Default::default(),
            images: Default::default(),
        };
        Self {
            device,
            queue,
            factory,
            bgl,
            ubuf,
            sampler,
            dummy,
            graph: None,
            pool: Pool::default(),
            target: None,
            parity: 0,
            bind_groups: Vec::new(),
            last_key: None,
            skip_unchanged: true,
        }
    }

    /// Headless engine on the default adapter (CLI renders, tests).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn headless() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
            .map_err(|e| format!("no GPU device: {e}"))?;
        Ok(Self::new(device, queue))
    }

    pub fn factory(&self) -> Factory {
        self.factory.clone()
    }

    pub fn set_graph(&mut self, g: GpuGraph) {
        self.graph = Some(g);
        self.bind_groups.clear();
        self.last_key = None;
    }

    /// True when the current graph ignores the clock: a still picture.
    pub fn is_still(&self) -> bool {
        self.graph.as_ref().is_some_and(|g| !g.reads_time)
    }

    /// GPU milliseconds per pass, measured in isolation: after a full frame
    /// (so every buffer holds real data), each pass is re-run `reps` times in
    /// one submission and timed wall-clock. Unlike per-pass timestamps, this
    /// stays honest on tile-based GPUs, which overlap and merge passes.
    pub fn time_passes(&mut self, reps: u32) -> Vec<(String, f64)> {
        let Some(g) = &self.graph else {
            return Vec::new();
        };
        let Some(t) = &self.target else {
            return Vec::new();
        };
        let out = &t.view;
        let cur = 1 - self.parity; // the parity the last frame drew with
        let mut res = Vec::new();
        for (i, (pass, pipeline)) in g.desc.passes.iter().zip(&g.pipelines).enumerate() {
            let uses_history = pass
                .inputs
                .iter()
                .any(|r| matches!(r, TexRef::HistPrev(_) | TexRef::HistNext(_)));
            let slot = if uses_history && g.desc.hists > 0 {
                cur
            } else {
                0
            };
            let Some(bg) = self
                .bind_groups
                .get(i)
                .and_then(|b| b[slot].as_ref().or(b[1 - slot].as_ref()))
            else {
                continue;
            };
            let target = match pass.output {
                TexRef::Buf(k) => &self.pool.bufs[k],
                TexRef::HistNext(k) => &self.pool.hists[k][cur],
                TexRef::HistPrev(k) => &self.pool.hists[k][1 - cur],
                TexRef::Image(k) => &g.images[k],
                TexRef::Screen => out,
            };
            let run = |n: u32| {
                let mut enc = self.device.create_command_encoder(&Default::default());
                for _ in 0..n {
                    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: target,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    rp.set_pipeline(pipeline);
                    rp.set_bind_group(0, bg, &[]);
                    rp.draw(0..3, 0..1);
                }
                let t = Instant::now();
                self.queue.submit([enc.finish()]);
                let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
                t.elapsed().as_secs_f64() * 1e3
            };
            run(2); // warm
            // subtract the fixed cost of a submission: time 1 rep and `reps` reps
            let one = run(1);
            let many = run(reps.max(2));
            res.push((
                pass.label.clone(),
                ((many - one) / (reps.max(2) - 1) as f64).max(0.0),
            ));
        }
        res
    }

    pub fn graph(&self) -> Option<&Graph> {
        self.graph.as_ref().map(|g| &g.desc)
    }

    pub fn has_graph(&self) -> bool {
        self.graph.is_some()
    }

    pub fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }

    /// Make sure the canvas is `w × h`. Returns true if it was (re)allocated.
    pub fn ensure_target(&mut self, w: u32, h: u32) -> bool {
        let (w, h) = (w.max(1), h.max(1));
        if self
            .target
            .as_ref()
            .is_some_and(|t| t.width == w && t.height == h)
        {
            return false;
        }
        self.target = Some(Target::new(&self.device, w, h));
        self.last_key = None;
        true
    }

    /// Forget feedback history (e.g. after a restart).
    pub fn reset_history(&mut self) {
        self.pool = Pool::default();
        self.bind_groups.clear();
        self.last_key = None;
    }

    /// Draw one frame into the canvas. Returns false when the frame was
    /// skipped because nothing the graph reads has changed — a still fractal
    /// costs nothing while you look at it.
    pub fn render(&mut self, u: &Uniforms, f: &Frame) -> bool {
        let Some(g) = &self.graph else { return false };
        let Some(t) = &self.target else { return false };
        let mut uk = *u;
        uk.res = [t.width as f32, t.height as f32];
        if !g.reads_time {
            (uk.time, uk.dt, uk.frame, uk.cycle) = (0.0, 0.0, 0.0, 0.0);
        }
        if !g.reads_mouse {
            uk.mouse = [0.0, 0.0];
        }
        let key = uk.to_bytes(f);
        if self.skip_unchanged && self.last_key.as_ref() == Some(&key) {
            return false;
        }
        let t = self.target.take().expect("target");
        self.draw(&t.view, t.width, t.height, u, f);
        self.target = Some(t);
        self.last_key = Some(key);
        true
    }

    fn draw(&mut self, out: &wgpu::TextureView, w: u32, h: u32, u: &Uniforms, f: &Frame) {
        let Some(g) = &self.graph else { return };
        let mut u = *u;
        u.res = [w as f32, h as f32];
        self.queue.write_buffer(&self.ubuf, 0, &u.to_bytes(f));
        if self.pool.ensure(&self.device, w, h, &g.desc)
            || self.bind_groups.len() != g.desc.passes.len()
        {
            self.bind_groups = vec![[None, None]; g.desc.passes.len()];
        }
        let (cur, prev) = (self.parity, 1 - self.parity);
        let mut enc = self.device.create_command_encoder(&Default::default());
        for (i, (pass, pipeline)) in g.desc.passes.iter().zip(&g.pipelines).enumerate() {
            // Inputs never include the screen, so a pass's bind group depends
            // only on the pool and (for feedback) the frame parity.
            let uses_history = pass
                .inputs
                .iter()
                .any(|r| matches!(r, TexRef::HistPrev(_) | TexRef::HistNext(_)));
            let slot = if uses_history { cur } else { 0 };
            if self.bind_groups[i][slot].is_none() {
                let view_of = |r: TexRef| -> &wgpu::TextureView {
                    match r {
                        TexRef::Buf(k) => &self.pool.bufs[k],
                        TexRef::HistNext(k) => &self.pool.hists[k][cur],
                        TexRef::HistPrev(k) => &self.pool.hists[k][prev],
                        TexRef::Image(k) => &g.images[k],
                        TexRef::Screen => &self.dummy,
                    }
                };
                let mut entries = vec![
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.ubuf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ];
                for k in 0..chain::codegen::MAX_INPUTS {
                    let v = pass.inputs.get(k).map_or(&self.dummy, |r| view_of(*r));
                    entries.push(wgpu::BindGroupEntry {
                        binding: 2 + k as u32,
                        resource: wgpu::BindingResource::TextureView(v),
                    });
                }
                self.bind_groups[i][slot] =
                    Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &self.bgl,
                        entries: &entries,
                    }));
            }
            let target = match pass.output {
                TexRef::Buf(k) => &self.pool.bufs[k],
                TexRef::HistNext(k) => &self.pool.hists[k][cur],
                TexRef::HistPrev(k) => &self.pool.hists[k][prev],
                TexRef::Image(k) => &g.images[k],
                TexRef::Screen => out,
            };
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&pass.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, self.bind_groups[i][slot].as_ref(), &[]);
            rp.draw(0..3, 0..1);
        }
        let hists = g.desc.hists;
        self.queue.submit([enc.finish()]);
        if hists > 0 {
            self.parity = prev;
        }
    }

    /// Block until the GPU has finished all submitted work.
    pub fn wait(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    /// Render `frames` frames at an arbitrary size (the last at `u`, earlier
    /// ones stepping back by `u.dt` so feedback effects settle) and read back.
    pub fn snapshot(
        &mut self,
        w: u32,
        h: u32,
        u: &Uniforms,
        frame_at: impl Fn(&Uniforms) -> Frame,
        frames: u32,
    ) -> Option<Image> {
        self.graph.as_ref()?;
        let t = Target::new(&self.device, w, h);
        let saved = std::mem::take(&mut self.pool);
        self.bind_groups.clear();
        for k in (0..frames.max(1)).rev() {
            let mut uk = *u;
            uk.time -= k as f32 * u.dt;
            uk.cycle -= k as f32 * u.dt / 2.0;
            uk.frame -= k as f32;
            let f = frame_at(&uk);
            self.draw(&t.view, w, h, &uk, &f);
        }
        let img = self.read(&t);
        self.pool = saved;
        self.bind_groups.clear();
        Some(img)
    }

    pub fn read_target(&self) -> Option<Image> {
        self.target.as_ref().map(|t| self.read(t))
    }

    /// The canvas, read back without blocking (the browser can't wait on the
    /// GPU): `done` runs when the copy lands. False if there's nothing drawn yet.
    pub fn read_target_then(&self, done: impl FnOnce(Image) + Send + 'static) -> bool {
        let Some(t) = self.target.as_ref() else {
            return false;
        };
        let (w, h) = (t.width, t.height);
        let row = w * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded as u64 * h as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            t.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let keep = buf.clone();
        buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            if r.is_err() {
                return;
            }
            let rgba = {
                let Ok(data) = keep.slice(..).get_mapped_range() else { return };
                let mut px = Vec::with_capacity((row * h) as usize);
                for y in 0..h as usize {
                    let start = y * padded as usize;
                    px.extend_from_slice(&data[start..start + row as usize]);
                }
                px
            };
            keep.unmap();
            done(Image {
                width: w,
                height: h,
                rgba,
            });
        });
        // natively, nudge the device so the callback fires; the browser fires it itself
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.device.poll(wgpu::PollType::Poll);
        true
    }

    fn read(&self, t: &Target) -> Image {
        let row = t.width * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded as u64 * t.height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            t.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(t.height),
                },
            },
            wgpu::Extent3d {
                width: t.width,
                height: t.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().expect("map readback buffer");
        let mut px = Vec::with_capacity((row * t.height) as usize);
        for y in 0..t.height as usize {
            let start = y * padded as usize;
            px.extend_from_slice(&data[start..start + row as usize]);
        }
        Image {
            width: t.width,
            height: t.height,
            rgba: px,
        }
    }
}

/// A CPU-side RGBA8 image.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// Nearest-neighbour upscale (keeps pixel art crisp).
    pub fn upscale(&self, k: u32) -> Image {
        if k <= 1 {
            return Image {
                width: self.width,
                height: self.height,
                rgba: self.rgba.clone(),
            };
        }
        let (w, h) = (self.width * k, self.height * k);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let src_row =
                &self.rgba[((y / k) * self.width * 4) as usize..][..(self.width * 4) as usize];
            for px in src_row.as_chunks::<4>().0 {
                for _ in 0..k {
                    rgba.extend_from_slice(px);
                }
            }
        }
        Image {
            width: w,
            height: h,
            rgba,
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// The image as PNG bytes.
    pub fn png_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        if let Ok(mut w) = enc.write_header() {
            let _ = w.write_image_data(&self.rgba);
        }
        out
    }

    pub fn save_png(&self, path: &std::path::Path) -> std::io::Result<()> {
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut enc = png::Encoder::new(file, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut w = enc.write_header().map_err(std::io::Error::other)?;
        w.write_image_data(&self.rgba)
            .map_err(std::io::Error::other)?;
        Ok(())
    }
}

impl Engine {
    /// Compile `src` and render one still at `seconds` (wall-clock seconds at
    /// the sketch's speed). Feedback effects get a second of warm-up frames.
    pub fn still(
        &mut self,
        src: &str,
        dir: &Path,
        w: u32,
        h: u32,
        seconds: f64,
        scale: u32,
    ) -> Result<Image, Diag> {
        let program = chain::compile(src)?;
        let g = self.factory.build(&program.graph, dir)?;
        self.set_graph(g);
        let scale = scale.max(1);
        let (rw, rh) = (w.div_ceil(scale), h.div_ceil(scale));
        let cycle = seconds * program.speed;
        let home = program.home.unwrap_or([0.0, 0.0, 1.0]);
        let u = Uniforms {
            res: [rw as f32, rh as f32],
            mouse: [0.5, 0.5],
            time: (cycle * 2.0) as f32,
            dt: 1.0 / 60.0,
            frame: (seconds * 60.0) as f32,
            cycle: cycle as f32,
            view: [home[0], home[1], home[2], 0.0],
        };
        let frames = if program.graph.hists > 0 { 60 } else { 1 };
        let frame_at = |u: &Uniforms| {
            program.frame(&chain::Clock {
                cycle: u.cycle as f64,
                mouse: u.mouse,
            })
        };
        let img = self.snapshot(rw, rh, &u, frame_at, frames).ok_or(Diag {
            span: None,
            message: "nothing to render".into(),
        })?;
        Ok(img.upscale(scale))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_layout_matches_wgsl() {
        let u = Uniforms {
            res: [1.0, 2.0],
            mouse: [3.0, 4.0],
            time: 5.0,
            dt: 6.0,
            frame: 7.0,
            cycle: 8.0,
            view: [9.0, 10.0, 11.0, 12.0],
        };
        let mut f = Frame {
            scalars: [0.0; 256],
            colors: [[0.0; 4]; 128],
            active: vec![],
        };
        f.scalars[0] = 13.0;
        f.colors[0] = [14.0, 15.0, 16.0, 17.0];
        let b = u.to_bytes(&f);
        assert_eq!(b.len() as u64, UNIFORM_SIZE);
        let at = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
        for i in 0..13 {
            assert_eq!(at(i), (i + 1) as f32);
        }
        // c starts after 12 header floats + 256 scalars (offset 1072, 16-aligned)
        assert_eq!(at(12 + 256), 14.0);
        assert!(
            chain::codegen::LIBRARY.contains("s: array<vec4f, 64>,\n    c: array<vec4f, 128>,")
        );
    }

    #[test]
    fn tidy_strips_handles() {
        assert_eq!(
            tidy("Expression [12] is invalid [x]"),
            "Expression  is invalid [x]"
        );
    }
}
