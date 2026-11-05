use super::catalog::{Arg, Role, SPECS};
use super::*;

fn ok(src: &str) -> Program {
    match compile(src) {
        Ok(p) => p,
        Err(e) => panic!("{src}\n→ {} at {:?}", e.msg, e.span),
    }
}

fn err(src: &str) -> String {
    match compile(src) {
        Ok(_) => panic!("expected an error for {src}"),
        Err(e) => e.msg,
    }
}

fn validate_all(p: &Program) {
    for pass in &p.graph.passes {
        if let Err(e) = crate::engine::validate(&pass.source()) {
            panic!("{}:\n{e}\n{}", pass.label, pass.body);
        }
    }
}

#[test]
fn every_source_and_effect_generates_valid_wgsl() {
    for s in SPECS
        .iter()
        .filter(|s| s.role == Role::Source && s.name != "image" && !s.hidden)
    {
        validate_all(&ok(s.name));
    }
    for s in SPECS.iter().filter(|s| s.role != Role::Source && !s.hidden) {
        let p = ok(&format!("mesh().{}()", s.name));
        validate_all(&p);
    }
    validate_all(&ok("mesh().dither(\"riviera\", 4)"));
    validate_all(&ok("image(\"x.png\").paint()"));
    validate_all(&ok(""));
}

#[test]
fn defaults_fill_every_slot() {
    let p = ok("mesh()");
    assert_eq!(p.scalars.len(), 1);
    assert_eq!(p.palettes.len(), 1);
    let f = p.frame(&Clock::default());
    assert_eq!(f.scalars[0], 1.0);
    assert_eq!(f.colors[0][3], 5.0, "dawn has 5 stops");
}

#[test]
fn values_are_uniforms_not_code() {
    // Changing numbers, patterns and palettes leaves the shaders untouched.
    let a = ok("mesh(\"dawn\").flow(.3).grain(.05)");
    let b = ok("mesh(\"#000 #fff\").flow(\"0 .8\").grain(sine.range(0, .2))");
    assert_eq!(a.graph, b.graph);
    let c = ok("mesh(\"dawn\").warp(.3).grain(.05)");
    assert_ne!(a.graph, c.graph);
}

#[test]
fn patterns_drive_values_over_time() {
    let p = ok("mesh().zoom(\"1 2\")");
    let at = |c: f64| {
        p.frame(&Clock {
            cycle: c,
            mouse: [0.0; 2],
        })
    };
    let z = |c: f64| at(c).scalars[1];
    assert_eq!(z(0.25), 1.0);
    assert_eq!(z(0.9), 2.0);
    let f = at(0.9);
    assert_eq!(f.active.len(), 1);
    let sp = f.active[0].clone();
    assert_eq!(&"mesh().zoom(\"1 2\")"[sp], "2");
}

#[test]
fn signals_and_arithmetic() {
    let p = ok("x = sine.range(1, 3)\nmesh().zoom(x * 2)");
    let f = p.frame(&Clock {
        cycle: 0.25,
        mouse: [0.0; 2],
    });
    assert!((f.scalars[1] - 6.0).abs() < 1e-5);
    let p = ok("mesh().move(mouse.x, \"0 .5\".slow(2))");
    let f = p.frame(&Clock {
        cycle: 1.5,
        mouse: [0.3, 0.0],
    });
    assert!((f.scalars[1] - 0.3).abs() < 1e-6);
    assert!((f.scalars[2] - 0.5).abs() < 1e-6);
}

#[test]
fn palette_patterns_crossfade() {
    let p = ok("mesh(\"<dawn ember>\")");
    let f0 = p.frame(&Clock {
        cycle: 0.5,
        mouse: [0.0; 2],
    });
    let f1 = p.frame(&Clock {
        cycle: 1.6,
        mouse: [0.0; 2],
    });
    let mid = p.frame(&Clock {
        cycle: 1.05,
        mouse: [0.0; 2],
    });
    assert_eq!(f0.colors[0][3], 5.0);
    assert_eq!(f1.colors[0][3], 6.0);
    assert_eq!(mid.colors[0][3], 8.0, "mid-crossfade resamples to 8 stops");
    let p = ok("pal = \"#ff0000 #0000ff\"\nmesh(\"<pal dawn>\")");
    assert_eq!(p.frame(&Clock::default()).colors[0][3], 2.0);
}

