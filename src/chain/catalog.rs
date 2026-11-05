//! Everything you can write: sources (pictures), effects (chained with `.`),
//! signals, and the curated palettes. Each entry has tasteful defaults, so
//! `mesh()` alone already looks good.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arg {
    /// A number, pattern or signal.
    Num(f64),
    /// A palette: a name like "dawn", hex stops "#f4a7b9 #a8c8f0", or "<dawn ember>".
    Pal(&'static str),
    /// One colour: "#e9547d" or "<#fff #000>".
    Color(&'static str),
    /// One of a fixed set of words.
    Word(&'static str, &'static [&'static str]),
    /// A file path (images).
    Path,
    /// Another picture (with its own chain), e.g. `displace(clouds(), .2)`.
    /// The default is a source name.
    Pic(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tex {
    Blur,
    Bloom,
    Glow,
    Chroma,
    Paint,
    Trail,
    Relief,
    Edges,
    Brush,
    ZoomBlur,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Source,
    /// Moves where the picture is looked up (warp, zoom, kaleido…).
    Domain,
    /// Changes colours (dither, hue, grain…).
    Color,
    /// Needs neighbouring pixels: renders what came before to a texture first.
    Tex(Tex),
}

#[derive(Debug)]
pub struct Spec {
    pub name: &'static str,
    pub role: Role,
    pub args: &'static [(&'static str, Arg)],
    pub doc: &'static str,
    /// WGSL template. `{prev}` is the previous stage (a function of uv),
    /// `{0}`, `{1}`… the arguments.
    pub wgsl: &'static str,
    /// Home view for sources drawn on the pannable plane (fractals).
    pub home: Option<[f32; 3]>,
    pub hidden: bool,
}

const fn spec(
    name: &'static str,
    role: Role,
    args: &'static [(&'static str, Arg)],
    doc: &'static str,
    wgsl: &'static str,
) -> Spec {
    Spec {
        name,
        role,
        args,
        doc,
        wgsl,
        home: None,
        hidden: false,
    }
}

const fn fractal(
    name: &'static str,
    args: &'static [(&'static str, Arg)],
    doc: &'static str,
    wgsl: &'static str,
    home: [f32; 3],
) -> Spec {
    Spec {
        name,
        role: Role::Source,
        args,
        doc,
        wgsl,
        home: Some(home),
        hidden: false,
    }
}

use Arg::{Color as C, Num as N, Pal as P};
use Role::{Color as Col, Domain as Dom, Source as Src};

pub const MASKS: &[&str] = &["circle", "box", "ring", "diamond", "blob"];
pub const BLENDS: &[&str] = &[
    "normal",
    "add",
    "screen",
    "multiply",
    "overlay",
    "soft",
    "difference",
];

