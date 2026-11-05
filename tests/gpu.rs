//! End-to-end tests on a real GPU. Skipped (with a note) when none is available.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use monet::chain;
use monet::engine::{Compiler, Engine, Image};
use monet::presets;

fn engine() -> Option<Engine> {
    match Engine::headless() {
        Ok(e) => Some(e),
        Err(e) => {
            eprintln!("skipping GPU test: {e}");
            None
        }
    }
}

fn still(e: &mut Engine, src: &str, seconds: f64) -> Image {
    e.still(src, Path::new("."), 64, 40, seconds, 1)
        .unwrap_or_else(|d| panic!("{src}: {d}"))
}

fn centre(img: &Image) -> [u8; 4] {
    img.pixel(img.width / 2, img.height / 2)
}

fn close(a: [u8; 4], b: [u8; 3], tol: i32) -> bool {
    (0..3).all(|i| (a[i] as i32 - b[i] as i32).abs() <= tol)
}

#[test]
fn every_preset_renders_something_interesting() {
    let Some(mut e) = engine() else { return };
    for p in presets::PRESETS {
        let img = e
            .still(p.source, Path::new("."), 160, 100, 3.0, 1)
            .unwrap_or_else(|d| panic!("{}: {d}", p.name));
        let mut lo = [255u8; 3];
        let mut hi = [0u8; 3];
        for px in img.rgba.chunks(4) {
            for c in 0..3 {
                lo[c] = lo[c].min(px[c]);
                hi[c] = hi[c].max(px[c]);
            }
        }
        let spread = (0..3).map(|c| hi[c] as i32 - lo[c] as i32).max().unwrap();
        assert!(spread > 40, "{} looks flat (spread {spread})", p.name);
    }
}

#[test]
fn what_you_write_is_what_you_see() {
    let Some(mut e) = engine() else { return };
    let img = still(&mut e, "solid(\"#e9547d\")", 0.0);
    assert!(
        close(centre(&img), [0xe9, 0x54, 0x7d], 1),
        "{:?}",
        centre(&img)
    );
}

#[test]
fn gradients_go_up_at_90_degrees() {
    let Some(mut e) = engine() else { return };
    let img = still(&mut e, "gradient(\"#000000 #ffffff\", 90)", 0.0);
    let top = img.pixel(32, 2)[0];
    let bottom = img.pixel(32, 37)[0];
    assert!(top > 200 && bottom < 60, "top {top} bottom {bottom}");
}

#[test]
fn layers_blend_with_opacity() {
    let Some(mut e) = engine() else { return };
    let img = still(
        &mut e,
        "solid(\"#000000\")\nsolid(\"#ff0000\").opacity(.5)",
        0.0,
    );
    assert!(close(centre(&img), [128, 0, 0], 2), "{:?}", centre(&img));
    let img = still(
        &mut e,
        "solid(\"#404040\")\nsolid(\"#404040\").blend(\"add\")",
        0.0,
    );
    assert!(
        close(centre(&img), [128, 128, 128], 2),
        "{:?}",
        centre(&img)
    );
}

#[test]
fn patterns_change_over_time() {
    let Some(mut e) = engine() else { return };
    // speed 0.5: one cycle every 2 s
    let src = "solid(\"<#ff0000 #0000ff>\")";
    assert!(close(centre(&still(&mut e, src, 1.0)), [255, 0, 0], 1));
    assert!(close(centre(&still(&mut e, src, 3.0)), [0, 0, 255], 1));
}

#[test]
fn texture_effects_run() {
    let Some(mut e) = engine() else { return };
    // blur of a flat colour is the same colour
    let img = still(&mut e, "solid(\"#3366cc\").blur()", 0.0);
    assert!(
        close(centre(&img), [0x33, 0x66, 0xcc], 2),
        "{:?}",
        centre(&img)
    );
    // bloom only ever adds light
    let plain = centre(&still(&mut e, "solid(\"#f0f0f0\")", 0.0));
    let bloomed = centre(&still(&mut e, "solid(\"#f0f0f0\").bloom(1, .2)", 0.0));
    assert!(bloomed[0] >= plain[0]);
    // feedback settles to the input for a still picture
    let img = still(&mut e, "solid(\"#20a060\").trail(.8)", 1.0);
    assert!(
        close(centre(&img), [0x20, 0xa0, 0x60], 4),
        "{:?}",
        centre(&img)
    );
    for fx in ["chroma", "paint", "glow"] {
        still(&mut e, &format!("mesh().{fx}()"), 1.0);
    }
}