#[test]
fn layers_and_blends() {
    let p = ok("water()\nglints().blend(\"add\")");
    assert_eq!(p.layers, 2);
    let last = p.graph.passes.last().unwrap();
    assert!(last.body.contains("blend_layer(acc, l1s0(uv), 1)"));
    let p = ok("stack(mesh(), stars().blend(\"screen\"))");
    assert_eq!(p.layers, 2);
}

#[test]
fn texture_effects_cut_passes() {
    let p = ok("mesh().bloom()");
    // segment → buffer, bright, blur ×2, composite
    assert_eq!(p.graph.passes.len(), 5);
    assert_eq!(p.graph.bufs, vec![1, 4, 4, 4]);
    let p = ok("mesh().trail()");
    assert_eq!(p.graph.hists, 1);
    let p = ok("mesh().warp().dither()");
    assert_eq!(
        p.graph.passes.len(),
        1,
        "domain + colour effects fuse into one pass"
    );
}

#[test]
fn fractals_have_a_home_view() {
    assert_eq!(ok("mandelbrot()").home, Some([-0.65, 0.0, 0.36]));
    assert_eq!(
        ok("view(1, 2, 3)\nmandelbrot()").home,
        Some([1.0, 2.0, 3.0])
    );
    assert_eq!(ok("mesh()").home, None);
}

#[test]
fn sliders_are_live_and_labelled() {
    let src = "amt = slider(0.3, 0, 1)\nmesh().flow(amt).grain(slider(.05))";
    let p = ok(src);
    assert_eq!(p.sliders.len(), 2);
    assert_eq!(p.sliders[0].label, "amt");
    assert_eq!(&src[p.sliders[0].span.clone()], "0.3");
    assert_eq!(p.sliders[1].label, "grain amount");
    assert_eq!(&src[p.sliders[1].span.clone()], ".05");
}

#[test]
fn speed_setting() {
    assert_eq!(ok("speed(1)\nmesh()").speed, 1.0);
    assert_eq!(ok("mesh()").speed, 0.5);
}

#[test]
fn friendly_errors() {
    assert!(err("mesh().flwo()").contains("Did you mean `flow`"));
    assert!(err("mesh(\"dwan\")").contains("Did you mean \"dawn\""));
    assert!(err("mesh().flow(1, 2, 3)").contains("flow(amount, speed)"));
    assert!(err("flow()").contains("is an effect"));
    assert!(err("mesh().mask(\"circel\")").contains("Did you mean \"circle\""));
    assert!(err("mesh().zoom(\"a b\")").contains("needs numbers"));
    assert!(err("sine").contains("Each line should be a picture"));
    assert!(err("mesh().stars()").contains("not an effect"));
    assert!(err("stack(mesh()).grain()").contains("each layer"));
    assert!(err("mesh = 1").contains("built-in"));
}

#[test]
fn error_positions() {
    let src = "mesh()\n  .flwo()";
    let e = compile(src).unwrap_err();
    assert_eq!(&src[e.span.clone()], "flwo");
    assert_eq!(e.line_col(src), (2, 4));
}

#[test]
fn every_arg_kind_is_covered_by_docs() {
    for s in SPECS {
        assert!(!s.doc.is_empty() || s.hidden, "{} needs a doc line", s.name);
        for (name, a) in s.args {
            assert!(!name.is_empty());
            if let Arg::Num(d) = a {
                assert!(d.is_finite());
            }
        }
    }
}

#[test]
fn pictures_as_arguments() {
    // a picture argument fuses into the same pass…
    let p = ok("mesh().displace(clouds(\"mono\", 3), .4)");
    assert_eq!(p.graph.passes.len(), 1);
    validate_all(&p);
    // …unless it needs textures itself, then its passes run first
    let p = ok("caustics().displace(spot(.2).trail(.97).blur(.5), .8)");
    assert_eq!(p.graph.hists, 1);
    assert!(p.graph.passes.len() >= 4);
    validate_all(&p);
    // defaults: displace bends by noise, mask by the mouse spot
    validate_all(&ok("mesh().displace()"));
    let p = ok("stripes().mask(spot(.3))");
    assert!(p.graph.passes[0].body.contains("col_mask_pic"));
    let p = ok("stripes().mask(\"ring\")");
    assert!(p.graph.passes[0].body.contains("col_mask("));
    assert!(err("mesh().displace(3)").contains("is a picture"));
}