pub static SPECS: &[Spec] = &[
    // ── sources ──────────────────────────────────────────────────────────────
    spec(
        "solid",
        Src,
        &[("color", C("#f4efe6"))],
        "one flat colour",
        "src_solid(uv, {0})",
    ),
    spec(
        "gradient",
        Src,
        &[("palette", P("dawn")), ("angle", N(90.0))],
        "linear gradient through a palette (angle in degrees)",
        "src_gradient(uv, {0}, {1})",
    ),
    spec(
        "radial",
        Src,
        &[("palette", P("dawn")), ("x", N(0.5)), ("y", N(0.5))],
        "radial gradient from a centre",
        "src_radial(uv, {0}, {1}, {2})",
    ),
    spec(
        "conic",
        Src,
        &[("palette", P("candy")), ("x", N(0.5)), ("y", N(0.5))],
        "sweep gradient around a centre",
        "src_conic(uv, {0}, {1}, {2})",
    ),
    spec(
        "mesh",
        Src,
        &[("palette", P("dawn")), ("speed", N(1.0))],
        "soft drifting mesh gradient",
        "src_mesh(uv, {0}, {1})",
    ),
    spec(
        "blobs",
        Src,
        &[
            ("palette", P("riviera")),
            ("count", N(6.0)),
            ("size", N(1.0)),
        ],
        "soft colour blobs on the first palette colour",
        "src_blobs(uv, {0}, {1}, {2})",
    ),
    spec(
        "clouds",
        Src,
        &[
            ("palette", P("monet")),
            ("scale", N(3.0)),
            ("speed", N(1.0)),
        ],
        "drifting clouds of fractal noise, like Photoshop's Clouds",
        "src_noise(uv, {0}, {1}, {2})",
    ),
    spec(
        "silk",
        Src,
        &[("palette", P("candy")), ("speed", N(1.0))],
        "folded, light-catching silk",
        "src_silk(uv, {0}, {1})",
    ),
    spec(
        "waves",
        Src,
        &[
            ("palette", P("ocean")),
            ("count", N(6.0)),
            ("height", N(1.0)),
        ],
        "stacked paper-cut waves",
        "src_waves(uv, {0}, {1}, {2})",
    ),
    spec(
        "aurora",
        Src,
        &[("palette", P("dusk")), ("speed", N(1.0))],
        "curtains of light",
        "src_aurora(uv, {0}, {1})",
    ),
    spec(
        "water",
        Src,
        &[
            ("palette", P("sunlit")),
            ("sparkle", N(1.0)),
            ("chop", N(1.0)),
        ],
        "late sun on open water",
        "src_water(uv, {0}, {1}, {2})",
    ),
    spec(
        "cells",
        Src,
        &[
            ("palette", P("citrus")),
            ("scale", N(5.0)),
            ("speed", N(1.0)),
        ],
        "drifting stained-glass cells",
        "src_cells(uv, {0}, {1}, {2})",
    ),
    spec(
        "plasma",
        Src,
        &[("palette", P("neon")), ("scale", N(1.0)), ("speed", N(1.0))],
        "classic plasma",
        "src_plasma(uv, {0}, {1}, {2})",
    ),
    spec(
        "rings",
        Src,
        &[
            ("palette", P("ember")),
            ("count", N(8.0)),
            ("speed", N(1.0)),
        ],
        "concentric bands",
        "src_rings(uv, {0}, {1}, {2})",
    ),
    spec(
        "stripes",
        Src,
        &[
            ("palette", P("citrus")),
            ("count", N(8.0)),
            ("angle", N(45.0)),
            ("speed", N(0.0)),
        ],
        "bands of palette colours",
        "src_stripes(uv, {0}, {1}, {2}, {3})",
    ),
    spec(
        "checker",
        Src,
        &[("palette", P("mono")), ("count", N(8.0))],
        "checkerboard of the first two colours",
        "src_checker(uv, {0}, {1})",
    ),
    spec(
        "dots",
        Src,
        &[
            ("palette", P("candy")),
            ("count", N(12.0)),
            ("size", N(0.5)),
        ],
        "a grid of dots",
        "src_dots(uv, {0}, {1}, {2})",
    ),
    spec(
        "stars",
        Src,
        &[("density", N(1.0))],
        "twinkling stars on transparent",
        "src_stars(uv, {0})",
    ),
    spec(
        "glints",
        Src,
        &[("amount", N(1.0)), ("size", N(1.0))],
        "four-point sparkles on transparent",
        "src_glints(uv, {0}, {1})",
    ),
    fractal(
        "mandelbrot",
        &[
            ("palette", P("fire")),
            ("iters", N(300.0)),
            ("power", N(2.0)),
            ("density", N(1.0)),
        ],
        "the Mandelbrot set (power > 2: multibrots). Drag to pan, scroll to zoom",
        "src_mandel(uv, {0}, {1}, {2}, {3})",
        [-0.65, 0.0, 0.36],
    ),
    fractal(
        "tricorn",
        &[
            ("palette", P("dusk")),
            ("iters", N(300.0)),
            ("density", N(1.0)),
        ],
        "the Mandelbar: z ← conj(z)² + c",
        "src_tricorn(uv, {0}, {1}, {2})",
        [-0.3, 0.0, 0.32],
    ),
    fractal(
        "kali",
        &[
            ("palette", P("lagoon")),
            ("cx", N(0.55)),
            ("cy", N(0.75)),
            ("iters", N(14.0)),
        ],
        "Kali's set: p ← |p|/|p|² − c, lace-like",
        "src_kali(uv, {0}, {1}, {2}, {3})",
        [0.0, 0.0, 0.45],
    ),
    fractal(
        "julia",
        &[
            ("palette", P("lagoon")),
            ("cx", N(-0.8)),
            ("cy", N(0.156)),
            ("iters", N(300.0)),
        ],
        "a Julia set for c = cx + cy·i",
        "src_julia(uv, {0}, {1}, {2}, {3})",
        [0.0, 0.0, 0.32],
    ),
    fractal(
        "ship",
        &[("palette", P("ember")), ("iters", N(300.0))],
        "the Burning Ship fractal",
        "src_ship(uv, {0}, {1})",
        [-0.45, -0.5, 0.36],
    ),
    fractal(
        "newton",
        &[("palette", P("candy")), ("power", N(3.0))],
        "Newton's method basins for zⁿ = 1",
        "src_newton(uv, {0}, {1})",
        [0.0, 0.0, 0.4],
    ),
    spec(
        "image",
        Src,
        &[("file", Arg::Path)],
        "a PNG or JPEG, fitted to cover",
        "src_image(uv, {0})",
    ),
    spec(
        "caustics",
        Src,
        &[
            ("palette", P("lagoon")),
            ("scale", N(1.0)),
            ("speed", N(1.0)),
        ],
        "light dancing on a pool floor",
        "src_caustics(uv, {0}, {1}, {2})",
    ),
    spec(
        "topo",
        Src,
        &[
            ("palette", P("paper")),
            ("lines", N(14.0)),
            ("scale", N(1.6)),
        ],
        "a topographic map with contour lines",
        "src_topo(uv, {0}, {1}, {2})",
    ),
    spec(
        "bokeh",
        Src,
        &[("palette", P("dusk")), ("count", N(28.0)), ("size", N(1.0))],
        "out-of-focus discs of light",
        "src_bokeh(uv, {0}, {1}, {2})",
    ),
    spec(
        "marble",
        Src,
        &[
            ("palette", P("paper")),
            ("scale", N(1.5)),
            ("speed", N(1.0)),
        ],
        "veined stone",
        "src_marble(uv, {0}, {1}, {2})",
    ),
    spec(
        "tunnel",
        Src,
        &[("palette", P("neon")), ("speed", N(1.0)), ("sides", N(8.0))],
        "fly down a tiled tunnel",
        "src_tunnel(uv, {0}, {1}, {2})",
    ),
    spec(
        "lilies",
        Src,
        &[
            ("palette", P("giverny")),
            ("count", N(1.0)),
            ("speed", N(1.0)),
        ],
        "Monet's water lilies: a pond, willow reflections, pads and blossoms",
        "src_lilies(uv, {0}, {1}, {2})",
    ),
    spec(
        "poppies",
        Src,
        &[
            ("palette", P("poppy")),
            ("density", N(1.0)),
            ("wind", N(1.0)),
        ],
        "Monet's poppy field: sky, trees, a meadow of red",
        "src_poppies(uv, {0}, {1}, {2})",
    ),
    spec(
        "sunrise",
        Src,
        &[("palette", P("harbour")), ("sun", N(1.0)), ("haze", N(1.0))],
        "Impression, Sunrise: a hazy harbour, an orange sun, boats",
        "src_sunrise(uv, {0}, {1}, {2})",
    ),
    spec(
        "orbits",
        Src,
        &[("palette", P("atom")), ("count", N(7.0)), ("width", N(1.0))],
        "hand-drawn orbits, like an atom (transparent between)",
        "src_orbits(uv, {0}, {1}, {2})",
    ),
    spec(
        "rays",
        Src,
        &[
            ("palette", P("atom")),
            ("count", N(64.0)),
            ("length", N(1.0)),
        ],
        "a starburst of rays tipped with dots (transparent between)",
        "src_rays(uv, {0}, {1}, {2})",
    ),
    spec(
        "peonies",
        Src,
        &[("palette", P("mono")), ("count", N(1.0)), ("size", N(1.0))],
        "lush peonies over dark leaves, shaded like an engraving (try .stipple())",
        "src_peonies(uv, {0}, {1}, {2})",
    ),
    spec(
        "sun",
        Src,
        &[("palette", P("mono")), ("height", N(0.0)), ("size", N(1.0))],
        "a sun on the horizon, its light streaking the water",
        "src_sun(uv, {0}, {1}, {2})",
    ),
    spec(
        "grid",
        Src,
        &[
            ("palette", P("graph")),
            ("count", N(8.0)),
            ("width", N(1.5)),
        ],
        "graph paper: first colour paper, second the ruling",
        "src_grid(uv, {0}, {1}, {2})",
    ),
    spec(
        "spot",
        Src,
        &[("size", N(0.12)), ("soft", N(0.8))],
        "a soft white spot at the mouse, to feed displace or mask",
        "src_spot(uv, {0}, {1})",
    ),
    // ── domain effects ───────────────────────────────────────────────────────
    spec(
        "zoom",
        Dom,
        &[("amount", N(1.5))],
        "scale about the centre",
        "{prev}(dom_zoom(uv, {0}))",
    ),
    spec(
        "rotate",
        Dom,
        &[("degrees", N(15.0))],
        "rotate about the centre",
        "{prev}(dom_rotate(uv, {0}))",
    ),
    spec(
        "spin",
        Dom,
        &[("turns", N(0.05))],
        "keep rotating (turns per cycle)",
        "{prev}(dom_spin(uv, {0}))",
    ),
    spec(
        "move",
        Dom,
        &[("x", N(0.1)), ("y", N(0.0))],
        "shift by a fraction of the screen",
        "{prev}(dom_move(uv, {0}, {1}))",
    ),
    spec(
        "scroll",
        Dom,
        &[("x", N(0.1)), ("y", N(0.0))],
        "keep moving (screens per cycle)",
        "{prev}(dom_scroll(uv, {0}, {1}))",
    ),
    spec(
        "warp",
        Dom,
        &[("amount", N(0.3)), ("scale", N(2.0))],
        "bend with slow noise",
        "{prev}(dom_warp(uv, {0}, {1}))",
    ),
    spec(
        "flow",
        Dom,
        &[("amount", N(0.3)), ("speed", N(1.0))],
        "liquid, ink-in-water motion",
        "{prev}(dom_flow(uv, {0}, {1}))",
    ),
    spec(
        "swirl",
        Dom,
        &[("amount", N(1.0)), ("radius", N(0.6))],
        "twist around the centre",
        "{prev}(dom_swirl(uv, {0}, {1}))",
    ),
    spec(
        "ripple",
        Dom,
        &[("amount", N(0.3)), ("rings", N(8.0))],
        "rings travelling outwards",
        "{prev}(dom_ripple(uv, {0}, {1}))",
    ),
    spec(
        "wave",
        Dom,
        &[("amount", N(0.3)), ("count", N(3.0))],
        "sideways sine wobble",
        "{prev}(dom_wave(uv, {0}, {1}))",
    ),
    spec(
        "kaleido",
        Dom,
        &[("sides", N(6.0))],
        "kaleidoscope",
        "{prev}(dom_kaleido(uv, {0}))",
    ),
    spec(
        "mirror",
        Dom,
        &[],
        "mirror left onto right",
        "{prev}(dom_mirror(uv))",
    ),
    spec(
        "tile",
        Dom,
        &[("count", N(3.0))],
        "repeat in a grid",
        "{prev}(dom_tile(uv, {0}))",
    ),
    spec(
        "pixelate",
        Dom,
        &[("size", N(8.0))],
        "chunky pixels (size in pixels)",
        "{prev}(dom_pixelate(uv, {0}))",
    ),
    spec(
        "lens",
        Dom,
        &[("amount", N(0.5)), ("radius", N(0.25))],
        "magnify around the mouse",
        "{prev}(dom_lens(uv, {0}, {1}))",
    ),
    spec(
        "polar",
        Dom,
        &[],
        "wrap the picture around the centre",
        "{prev}(dom_polar(uv))",
    ),
    spec(
        "place",
        Dom,
        &[
            ("x", N(0.1)),
            ("y", N(0.1)),
            ("w", N(0.3)),
            ("h", N(0.4)),
            ("radius", N(0.015)),
            ("border", N(0.0)),
            ("color", C("#e5432a")),
        ],
        "put the picture in a card: x y w h (lower-left, fractions of the screen), rounded, optional border px",
        "col_place({prev}(dom_place(uv, {0}, {1}, {2}, {3})), uv, {0}, {1}, {2}, {3}, {4}, {5}, {6})",
    ),
    spec(
        "shuffle",
        Dom,
        &[("count", N(8.0)), ("amount", N(0.35)), ("rate", N(1.0))],
        "cut into tiles and swap some (reshuffles every 1/rate cycles)",
        "{prev}(dom_shuffle(uv, {0}, {1}, {2}))",
    ),
    spec(
        "droste",
        Dom,
        &[("scale", N(0.6)), ("speed", N(0.0))],
        "the picture inside itself, forever (speed > 0 zooms in)",
        "{prev}(dom_droste(uv, {0}, {1}))",
    ),
    spec(
        "displace",
        Dom,
        &[("picture", Arg::Pic("clouds")), ("amount", N(0.3))],
        "bend the picture along another picture's slopes",
        "{prev}(dom_displace(uv, {0}(uv), {0}(uv + vec2f(0.004, 0.0)), {0}(uv + vec2f(0.0, 0.004)), {1}))",
    ),
    spec(
        "dive",
        Dom,
        &[("speed", N(1.0)), ("depth", N(10.0))],
        "endless zoom (fractals: into the view's centre)",
        "{prev}(dom_dive(uv, {0}, {1}))",
    ),
    spec(
        "reeded",
        Dom,
        &[("count", N(24.0)), ("amount", N(0.6))],
        "fluted, reeded glass",
        "col_reeded({prev}(dom_reeded(uv, {0}, {1})), uv, {0}, {1})",
    ),
    spec(
        "glass",
        Dom,
        &[("scale", N(6.0)), ("amount", N(0.4))],
        "a pane of faceted glass cells",
        "col_glass({prev}(dom_glass(uv, {0}, {1})), uv, {0}, {1})",
    ),
    // ── colour effects ───────────────────────────────────────────────────────
    spec(
        "hue",
        Col,
        &[("degrees", N(30.0))],
        "rotate hues (perceptually)",
        "col_hue({prev}(uv), {0})",
    ),
    spec(
        "saturate",
        Col,
        &[("amount", N(1.3))],
        "more (or less, < 1) colour",
        "col_saturate({prev}(uv), {0})",
    ),
    spec(
        "contrast",
        Col,
        &[("amount", N(1.2))],
        "push lights and darks apart",
        "col_contrast({prev}(uv), {0})",
    ),
    spec(
        "brightness",
        Col,
        &[("amount", N(0.1))],
        "lighten (or darken, < 0)",
        "col_brightness({prev}(uv), {0})",
    ),
    spec("invert", Col, &[], "negative", "col_invert({prev}(uv))"),
    spec(
        "tint",
        Col,
        &[("color", C("#e9547d")), ("amount", N(0.3))],
        "wash with a colour",
        "col_tint({prev}(uv), {0}, {1})",
    ),
    spec(
        "map",
        Col,
        &[("palette", P("ember"))],
        "gradient-map brightness onto a palette",
        "col_map({prev}(uv), {0})",
    ),
    spec(
        "posterize",
        Col,
        &[("levels", N(5.0))],
        "fewer colour steps",
        "col_posterize({prev}(uv), {0})",
    ),
    spec(
        "dither",
        Col,
        &[("levels", N(4.0)), ("size", N(1.0))],
        "ordered dither: levels per channel, or a palette",
        "col_dither({prev}(uv), uv, {0}, {1})",
    ),
    Spec {
        name: "dither_palette",
        role: Col,
        args: &[
            ("palette", P("riviera")),
            ("size", N(1.0)),
            ("spread", N(0.3)),
        ],
        doc: "",
        wgsl: "col_dither_pal({prev}(uv), uv, {0}, {1}, {2})",
        home: None,
        hidden: true,
    },
    spec(
        "halftone",
        Col,
        &[("size", N(8.0)), ("angle", N(45.0))],
        "printed dots on paper",
        "col_halftone({prev}(ht_cell(uv, {0}, {1})), uv, {0}, {1})",
    ),
    spec(
        "ascii",
        Col,
        &[("size", N(10.0))],
        "draw with characters",
        "col_ascii({prev}(px_cell(uv, {0})), uv, {0})",
    ),
    spec(
        "scanlines",
        Col,
        &[("amount", N(0.3))],
        "CRT lines",
        "col_scanlines({prev}(uv), uv, {0})",
    ),
    spec(
        "grain",
        Col,
        &[("amount", N(0.06))],
        "film grain",
        "col_grain({prev}(uv), uv, {0})",
    ),
    // Photoshop's Add Noise (the flowing-noise picture is `clouds`)
    Spec {
        name: "noise",
        role: Col,
        args: &[
            ("amount", N(0.1)),
            ("color", N(1.0)),
            ("size", N(1.0)),
            ("motion", N(0.0)),
            ("kind", Arg::Word("gaussian", NOISE_KINDS)),
        ],
        doc: "Photoshop's Add Noise: color 0 for monochromatic, size in px, motion 0 for still, \"gaussian\" or \"uniform\"",
        wgsl: "col_noise({prev}(uv), uv, {0}, {1}, {2}, {3}, {4})",
        home: None,
        hidden: false,
    },
    // …and with motion left at 0: no clock in the shader, so a still picture stays free
    Spec {
        name: "noise_still",
        role: Col,
        args: &[
            ("amount", N(0.1)),
            ("color", N(1.0)),
            ("size", N(1.0)),
            ("kind", Arg::Word("gaussian", NOISE_KINDS)),
        ],
        doc: "",
        wgsl: "col_noise_at({prev}(uv), uv, {0}, {1}, {2}, 0u, {3})",
        home: None,
        hidden: true,
    },
    spec(
        "vignette",
        Col,
        &[("amount", N(0.5))],
        "darken the edges",
        "col_vignette({prev}(uv), uv, {0})",
    ),
    spec(
        "mask",
        Col,
        &[
            ("shape", Arg::Word("circle", MASKS)),
            ("size", N(0.4)),
            ("soft", N(0.02)),
        ],
        "cut out a shape (circle box ring diamond blob), or mask by a picture: mask(spot())",
        "col_mask({prev}(uv), uv, {0}, {1}, {2})",
    ),
    spec(
        "opacity",
        Col,
        &[("amount", N(0.5))],
        "see-through",
        "col_opacity({prev}(uv), {0})",
    ),
    spec(
        "stipple",
        Col,
        &[
            ("palette", P("cobalt")),
            ("size", N(1.0)),
            ("contrast", N(1.2)),
        ],
        "riso / engraving grain: dark → first ink, light → last (more stops: more inks)",
        "col_stipple({prev}(uv), uv, {0}, {1}, {2})",
    ),
    spec(
        "canvas",
        Col,
        &[("amount", N(0.6))],
        "linen canvas weave",
        "col_canvas({prev}(uv), uv, {0})",
    ),
    spec(
        "contour",
        Col,
        &[("lines", N(10.0)), ("width", N(1.0))],
        "draw isolines through the brightness",
        "col_contour({prev}(uv), {0}, {1})",
    ),
    spec(
        "duotone",
        Col,
        &[("palette", P("ink")), ("contrast", N(1.2))],
        "two colours: the palette's first and last",
        "col_duotone({prev}(uv), {0}, {1})",
    ),
    // stand-ins for shaders written in the sketch (`picture name(…) { … }`); the
    // instance carries the code, so these never generate anything themselves
    Spec {
        name: "user_picture",
        role: Src,
        args: &[],
        doc: "",
        wgsl: "",
        home: None,
        hidden: true,
    },
    Spec {
        name: "user_effect",
        role: Col,
        args: &[],
        doc: "",
        wgsl: "",
        home: None,
        hidden: true,
    },
    Spec {
        name: "mask_picture",
        role: Col,
        args: &[("picture", Arg::Pic("spot")), ("amount", N(1.0))],
        doc: "",
        wgsl: "col_mask_pic({prev}(uv), {0}(uv), {1})",
        home: None,
        hidden: true,
    },
    // ── texture effects (each renders the chain so far to a texture) ─────────
    spec(
        "blur",
        Role::Tex(Tex::Blur),
        &[("amount", N(0.5))],
        "soft focus",
        "",
    ),
    spec(
        "bloom",
        Role::Tex(Tex::Bloom),
        &[("amount", N(0.6)), ("threshold", N(0.6))],
        "highlights bleed light",
        "",
    ),
    spec(
        "glow",
        Role::Tex(Tex::Glow),
        &[("amount", N(0.5)), ("size", N(0.5))],
        "everything glows softly",
        "",
    ),
    spec(
        "chroma",
        Role::Tex(Tex::Chroma),
        &[("amount", N(0.3))],
        "chromatic aberration",
        "",
    ),
    spec(
        "paint",
        Role::Tex(Tex::Paint),
        &[("size", N(0.5))],
        "painterly brush strokes (Kuwahara)",
        "",
    ),
    spec(
        "trail",
        Role::Tex(Tex::Trail),
        &[("amount", N(0.85))],
        "motion trails (feedback)",
        "",
    ),
    spec(
        "relief",
        Role::Tex(Tex::Relief),
        &[("amount", N(0.6)), ("angle", N(135.0))],
        "light it like embossed paper",
        "",
    ),
    spec(
        "brush",
        Role::Tex(Tex::Brush),
        &[("size", N(1.0)), ("chaos", N(0.3)), ("angle", N(15.0))],
        "impressionist brushstrokes that follow the picture (angle: where they lean in flat areas)",
        "",
    ),
    spec(
        "zoomblur",
        Role::Tex(Tex::ZoomBlur),
        &[("amount", N(0.5)), ("x", N(0.5)), ("y", N(0.5))],
        "motion blur rushing toward a point",
        "",
    ),
    spec(
        "edges",
        Role::Tex(Tex::Edges),
        &[("amount", N(1.0))],
        "ink the outlines",
        "",
    ),
];

