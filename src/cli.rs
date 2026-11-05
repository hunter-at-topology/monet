use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use monet::chain;
use monet::engine::{Engine, Uniforms};
use monet::presets;

const HELP: &str = "\
monet — a tiny, fast live-coding environment for moving pictures

Sketches are chains: a picture, then effects.

    mesh(\"dawn\").flow(.3).dither(\"riviera\", 4)
    glints().blend(\"add\")              // each line is a layer

USAGE
  monet [FILE]                       open the editor (optionally on a file; it hot-reloads)
  monet check  <FILE|PRESET>         compile a sketch, print problems
  monet render <FILE|PRESET> [-o out.png] [--size 1600x1000] [--time 3.0] [--scale N]
               [--video out.mp4 --seconds 4 --fps 30 --rate 1]   an H.264 clip instead (needs ffmpeg)
  monet bench [WxH]                  time compiles and frames for every preset
  monet profile [all|NAME…] [--size WxH] [--frames N]
                                     GPU time per pass (timestamp queries), hottest first
  monet presets                      list the built-in sketches
  monet skill                        print the agent skill (teach Claude or any agent to write monet)

PRESETS
  monet:     lilies poppies sunrise rouen impression
  paintings: dawn riviera glints pool aurora silk lacquer atlas stained marble reeded
             waves bokeh pulse lens halftone ascii trails
  covers:    riso poster cobalt-sun vermilion riso-seahorse orbit ring swirl op
             atom starburst mosaic tunnel
  fractals:  mandelbrot seahorse dive morph julia kali tricorn newton
  · blank
";

pub fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-h" | "--help" | "help") => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Some("check") => cli(check(&args[1..])),
        Some("render") => cli(render(&args[1..])),
        Some("bench") => cli(bench(&args[1..])),
        Some("profile") => cli(profile(&args[1..])),
        Some("skill") => {
            print!("{}", monet::skill::text());
            ExitCode::SUCCESS
        }
        Some("presets") => {
            for p in presets::PRESETS {
                let first = p
                    .source
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim_start_matches("// ");
                println!("{:<12} {first}", p.name);
            }
            ExitCode::SUCCESS
        }
        Some(path) if path.starts_with('-') => cli(Err(format!("unknown flag {path}\n\n{HELP}"))),
        path => cli(monet::app::run(path.map(PathBuf::from))),
    }
}

fn cli(r: Result<(), String>) -> ExitCode {
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("monet: {e}");
            ExitCode::FAILURE
        }
    }
}

/// (source, directory images are relative to)
fn load(name: &str) -> Result<(String, PathBuf), String> {
    if let Some(p) = presets::find(name) {
        return Ok((p.source.to_string(), PathBuf::from(".")));
    }
    let src = std::fs::read_to_string(name).map_err(|e| format!("{name}: {e}"))?;
    let dir = Path::new(name)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    Ok((src, dir))
}

fn located(name: &str, src: &str, e: &chain::Error) -> String {
    let (l, c) = e.line_col(src);
    format!("{name}:{l}:{c}: {}", e.msg)
}

fn check(args: &[String]) -> Result<(), String> {
    let name = args.first().ok_or("check needs a file or preset")?;
    let (src, _) = load(name)?;
    let t = Instant::now();
    let p = chain::compile(&src).map_err(|e| located(name, &src, &e))?;
    let ms = t.elapsed().as_secs_f64() * 1e3;
    for pass in &p.graph.passes {
        monet::engine::validate(&pass.source())
            .map_err(|e| format!("{name}: internal shader error in {}: {e}", pass.label))?;
    }
    println!(
        "ok · {} layer{} · {} pass{} · {} value{} · {ms:.2} ms",
        p.layers,
        if p.layers == 1 { "" } else { "s" },
        p.graph.passes.len(),
        if p.graph.passes.len() == 1 { "" } else { "es" },
        p.scalars.len(),
        if p.scalars.len() == 1 { "" } else { "s" },
    );
    Ok(())
}