#[test]
fn fractals_measure_their_own_pixels() {
    // AA and edge glow come from derivatives, so effects that move
    // coordinates first (dive, zoom, kaleido) keep fractals sharp.
    let p = ok("mandelbrot().dive(.5, 10).kaleido(6)");
    validate_all(&p);
    assert!(crate::chain::codegen::LIBRARY.contains("fn aa4("));
    assert!(
        !crate::chain::codegen::LIBRARY.contains("0.5 / u.res;\n        let c = plane(uv + o)")
    );
}

#[test]
fn painterly_and_cover_effects() {
    // brush and zoomblur are texture effects: picture → buffer → strokes
    let p = ok("lilies().brush(1, .3, 0).canvas()");
    assert_eq!(p.graph.passes.len(), 2);
    assert!(p.graph.passes[1].body.contains("brush("));
    validate_all(&p);
    let p = ok("poppies().shuffle(6, .3, .5).droste(.6, .1).zoomblur()");
    validate_all(&p);
    // scene palettes keep their documented stop order
    for (name, stops) in [("giverny", 8), ("poppy", 7), ("harbour", 6)] {
        assert_eq!(
            super::catalog::palette(name).unwrap().len(),
            stops,
            "{name}"
        );
    }
    // the shuffle seed is time: it changes by cycle, never by value edits
    let a = ok("mesh().shuffle(6, .3, .5)");
    let b = ok("mesh().shuffle(9, .7, 2)");
    assert_eq!(a.graph, b.graph);
}

#[test]
fn riso_and_cards() {
    let p = ok(
        "peonies().stipple(\"cobalt\")\nsun().place(.1, .4, .2, .2, .02).stipple(\"vermilion\")\ngrid(\"graph\").place(.1, .05, .2, .3, .02, 2)",
    );
    assert_eq!(p.layers, 3);
    assert_eq!(
        p.graph.passes.len(),
        1,
        "cards and stipple fuse into one pass"
    );
    let body = &p.graph.passes[0].body;
    assert!(
        body.contains("col_place(") && body.contains("dom_place(") && body.contains("col_stipple(")
    );
    validate_all(&p);
    assert!(
        err("mesh().place(1, 2, 3, 4, 5, 6, \"#fff\", 8)")
            .contains("place(x, y, w, h, radius, border, color)")
    );
}

#[test]
fn noise_is_the_effect_and_clouds_is_the_picture() {
    // the flowing-noise picture is clouds(); noise() alone says so
    validate_all(&ok("clouds(\"dawn\")"));
    assert!(err("noise(\"dawn\")").contains("clouds()"));
    // chained: Photoshop-style Add Noise, every option
    for src in [
        "mesh().noise()",
        "mesh().noise(.2, 0)",
        "mesh().noise(.2, 1, 3)",
        "mesh().noise(.2, .5, 2, 1)",
        "mesh().noise(.2, .5, 2, 0, \"uniform\")",
        "mesh().noise(.2, .5, 1, sine.range(0, 1), \"gaussian\")",
    ] {
        validate_all(&ok(src));
    }
    assert!(err("mesh().noise(.2, 1, 1, 0, \"pink\")").contains("gaussian"));
    // the written name in messages, never the internal one
    let e = err("mesh().noise(1, 2, 3, 4, \"uniform\", 6)");
    assert!(
        e.contains("`noise`") && e.contains("noise(amount, color, size, motion, kind)"),
        "{e}"
    );
}

#[test]
fn still_noise_has_no_clock_in_its_shader() {
    let reads_time = |src: &str| {
        let p = ok(src);
        p.graph
            .passes
            .iter()
            .any(|pass| pass.source().contains("u.time"))
    };
    assert!(!reads_time("solid(\"#808080\").noise(.2)"));
    assert!(!reads_time(
        "solid(\"#808080\").noise(.2, 1, 2, 0, \"uniform\")"
    ));
    assert!(reads_time("solid(\"#808080\").noise(.2, 1, 1, 1)"));
}