pub fn find(name: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.name == name)
}

pub const NOISE_KINDS: &[&str] = &["gaussian", "uniform"];

/// One line for each word an argument can take, for completion and hover.
pub const WORD_DOCS: &[(&str, &str)] = &[
    ("gaussian", "soft, film-like grain (Photoshop's Gaussian)"),
    ("uniform", "even, crisper grain (Photoshop's Uniform)"),
    ("circle", "a round cut-out"),
    ("box", "a square cut-out"),
    ("ring", "a hollow circle"),
    ("diamond", "a square on its point"),
    ("blob", "a soft, wobbling shape"),
    ("normal", "on top, by its alpha"),
    ("add", "brighter: light adds up"),
    ("screen", "brighter, never blown out"),
    ("multiply", "darker: like layered inks"),
    ("overlay", "more contrast"),
    ("soft", "gentle contrast, soft light"),
    ("difference", "inverts where it overlaps"),
];

pub fn word_doc(w: &str) -> Option<&'static str> {
    WORD_DOCS.iter().find(|d| d.0 == w).map(|d| d.1)
}

/// The name a spec is written with (an alias's, if it has one).
pub fn written(s: &Spec) -> &'static str {
    const VARIANTS: &[(&str, &str)] = &[
        ("noise_still", "noise"),
        ("dither_palette", "dither"),
        ("mask_picture", "mask"),
    ];
    VARIANTS
        .iter()
        .find(|v| v.0 == s.name)
        .map_or(s.name, |v| v.1)
}