#[test]
fn images_load_relative_to_the_sketch() {
    let Some(mut e) = engine() else { return };
    let dir = std::env::temp_dir().join(format!("monet-img-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let img = Image {
        width: 2,
        height: 2,
        rgba: [10u8, 200, 90, 255].repeat(4),
    };
    img.save_png(&dir.join("dot.png")).unwrap();
    let out = e.still("image(\"dot.png\")", &dir, 32, 32, 0.0, 1).unwrap();
    assert!(close(centre(&out), [10, 200, 90], 2), "{:?}", centre(&out));
    let missing = e.still("image(\"nope.png\")", &dir, 8, 8, 0.0, 1);
    assert!(missing.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sketch_errors_are_diagnostics_not_crashes() {
    let Some(mut e) = engine() else { return };
    let d = e
        .still("mesh().flwo()", Path::new("."), 8, 8, 0.0, 1)
        .err()
        .unwrap();
    assert!(d.message.contains("Did you mean `flow`"));
    assert_eq!(d.span, Some(7..11));
}

#[test]
fn pipelines_are_cached_by_source() {
    let Some(e) = engine() else { return };
    let f = e.factory();
    let g = chain::compile("silk().bloom().grain()").unwrap().graph;
    let t = Instant::now();
    f.build(&g, Path::new(".")).unwrap();
    let cold = t.elapsed();
    let g2 = chain::compile("silk(\"ember\", 2).bloom(.1).grain(sine)")
        .unwrap()
        .graph;
    assert_eq!(g, g2, "values don't change the graph");
    let t = Instant::now();
    f.build(&g2, Path::new(".")).unwrap();
    let warm = t.elapsed();
    assert!(
        warm < cold / 4 || warm < Duration::from_millis(2),
        "cold {cold:?} warm {warm:?}"
    );
}

#[test]
fn compiler_is_latest_wins() {
    let Some(e) = engine() else { return };
    let mut c = Compiler::spawn(e.factory(), Arc::new(|| {}));
    let mut last = 0;
    for src in [
        "mesh()",
        "mesh().warp()",
        "mesh().warp().grain()",
        "clouds().dither(\"riviera\")",
    ] {
        last = c.submit(&chain::compile(src).unwrap().graph, Path::new("."));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(done) = c.poll()
            && done.generation == last
        {
            assert!(done.result.is_ok());
            break;
        }
        assert!(Instant::now() < deadline, "compiler never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn masks_by_picture() {
    let Some(mut e) = engine() else { return };
    let shown = still(
        &mut e,
        "solid(\"#00ff00\")\nsolid(\"#ff0000\").mask(solid(\"#ffffff\"))",
        0.0,
    );
    assert!(
        close(centre(&shown), [255, 0, 0], 1),
        "{:?}",
        centre(&shown)
    );
    let hidden = still(
        &mut e,
        "solid(\"#00ff00\")\nsolid(\"#ff0000\").mask(solid(\"#000000\"))",
        0.0,
    );
    assert!(
        close(centre(&hidden), [0, 255, 0], 1),
        "{:?}",
        centre(&hidden)
    );
}

#[test]
fn displacement_by_a_flat_picture_changes_nothing() {
    let Some(mut e) = engine() else { return };
    let a = still(&mut e, "gradient(\"#000000 #ffffff\", 0)", 0.0);
    let b = still(
        &mut e,
        "gradient(\"#000000 #ffffff\", 0).displace(solid(\"#808080\"), 1)",
        0.0,
    );
    assert!(close(
        centre(&a),
        [centre(&b)[0], centre(&b)[1], centre(&b)[2]],
        1
    ));
}

#[test]
fn brushwork_keeps_the_colour() {
    let Some(mut e) = engine() else { return };
    // strokes vary in tone a little (broken colour) but stay the same paint
    let img = e
        .still(
            "solid(\"#5d8a8c\").brush(1, .3, 0)",
            Path::new("."),
            128,
            80,
            1.0,
            1,
        )
        .unwrap();
    let n = (img.width * img.height) as f64;
    let mean: Vec<f64> = (0..3)
        .map(|c| img.rgba.chunks(4).map(|p| p[c] as f64).sum::<f64>() / n)
        .collect();
    let want = [0x5d as f64, 0x8a as f64, 0x8c as f64];
    for c in 0..3 {
        assert!(
            (mean[c] - want[c]).abs() < 16.0,
            "mean {mean:?} vs {want:?}"
        );
    }
    for p in ["lilies", "poppies", "sunrise"] {
        let src = presets::find(p).unwrap().source;
        e.still(src, Path::new("."), 96, 60, 2.0, 1)
            .unwrap_or_else(|d| panic!("{p}: {d}"));
    }
}

#[test]
fn still_pictures_skip_unchanged_frames() {
    use monet::engine::Uniforms;
    let Some(mut e) = engine() else { return };
    e.ensure_target(64, 40);
    let frame = |p: &chain::Program, t: f64| {
        p.frame(&chain::Clock {
            cycle: t,
            mouse: [0.5, 0.5],
        })
    };
    let at = |t: f32| Uniforms {
        time: t,
        cycle: t / 2.0,
        frame: t * 60.0,
        ..Default::default()
    };

    // A fractal reads no clock: the second identical frame is free…
    let still = chain::compile("mandelbrot()").unwrap();
    e.set_graph(e.factory().build(&still.graph, Path::new(".")).unwrap());
    assert!(e.is_still());
    assert!(e.render(&at(1.0), &frame(&still, 0.5)));
    assert!(
        !e.render(&at(2.0), &frame(&still, 1.0)),
        "time moved, but the fractal doesn't read it"
    );
    // …until something it reads changes (here: the view, i.e. a pan)
    let mut panned = at(3.0);
    panned.view = [0.1, 0.0, 1.0, 0.0];
    assert!(e.render(&panned, &frame(&still, 1.5)));

    // Anything animated always draws.
    let moving = chain::compile("mesh()").unwrap();
    e.set_graph(e.factory().build(&moving.graph, Path::new(".")).unwrap());
    assert!(!e.is_still());
    assert!(e.render(&at(1.0), &frame(&moving, 0.5)));
    assert!(e.render(&at(1.0 + 1.0 / 60.0), &frame(&moving, 0.51)));
}

#[test]
fn stipple_prints_only_its_inks_and_cards_cut_out() {
    let Some(mut e) = engine() else { return };
    let img = still(
        &mut e,
        "gradient(\"#000000 #ffffff\", 0).stipple(\"cobalt\")",
        0.0,
    );
    let inks = [[0x1b, 0x2f, 0xb2], [0xf1, 0xec, 0xe1]];
    for px in img.rgba.chunks(4) {
        assert!(
            inks.iter()
                .any(|k| close([px[0], px[1], px[2], 255], *k, 2)),
            "{px:?} isn't an ink"
        );
    }
    // left is dark → mostly ink, right is light → mostly paper
    let ink_share = |x0: u32, x1: u32| {
        let mut n = 0;
        let mut ink = 0;
        for y in 0..img.height {
            for x in x0..x1 {
                n += 1;
                ink += usize::from(close(img.pixel(x, y), inks[0], 2));
            }
        }
        ink as f64 / n as f64
    };
    assert!(ink_share(0, 10) > 0.75 && ink_share(54, 64) < 0.25);
    // a card shows its picture inside and what's beneath outside
    let card = still(
        &mut e,
        "solid(\"#000000\")\nsolid(\"#ff0000\").place(.25, .25, .5, .5, .05)",
        0.0,
    );
    assert!(close(centre(&card), [255, 0, 0], 1));
    assert!(close(card.pixel(2, 2), [0, 0, 0], 1));
}

#[test]
fn noise_is_centred_and_mono_means_grey() {
    let Some(mut e) = engine() else { return };
    let stats = |img: &Image| {
        let n = (img.rgba.len() / 4) as f64;
        let mut mean = [0f64; 3];
        let (mut spread, mut chroma) = (0f64, 0f64);
        for px in img.rgba.chunks(4) {
            for c in 0..3 {
                mean[c] += px[c] as f64 / n;
            }
            spread += (px[0] as f64 - 128.0).abs() / n;
            chroma += ((px[0] as i32 - px[1] as i32).abs() + (px[1] as i32 - px[2] as i32).abs())
                as f64
                / n;
        }
        (mean, spread, chroma)
    };
    for kind in ["gaussian", "uniform"] {
        let (mean, spread, chroma) = stats(&still(
            &mut e,
            &format!("solid(\"#808080\").noise(.3, 1, 1, 0, \"{kind}\")"),
            0.0,
        ));
        assert!(
            mean.iter().all(|m| (m - 128.0).abs() < 4.0),
            "{kind}: noise shifts the colour {mean:?}"
        );
        assert!(spread > 8.0, "{kind}: too faint ({spread})");
        assert!(
            chroma > 8.0,
            "{kind}: colour noise should be coloured ({chroma})"
        );
    }
    let (_, spread, chroma) = stats(&still(&mut e, "solid(\"#808080\").noise(.3, 0)", 0.0));
    assert!(
        spread > 8.0 && chroma < 1.5,
        "mono noise should be grey (spread {spread}, chroma {chroma})"
    );
    // bigger grain, same strength
    let (_, big, _) = stats(&still(&mut e, "solid(\"#808080\").noise(.3, 0, 4)", 0.0));
    assert!(
        (big / spread - 1.0).abs() < 0.35,
        "size 4 changes the strength: {big} vs {spread}"
    );
    // still noise doesn't change between frames; moving noise does
    let a = still(&mut e, "solid(\"#808080\").noise(.3)", 0.0);
    let b = still(&mut e, "solid(\"#808080\").noise(.3)", 1.0);
    assert_eq!(a.rgba, b.rgba);
    let c = still(&mut e, "solid(\"#808080\").noise(.3, 1, 1, 1)", 0.0);
    let d = still(&mut e, "solid(\"#808080\").noise(.3, 1, 1, 1)", 1.0);
    assert_ne!(c.rgba, d.rgba);
}

#[test]
fn sketch_shaders_draw_what_they_say() {
    let Some(mut e) = engine() else { return };
    // a picture: left half one palette colour, right half the other
    let src = "picture halves(colors = \"#ff0000 #0000ff\") {
    return vec4f(palc(colors, select(0u, 1u, uv.x > 0.5)), 1.0);
}
halves()";
    let img = still(&mut e, src, 0.0);
    assert!(close(img.pixel(8, 20), [255, 0, 0], 2), "{:?}", img.pixel(8, 20));
    assert!(close(img.pixel(56, 20), [0, 0, 255], 2), "{:?}", img.pixel(56, 20));
    // an effect that mirrors the picture so far: the left reads the right
    let mirrored = still(&mut e, &format!("{src}\n  .flip()\neffect flip() {{ return pic(vec2f(1.0 - uv.x, uv.y)); }}"), 0.0);
    assert!(close(mirrored.pixel(8, 20), [0, 0, 255], 2), "{:?}", mirrored.pixel(8, 20));
    // values reach the shader, and picture parameters draw their pictures
    let src = "effect dim(k = 1) { let c = pic(uv); return vec4f(c.rgb * k, c.a); }
effect over(top = solid(\"#00ff00\")) { return top(uv); }
solid(\"#ffffff\").dim(.5)
solid(\"#000000\").over().blend(\"add\")";
    let c = centre(&still(&mut e, src, 0.0));
    assert!(close(c, [127, 255, 127], 3), "{c:?}");
}