#[test]
fn signal_maths_on_a_picture_says_what_is_wrong() {
    let e = err("silk(\"candy\", .6)\n  .grain(.03)\n  .div(2)");
    assert!(e.contains("numbers and signals") && e.contains("brightness"), "{e}");
    assert!(!e.contains("Did you mean `div`"), "{e}");
    // and no suggestion is ever the word itself
    assert!(!err("mesh().blurr()").contains("`blurr`?"));
}

// ── shaders written in the sketch ────────────────────────────────────────────

const BREATHE: &str = "// rings that breathe
picture breathe(colors = \"dawn\", rings = 8, speed = .5) {
    let d = length(asp(uv));
    return vec4f(pal(colors, fract(d * rings - u.time * speed)), 1.0);
}
";

/// Where an error points, as (line, column, the text underlined).
fn err_at(src: &str) -> (usize, usize, String, String) {
    match compile(src) {
        Ok(_) => panic!("expected an error for\n{src}"),
        Err(e) => {
            let (l, c) = e.line_col(src);
            (l, c, src[e.span.clone()].to_string(), e.msg)
        }
    }
}

#[test]
fn sketch_pictures_compile_and_take_values_like_built_ins() {
    let p = ok(&format!("{BREATHE}breathe(\"ember\", sine.range(4, 12))\n  .grain()"));
    validate_all(&p);
    let body = &p.graph.passes.last().unwrap().body;
    assert!(body.contains("fn l0s0_f(uv: vec2f, colors: u32, rings: f32, speed: f32) -> vec4f"), "{body}");
    assert!(body.contains("return vec4f(pal(colors"), "the code as written");
    assert!(p.scalars.len() >= 2 && !p.palettes.is_empty());
    // bare, defaults, and before the definition
    validate_all(&ok(&format!("breathe\nbreathe()\n{BREATHE}")));
    // slider labels name the parameter
    let p = ok(&format!("{BREATHE}breathe(\"dawn\", slider(8, 1, 20))"));
    assert_eq!(p.sliders[0].label, "breathe rings");
}

#[test]
fn sketch_effects_read_the_picture_so_far() {
    let src = "effect wobble(amount = .02, rings = 40) {
    let p = asp(uv);
    return pic(uv + normalize(p + 1e-5) * sin(length(p) * rings - u.time) * amount);
}
effect fade(k = .5) { let c = pic(uv); return vec4f(c.rgb * k, c.a); }
mesh(\"dawn\").wobble().blur(2).fade(sine).wobble(.01)";
    let p = ok(src);
    validate_all(&p);
    let all: String = p.graph.passes.iter().map(|p| p.body.clone()).collect();
    assert!(all.contains("return l0s0(uv + normalize"), "pic → the stage before: {all}");
    assert!(all.contains("let c = l0s2(uv)"), "after a texture effect, pic reads it");
    assert!(!all.contains("pic("), "{all}");
}

#[test]
fn picture_parameters_are_functions() {
    let src = "effect through(other = clouds(\"mono\"), amount = .5) {
    let o = other(uv);
    return mix(pic(uv), o, amount);
}
picture duo(a = mesh(), b = spot(.3).blur()) { return mix(a(uv), b(uv), uv.x); }
w = stripes()
mesh().through()
mesh().through(rings().flow(), .2)
duo(w)
duo";
    validate_all(&ok(src));
    assert!(err("effect e(p = mesh()) { return p(uv); }\nmesh().e(3)").contains("is a picture"));
}

#[test]
fn helpers_are_shared_by_every_shader() {
    let src = "wgsl {
fn sd_box(p: vec2f, b: vec2f) -> f32 {
    let d = abs(p) - b;
    return length(max(d, vec2f(0.0))) + min(max(d.x, d.y), 0.0);
}
}
picture boxes(size = .2) {
    let d = sd_box(asp(uv), vec2f(size));
    return vec4f(vec3f(smoothstep(0.0, 0.01, d)), 1.0);
}
boxes().kaleido()";
    validate_all(&ok(src));
}