fn render(args: &[String]) -> Result<(), String> {
    let mut it = args.iter();
    let name = it.next().ok_or("render needs a file or preset")?;
    let (src, dir) = load(name)?;
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("monet");
    let mut out = PathBuf::from(format!("{stem}.png"));
    let (mut w, mut h, mut time, mut scale) = (1600u32, 1000u32, 3.0f64, 1u32);
    let (mut seconds, mut fps, mut video, mut rate): (f64, u32, Option<PathBuf>, f64) =
        (4.0, 30, None, 1.0);
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "-o" | "--out" => out = PathBuf::from(val()?),
            "--video" => video = Some(PathBuf::from(val()?)),
            "--seconds" => seconds = val()?.parse().map_err(|_| "bad --seconds")?,
            "--fps" => fps = val()?.parse().map_err(|_| "bad --fps")?,
            "--rate" => rate = val()?.parse().map_err(|_| "bad --rate")?,
            "--size" => {
                let v = val()?;
                let (a, b) = v.split_once('x').ok_or("--size wants WxH")?;
                w = a.parse().map_err(|_| "bad width")?;
                h = b.parse().map_err(|_| "bad height")?;
            }
            "--time" => time = val()?.parse().map_err(|_| "bad --time")?,
            "--scale" => scale = val()?.parse().map_err(|_| "bad --scale")?,
            other => return Err(format!("unknown option {other}")),
        }
    }
    chain::compile(&src).map_err(|e| located(name, &src, &e))?;
    if let Some(v) = video {
        return render_video(name, &src, &dir, &v, w, h, time, seconds, fps, rate);
    }
    let mut engine = Engine::headless()?;
    let img = engine
        .still(&src, &dir, w, h, time, scale)
        .map_err(|d| format!("{name}: {d}"))?;
    img.save_png(&out)
        .map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote {} ({}×{})", out.display(), img.width, img.height);
    Ok(())
}

fn bench(args: &[String]) -> Result<(), String> {
    let (w, h) = match args.first() {
        Some(s) => {
            let (a, b) = s.split_once('x').ok_or("bench size wants WxH")?;
            (
                a.parse().map_err(|_| "bad width")?,
                b.parse().map_err(|_| "bad height")?,
            )
        }
        None => (1920u32, 1080u32),
    };
    let mut engine = Engine::headless()?;
    engine.skip_unchanged = false;
    engine.ensure_target(w, h);
    println!(
        "{w}×{h} · compile = parse+eval+codegen · pipelines = naga + GPU (cold) · frame = GPU time\n"
    );
    println!(
        "{:<12} {:>8} {:>7} {:>11} {:>9}",
        "sketch", "compile", "passes", "pipelines", "frame"
    );
    for p in presets::PRESETS {
        let t = Instant::now();
        let prog = chain::compile(p.source).map_err(|e| format!("{}: {}", p.name, e.msg))?;
        let compile = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        let g = engine
            .factory()
            .build(&prog.graph, Path::new("."))
            .map_err(|d| format!("{}: {d}", p.name))?;
        let pipes = t.elapsed().as_secs_f64() * 1e3;
        engine.set_graph(g);
        let mut u = Uniforms {
            res: [w as f32, h as f32],
            ..Default::default()
        };
        if let Some(v) = prog.home {
            u.view = [v[0], v[1], v[2], 0.0];
        }
        let f = prog.frame(&chain::Clock::default());
        for _ in 0..5 {
            engine.render(&u, &f);
        }
        engine.wait();
        let n = 60;
        let t = Instant::now();
        for i in 0..n {
            u.time = i as f32 / 60.0;
            engine.render(&u, &f);
        }
        engine.wait();
        let frame = t.elapsed().as_secs_f64() * 1e3 / n as f64;
        println!(
            "{:<12} {:>6.3}ms {:>7} {:>9.1}ms {:>7.2}ms",
            p.name,
            compile,
            prog.graph.passes.len(),
            pipes,
            frame
        );
    }
    Ok(())
}