/// The effect a chained `.name(…)` means.
pub fn effect(name: &str) -> Option<&'static Spec> {
    find(name).filter(|s| !s.hidden)
}

/// Every effect as it's written after a dot, for completion and the reference.
pub fn effects() -> impl Iterator<Item = (&'static str, &'static Spec)> {
    SPECS
        .iter()
        .filter(|s| !s.hidden && s.role != Role::Source)
        .map(|s| (s.name, s))
}

/// Layer-level methods that aren't stages.
pub const LAYER_METHODS: &[(&str, &str)] = &[(
    "blend",
    "how this layer mixes with the ones below: normal add screen multiply overlay soft difference",
)];

/// Signals (all 0..1) and their methods, for docs and suggestions.
pub const SIGNALS: &[(&str, &str)] = &[
    ("sine", "smooth up and down, once per cycle"),
    ("cosine", "sine, a quarter cycle later"),
    ("saw", "ramp up, once per cycle"),
    ("isaw", "ramp down"),
    ("tri", "up then down"),
    ("square", "off, then on"),
    ("rand", "a new random value each cycle"),
    ("perlin", "smooth random wander"),
    ("mouse.x", "pointer position"),
    ("mouse.y", "pointer position"),
    ("time", "seconds (at the default speed)"),
];

pub const SIGNAL_METHODS: &[(&str, &str)] = &[
    ("range", "range(lo, hi): rescale 0..1 to lo..hi"),
    ("slow", "slow(n): n times slower"),
    ("fast", "fast(n): n times faster"),
    ("early", "early(c): shift earlier by c cycles"),
    ("late", "late(c): shift later by c cycles"),
    ("segment", "segment(n): hold n values per cycle"),
    ("add", "add(x): plus x (a number or another signal)"),
    ("sub", "sub(x): minus x"),
    ("mul", "mul(x): times x"),
    ("div", "div(x): divided by x"),
    ("smooth", "smooth(): glide across each step"),
    ("sharp", "sharp(): jump between steps"),
];