#[test]
fn sketch_shader_misuse_reads_well() {
    let e = |s: &str| err(&format!("{BREATHE}{s}"));
    assert!(e("mesh().breathe()").contains("is a picture, not an effect"));
    assert!(err("effect e() { return pic(uv); }\ne()").contains("Chain it onto a picture"));
    assert!(e("breathe(\"dawn\", 1, 2, 3)").contains("takes 3 values: breathe(colors, rings, speed)"));
    assert!(e("breathe(4)").contains("is a palette"));
    assert!(e("breathe(\"dawm\")").contains("Did you mean \"dawn\""));
    assert!(err("picture mesh() { return vec4f(1.0); }").contains("built-in name"));
    assert!(err("picture a() { return vec4f(1.0); }\npicture a() { return vec4f(0.0); }").contains("already defined"));
    assert!(err("picture a(uv) { return vec4f(1.0); }").contains("already given"));
    assert!(err("picture a(loop = 1) { return vec4f(1.0); }").contains("WGSL keyword"));
    assert!(err("picture a(x, x) { return vec4f(1.0); }").contains("already a parameter"));
    assert!(e("breathe = 1").contains("shader defined in this sketch"));
    assert!(e("mesh().breathx()").contains("Did you mean `breathe`"));
}

#[test]
fn wgsl_mistakes_are_underlined_where_they_are() {
    // a typo on the third line of the body
    let src = "mesh()\n// a\npicture p(k = 1) {\n    let a = k * 2.0;\n    let b = a +;\n    return vec4f(b);\n}\np()";
    let (line, _, text, msg) = err_at(src);
    assert_eq!(line, 5, "{msg}: {text}");
    assert!(msg.starts_with("WGSL:"), "{msg}");
    // an unknown name: the name itself
    let src = "picture p() {\n    return vec4f(vec3f(sin(time)), 1.0);\n}";
    let (line, col, text, msg) = err_at(src);
    assert_eq!((line, text.as_str()), (2, "time"), "{msg}");
    assert_eq!(col, 28);
    assert_eq!(msg, "`time` is u.time in a shader");
    let src = "picture p() { return vec4f(vec3f(fbn(uv, 4)), 1.0); }";
    assert_eq!(err_at(src).3, "unknown name `fbn`. Did you mean `fbm`?");
    // a type mistake: returning a vec3f
    let src = "picture p() {\n    let c = vec3f(1.0);\n    return c;\n}";
    let (line, _, text, msg) = err_at(src);
    assert_eq!((line, text.as_str()), (2, "vec3f(1.0)"), "{msg}");
    assert!(msg.contains("vec4f(rgb, 1.0)"), "{msg}");
    // forgetting to return: the shader's name
    let (_, _, text, msg) = err_at("picture p() {\n    let c = vec4f(1.0);\n}");
    assert_eq!(text, "p");
    assert!(msg.contains("never returns"), "{msg}");
    // pic in a picture
    let src = "picture p() { return pic(uv); }";
    assert!(err_at(src).3.contains("only effects have pic()"));
    // in a helper block
    let src = "wgsl {\n  fn two() -> f32 { return 2; }\n  fn bad() -> f32 { return nope; }\n}\nmesh()";
    let (line, _, text, msg) = err_at(src);
    assert_eq!((line, text.as_str()), (3, "nope"), "{msg}");
    // unused shaders are checked too
    assert!(compile("effect e() { return pic(uv) * ; }\nmesh()").is_err());
    // single-line bodies
    let src = "effect e() { return pic(uv) * zz; }\nmesh().e()";
    assert_eq!(err_at(src).2, "zz");
}

#[test]
fn sketch_shaders_only_tick_when_they_read_the_clock() {
    let still = ok("picture p() { return vec4f(uv, 0.0, 1.0); }\np()");
    assert!(still.graph.passes.iter().all(|p| !p.source().contains("u.time")));
    let moving = ok(&format!("{BREATHE}breathe()"));
    assert!(moving.graph.passes.iter().any(|p| p.source().contains("u.time")));
}