fn profile(args: &[String]) -> Result<(), String> {
    let (mut w, mut h, mut frames) = (1920u32, 1080u32, 40usize);
    let mut names: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (a, b) = v.split_once('x').ok_or("--size wants WxH")?;
                w = a.parse().map_err(|_| "bad width")?;
                h = b.parse().map_err(|_| "bad height")?;
            }
            "--frames" => {
                frames = it
                    .next()
                    .ok_or("--frames needs N")?
                    .parse()
                    .map_err(|_| "bad --frames")?
            }
            "all" => names.extend(presets::PRESETS.iter().map(|p| p.name.to_string())),
            n => names.push(n.to_string()),
        }
    }
    if names.is_empty() {
        names.extend(presets::PRESETS.iter().map(|p| p.name.to_string()));
    }
    let mut engine = Engine::headless()?;
    engine.skip_unchanged = false; // measure every frame, even still ones
    engine.ensure_target(w, h);
    let detail = names.len() == 1;
    println!("{w}×{h} · GPU ms: whole frame (mean of {frames}) and each pass in isolation\n");
    let mut rows: Vec<(String, f64, String, f64)> = Vec::new();
    for name in &names {
        let (src, dir) = load(name)?;
        let prog = chain::compile(&src).map_err(|e| located(name, &src, &e))?;
        let g = engine
            .factory()
            .build(&prog.graph, &dir)
            .map_err(|d| format!("{name}: {d}"))?;
        engine.set_graph(g);
        let home = prog.home.unwrap_or([0.0, 0.0, 1.0]);
        let frame_at = |i: usize| {
            let cycle = 3.0 + i as f64 / 60.0 * prog.speed;
            let u = Uniforms {
                res: [w as f32, h as f32],
                mouse: [0.5, 0.5],
                time: (cycle * 2.0) as f32,
                dt: 1.0 / 60.0,
                frame: i as f32,
                cycle: cycle as f32,
                view: [home[0], home[1], home[2], 0.0],
            };
            (
                u,
                prog.frame(&chain::Clock {
                    cycle,
                    mouse: [0.5, 0.5],
                }),
            )
        };
        for i in 0..5 {
            let (u, f) = frame_at(i);
            engine.render(&u, &f);
        }
        engine.wait();
        let t0 = std::time::Instant::now();
        for i in 0..frames {
            let (u, f) = frame_at(5 + i);
            engine.render(&u, &f);
        }
        engine.wait();
        let frame_ms = t0.elapsed().as_secs_f64() * 1e3 / frames as f64;
        let mut sums = engine.time_passes(12);
        let total: f64 = sums.iter().map(|s| s.1).sum::<f64>().max(1e-9);
        sums.sort_by(|a, b| b.1.total_cmp(&a.1));
        if detail {
            for (label, ms) in &sums {
                let bar = "█".repeat(((ms / total.max(1e-9)) * 30.0).round() as usize);
                println!(
                    "{:>8.3} ms  {:>5.1}%  {:<34} {bar}",
                    ms,
                    ms / total * 100.0,
                    label
                );
            }
            println!("{:>8.3} ms  sum of passes ({})", total, sums.len());
            println!("{:>8.3} ms  whole frame", frame_ms);
        }
        let (hot, hot_ms) = sums.first().cloned().unwrap_or_default();
        rows.push((name.clone(), frame_ms, hot, hot_ms));
    }
    if !detail {
        rows.sort_by(|a, b| b.1.total_cmp(&a.1));
        println!("{:<12} {:>9}   hottest pass", "sketch", "frame");
        for (n, t, hot, ms) in &rows {
            println!("{:<12} {:>7.3}ms   {hot} ({ms:.3}ms)", n, t);
        }
        let sum: f64 = rows.iter().map(|r| r.1).sum();
        println!("\n{:<12} {:>7.3}ms   ({} sketches)", "all", sum, rows.len());
    }
    Ok(())
}

/// Render `seconds` of a sketch, starting at `start`, straight into ffmpeg
/// (H.264, yuv420p). The mouse traces a slow loop so interactive sketches
/// (lens, pool, glints) come alive; feedback effects get a second of warm-up.
#[allow(clippy::too_many_arguments)]
fn render_video(
    name: &str,
    src: &str,
    dir: &Path,
    out: &Path,
    w: u32,
    h: u32,
    start: f64,
    seconds: f64,
    fps: u32,
    rate: f64,
) -> Result<(), String> {
    use std::io::Write;
    let prog = chain::compile(src).map_err(|e| located(name, src, &e))?;
    let mut engine = Engine::headless()?;
    engine.skip_unchanged = false;
    let g = engine
        .factory()
        .build(&prog.graph, dir)
        .map_err(|d| format!("{name}: {d}"))?;
    engine.set_graph(g);
    engine.ensure_target(w, h);
    let home = prog.home.unwrap_or([0.0, 0.0, 1.0]);
    let mut ff = std::process::Command::new("ffmpeg")
        .args([
            "-loglevel",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{w}x{h}"),
            "-r",
            &fps.to_string(),
            "-i",
            "-",
        ])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "slow",
            "-crf",
            "14",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
        ])
        .arg(out)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e} (is ffmpeg installed?)"))?;
    let stdin = ff.stdin.as_mut().ok_or("ffmpeg stdin")?;
    let warm = if prog.graph.hists > 0 { fps as i64 } else { 0 };
    let total = (seconds * fps as f64).round() as i64;
    for i in -warm..total {
        let secs = start + i as f64 / fps as f64 * rate;
        let cycle = secs * prog.speed;
        let a = secs * 0.6;
        let mouse = [
            0.5 + 0.22 * a.cos() as f32,
            0.5 + 0.18 * (a * 1.3).sin() as f32,
        ];
        let u = Uniforms {
            res: [w as f32, h as f32],
            mouse,
            time: (cycle * 2.0) as f32,
            dt: 1.0 / fps as f32,
            frame: (secs * fps as f64) as f32,
            cycle: cycle as f32,
            view: [home[0], home[1], home[2], 0.0],
        };
        let f = prog.frame(&chain::Clock { cycle, mouse });
        engine.render(&u, &f);
        if i >= 0 {
            let img = engine.read_target().ok_or("no frame")?;
            stdin
                .write_all(&img.rgba)
                .map_err(|e| format!("ffmpeg: {e}"))?;
        }
    }
    drop(ff.stdin.take());
    let status = ff.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("ffmpeg failed ({status})"));
    }
    println!("wrote {} ({w}×{h}, {seconds}s at {fps} fps)", out.display());
    Ok(())
}