pub const FUNCTIONS: &[(&str, &str)] = &[
    (
        "slider",
        "slider(value, lo, hi): a live slider in the sliders card",
    ),
    ("speed", "speed(cps): cycles per second (default 0.5)"),
    ("view", "view(x, y, zoom): where fractals start"),
    ("stack", "stack(a, b, …): layers, bottom first"),
];

/// Curated palettes, dark to light where that makes sense.
pub static PALETTES: &[(&str, &[&str])] = &[
    (
        "dawn",
        &["#fde2d0", "#f7b5a8", "#e98fa6", "#a99bd6", "#8fc1e8"],
    ),
    (
        "riviera",
        &[
            "#f0be90", "#c9a240", "#d65a57", "#d84a12", "#e9547d", "#46c0b2", "#4a86bf",
        ],
    ),
    (
        "sunlit",
        &[
            "#061a19", "#123a30", "#33533c", "#6b6a40", "#c9a86a", "#fff0cf",
        ],
    ),
    (
        "lagoon",
        &["#04161a", "#0c3b3a", "#1e6b62", "#5fa79a", "#cfe8d9"],
    ),
    (
        "monet",
        &[
            "#3d5a80", "#6b8fb5", "#a9c4d8", "#e6d7b8", "#c9a7c7", "#8fb39a",
        ],
    ),
    (
        "ember",
        &[
            "#1a0b12", "#5c1530", "#b0303a", "#e8653a", "#f7a440", "#ffe2a8",
        ],
    ),
    (
        "fire",
        &[
            "#07040a", "#3a0a14", "#a3201b", "#ef6a1a", "#ffc24a", "#fff6dc",
        ],
    ),
    (
        "dusk",
        &[
            "#120d2a", "#3b2a63", "#8d4b8f", "#d26a8c", "#f6a58f", "#ffe4c4",
        ],
    ),
    (
        "citrus",
        &["#fff6e0", "#ffd166", "#ff9f1c", "#ef476f", "#2ec4b6"],
    ),
    (
        "candy",
        &["#ffd6e8", "#ff9ec7", "#b8b5ff", "#7ad7f0", "#b8f2e6"],
    ),
    (
        "ocean",
        &["#caf0f8", "#90e0ef", "#00b4d8", "#0077b6", "#03045e"],
    ),
    (
        "sage",
        &["#1f2b24", "#3f5a47", "#6b8f71", "#b7cfb0", "#eef3e8"],
    ),
    (
        "neon",
        &["#0a0612", "#3a0ca3", "#7209b7", "#f72585", "#4cc9f0"],
    ),
    (
        "paper",
        &["#2e2a24", "#8a7a63", "#cbbba0", "#e9e0cf", "#f7f3ea"],
    ),
    ("mono", &["#0e0e10", "#4a4a50", "#a3a3a8", "#f2f2f0"]),
    ("ink", &["#0b1d33", "#f4efe6"]),
    // Monet. Scene palettes have a fixed stop order (see each scene's doc).
    (
        "giverny",
        &[
            "#18323f", "#2f5868", "#5d8a8c", "#b9cfd0", "#3f6b34", "#8aa65a", "#e48aa7", "#f7e6ec",
        ],
    ),
    (
        "poppy",
        &[
            "#2f4a2c", "#6e8c45", "#b5c06e", "#d43b28", "#f0703a", "#b7cde0", "#f1f3ef",
        ],
    ),
    (
        "harbour",
        &[
            "#1f2b33", "#4f6470", "#7f9298", "#b8b9a8", "#e8682a", "#f5a14c",
        ],
    ),
    (
        "rouen",
        &["#3b4e6b", "#7a8fb0", "#c7b7c9", "#efd9b8", "#f6e9cf"],
    ),
    ("atom", &["#ff4a1c", "#ff7a45", "#ff9fbf", "#2c2c2c"]),
    // riso inks on paper (ink first, paper last)
    ("cobalt", &["#1b2fb2", "#f1ece1"]),
    ("vermilion", &["#e5432a", "#f3ede2"]),
    ("riso", &["#1b2fb2", "#e5432a", "#f1ece1"]),
    ("graph", &["#f1ece1", "#e5432a"]),
];

pub fn palette(name: &str) -> Option<&'static [&'static str]> {
    PALETTES.iter().find(|p| p.0 == name).map(|p| p.1)
}

/// Every name a user might mean, for "did you mean" suggestions.
pub fn all_names() -> impl Iterator<Item = &'static str> {
    SPECS
        .iter()
        .filter(|s| !s.hidden)
        .map(|s| s.name)
        .chain(SIGNALS.iter().map(|s| s.0))
        .chain(SIGNAL_METHODS.iter().map(|s| s.0))
        .chain(FUNCTIONS.iter().map(|s| s.0))
        .chain(LAYER_METHODS.iter().map(|s| s.0))
}

pub fn signature(s: &Spec) -> String {
    signature_as(s.name, s)
}

/// A number as you'd type it: `.3`, `1`, `-.5`, `300`.
pub fn fmt_num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    match s {
        "-0" => "0".into(),
        _ if s.starts_with("0.") => s[1..].into(),
        _ if s.starts_with("-0.") => format!("-{}", &s[2..]),
        _ => s.into(),
    }
}

/// An argument's default, as you'd type it.
pub fn default_text(a: &Arg) -> Option<String> {
    Some(match a {
        Arg::Num(v) => fmt_num(*v),
        Arg::Pal(p) | Arg::Color(p) | Arg::Word(p, _) => format!("\"{p}\""),
        Arg::Pic(src) => format!("{src}()"),
        Arg::Path => return None,
    })
}

/// What an argument takes, in a few words: shown while you type it.
pub fn arg_hint(a: &Arg) -> String {
    match a {
        Arg::Num(_) => "a number, pattern or signal".into(),
        Arg::Pal(_) => "a palette: type \" for the list".into(),
        Arg::Color(_) => "a colour, like \"#e9547d\"".into(),
        Arg::Word(_, opts) => format!("one of {}", opts.join(", ")),
        Arg::Path => "an image file, like \"photo.jpg\"".into(),
        Arg::Pic(_) => "a picture, like clouds() or spot().trail(.9)".into(),
    }
}

/// `flow(amount = .3, speed = 1)`: the signature with every default.
pub fn signature_defaults(name: &str, s: &Spec) -> String {
    let args: Vec<String> = s
        .args
        .iter()
        .map(|(n, a)| match default_text(a) {
            Some(d) => format!("{n} = {d}"),
            None => n.to_string(),
        })
        .collect();
    format!("{name}({})", args.join(", "))
}

/// The signature under the name it's written with (aliases).
pub fn signature_as(name: &str, s: &Spec) -> String {
    let args: Vec<&str> = s.args.iter().map(|a| a.0).collect();
    format!("{name}({})", args.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_option_is_documented() {
        for s in SPECS {
            for (_, a) in s.args {
                if let Arg::Word(_, opts) = a {
                    for o in *opts {
                        assert!(word_doc(o).is_some(), "{} option {o} needs a WORD_DOCS line", s.name);
                    }
                }
            }
        }
        for b in BLENDS {
            assert!(word_doc(b).is_some(), "blend {b}");
        }
    }

    #[test]
    fn numbers_read_as_typed() {
        for (v, t) in [(0.3, ".3"), (1.0, "1"), (-0.5, "-.5"), (300.0, "300"), (10.5, "10.5"), (0.06, ".06"), (0.0, "0"), (-0.0, "0")] {
            assert_eq!(fmt_num(v), t);
        }
        let flow = find("flow").unwrap();
        assert!(signature_defaults("flow", flow).starts_with("flow(amount = "));
    }

    #[test]
    fn names_are_unique_and_palettes_parse() {
        let mut names: Vec<_> = SPECS.iter().map(|s| s.name).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
        for (name, cols) in PALETTES {
            assert!(cols.len() <= 8, "{name}");
            for c in *cols {
                assert!(
                    crate::chain::mini::parse_hex(&c[1..]).is_some(),
                    "{name} {c}"
                );
            }
        }
        for s in SPECS {
            for (_, a) in s.args {
                if let Arg::Pal(p) = a {
                    assert!(palette(p).is_some(), "{} default {p}", s.name);
                }
            }
            if !matches!(s.role, Role::Tex(_)) {
                for i in 0..s.args.len() {
                    assert!(
                        s.wgsl.contains(&format!("{{{i}}}")),
                        "{} uses arg {i}",
                        s.name
                    );
                }
            }
        }
    }
}
