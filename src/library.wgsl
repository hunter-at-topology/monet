// ── monet library ─────────────────────────────────────────────────────────────
// Every pass is this library plus a few generated lines. Sources return
// vec4f (straight alpha), effects take and return them. Colours are
// display-referred sRGB (what you write is what you see); palettes live in
// OKLab and are interpolated there, so gradients never go muddy.
// ──────────────────────────────────────────────────────────────────────────────

struct U {
    res: vec2f, mouse: vec2f,
    time: f32, dt: f32, frame: f32, cycle: f32,
    view: vec4f,
    s: array<vec4f, 64>,
    c: array<vec4f, 128>,
}
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var t0: texture_2d<f32>;
@group(0) @binding(3) var t1: texture_2d<f32>;
@group(0) @binding(4) var t2: texture_2d<f32>;
@group(0) @binding(5) var t3: texture_2d<f32>;
@group(0) @binding(6) var t4: texture_2d<f32>;
@group(0) @binding(7) var t5: texture_2d<f32>;
@group(0) @binding(8) var t6: texture_2d<f32>;
@group(0) @binding(9) var t7: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

const PI: f32 = 3.14159265359;
const TAU: f32 = 6.28318530718;

fn frag_uv(pos: vec4f, div: f32) -> vec2f {
    let r = ceil(u.res / div);
    return vec2f(pos.x, r.y - pos.y) / r;
}
fn tx(t: texture_2d<f32>, uv: vec2f) -> vec4f {
    return textureSampleLevel(t, smp, vec2f(uv.x, 1.0 - uv.y), 0.0);
}

// ── coordinates ──────────────────────────────────────────────────────────────
/// The frame's aspect ratio. Inside `.place()` it's the card's, so pictures
/// placed in a card are drawn for the card, not stretched from the screen.
var<private> card_aspect: f32;
fn aspect() -> f32 { return select(u.res.x / u.res.y, card_aspect, card_aspect > 0.0); }
/// Aspect-correct, centred: y in [-0.5, 0.5].
fn asp(uv: vec2f) -> vec2f { return (uv - 0.5) * vec2f(aspect(), 1.0); }
fn unasp(p: vec2f) -> vec2f { return p / vec2f(aspect(), 1.0) + 0.5; }
/// The pannable, zoomable plane (fractals).
fn plane(uv: vec2f) -> vec2f { return asp(uv) / u.view.z + u.view.xy; }
fn rot(a: f32) -> mat2x2f { let c = cos(a); let s = sin(a); return mat2x2f(c, s, -s, c); }
fn sat(v: f32) -> f32 { return clamp(v, 0.0, 1.0); }
fn tri(x: f32) -> f32 { return 1.0 - abs(2.0 * fract(x * 0.5) - 1.0); }

// ── hashing & noise ──────────────────────────────────────────────────────────
fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}
fn pcg2(v: vec2u) -> vec2u {
    var q = v * 1664525u + 1013904223u;
    q.x += q.y * 1664525u; q.y += q.x * 1664525u;
    q = q ^ (q >> vec2u(16u));
    q.x += q.y * 1664525u; q.y += q.x * 1664525u;
    return q ^ (q >> vec2u(16u));
}
fn hash11(p: f32) -> f32 { return f32(pcg(bitcast<u32>(i32(floor(p))))) / 4294967295.0; }
fn hash21(p: vec2f) -> f32 { return f32(pcg2(bitcast<vec2u>(vec2i(floor(p)))).x) / 4294967295.0; }
fn hash22(p: vec2f) -> vec2f { return vec2f(pcg2(bitcast<vec2u>(vec2i(floor(p))))) / 4294967295.0; }

/// Gradient noise, roughly [0, 1].
fn noise(p: vec2f) -> f32 {
    let i = floor(p); let f = fract(p);
    let w = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let ga = hash22(i) * 2.0 - 1.0;
    let gb = hash22(i + vec2f(1.0, 0.0)) * 2.0 - 1.0;
    let gc = hash22(i + vec2f(0.0, 1.0)) * 2.0 - 1.0;
    let gd = hash22(i + vec2f(1.0, 1.0)) * 2.0 - 1.0;
    let a = dot(ga, f); let b = dot(gb, f - vec2f(1.0, 0.0));
    let c = dot(gc, f - vec2f(0.0, 1.0)); let d = dot(gd, f - vec2f(1.0, 1.0));
    return 0.5 + 0.7 * mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}
fn noise3(p: vec2f, t: f32) -> f32 {
    let i = floor(t); let f = smoothstep(0.0, 1.0, fract(t));
    return mix(noise(p + hash22(vec2f(i, 7.0)) * 97.0), noise(p + hash22(vec2f(i + 1.0, 7.0)) * 97.0), f);
}
fn fbm(p: vec2f, oct: i32) -> f32 {
    var v = 0.0; var a = 0.5; var q = p; var n = 0.0;
    let m = mat2x2f(1.6, 1.2, -1.2, 1.6);
    for (var i = 0; i < oct; i++) { v += a * noise(q); n += a; q = m * q; a *= 0.5; }
    return v / n;
}
fn voronoi(p: vec2f, t: f32) -> vec3f {
    let i = floor(p); let f = fract(p);
    var best = vec3f(8.0, 8.0, 0.0);
    for (var y = -1; y <= 1; y++) { for (var x = -1; x <= 1; x++) {
        let g = vec2f(f32(x), f32(y));
        let h = hash22(i + g);
        let o = 0.5 + 0.4 * sin(t + TAU * h);
        let d = length(g + o - f);
        if (d < best.x) { best = vec3f(d, best.x, hash21(i + g + 0.5)); }
        else if (d < best.y) { best.y = d; }
    } }
    return best;
}

// ── colour ───────────────────────────────────────────────────────────────────
fn to_lin(c: vec3f) -> vec3f { return select(pow((c + 0.055) / 1.055, vec3f(2.4)), c / 12.92, c <= vec3f(0.04045)); }
fn to_srgb(c: vec3f) -> vec3f {
    let x = max(c, vec3f(0.0));
    return select(1.055 * pow(x, vec3f(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3f(0.0031308));
}
fn oklab_to_srgb(c: vec3f) -> vec3f {
    let l_ = c.x + 0.3963377774 * c.y + 0.2158037573 * c.z;
    let m_ = c.x - 0.1055613458 * c.y - 0.0638541728 * c.z;
    let s_ = c.x - 0.0894841775 * c.y - 1.2914855480 * c.z;
    let l = l_ * l_ * l_; let m = m_ * m_ * m_; let s = s_ * s_ * s_;
    return to_srgb(vec3f(
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s));
}
fn cbrt(x: f32) -> f32 { return sign(x) * pow(abs(x), 1.0 / 3.0); }
fn srgb_to_oklab(c: vec3f) -> vec3f {
    let r = to_lin(clamp(c, vec3f(0.0), vec3f(1.0)));
    let l = cbrt(0.4122214708 * r.x + 0.5363325363 * r.y + 0.0514459929 * r.z);
    let m = cbrt(0.2119034982 * r.x + 0.6806995451 * r.y + 0.1073969566 * r.z);
    let s = cbrt(0.0883024619 * r.x + 0.2817188376 * r.y + 0.6299787005 * r.z);
    return vec3f(
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s);
}
fn luma(c: vec3f) -> f32 { return dot(c, vec3f(0.2126, 0.7152, 0.0722)); }
fn aces(x: vec3f) -> vec3f { return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3f(0.0), vec3f(1.0)); }

/// Palette `b` (a slot base): `pal(b, t)` is a smooth OKLab gradient through
/// its stops, `palc(b, i)` the i-th stop (wrapping).
fn paln(b: u32) -> u32 { return clamp(u32(u.c[b].w), 1u, 8u); }
fn pal_lab(b: u32, t: f32) -> vec3f {
    let n = paln(b);
    if (n == 1u) { return u.c[b].xyz; }
    let x = sat(t) * f32(n - 1u);
    let i = min(u32(x), n - 2u);
    return mix(u.c[b + i].xyz, u.c[b + i + 1u].xyz, x - f32(i));
}
fn pal(b: u32, t: f32) -> vec3f { return oklab_to_srgb(pal_lab(b, t)); }
fn palc_lab(b: u32, i: u32) -> vec3f { return u.c[b + (i % paln(b))].xyz; }
fn palc(b: u32, i: u32) -> vec3f { return oklab_to_srgb(palc_lab(b, i)); }

fn bayer2(a: vec2f) -> f32 { let b = floor(a); return fract(b.x * 0.5 + b.y * b.y * 0.75); }
fn bayer4(a: vec2f) -> f32 { return bayer2(0.5 * a) * 0.25 + bayer2(a); }
fn bayer(p: vec2f) -> f32 { return bayer4(0.5 * p) * 0.25 + bayer2(p); }

// ── light ────────────────────────────────────────────────────────────────────
fn star(p: vec2f, r: f32) -> f32 {
    let q = abs(p) / r;
    let core = 1.0 / (1.0 + dot(q, q) * 24.0);
    let rays = exp(-q.x * 16.0 - q.y * 1.6) + exp(-q.y * 16.0 - q.x * 1.6);
    return core + rays * 0.55;
}
fn glow(d: f32, r: f32) -> f32 { return r * r / (d * d + r * r * 0.05); }
fn glint_field(s: vec2f, size: f32) -> f32 {
    var g = 0.0;
    for (var y = -1; y <= 1; y++) { for (var x = -1; x <= 1; x++) {
        let cell = floor(s) + vec2f(f32(x), f32(y));
        let o = hash22(cell);
        let c = cell + 0.2 + o * 0.6;
        if (length(s - c) >= 1.5) { continue; }   // fades to nothing beyond 1.5
        let life = smoothstep(0.62, 0.86, noise3(c * 0.3, u.time * 0.8 + o.x * 5.0));
        if (life <= 0.0) { continue; }            // most sparkles are resting
        let twinkle = 0.55 + 0.45 * sin(u.time * (4.0 + o.y * 6.0) + o.x * TAU);
        let sz = (0.25 + o.y * o.y * 0.9) * size;
        let fade = pow(sat(1.0 - length(s - c) / 1.5), 2.0);
        g += (star(s - c, sz) * 2.6 * twinkle + glow(length(s - c), sz) * 0.08) * life * fade;
    } }
    return g;
}
fn vig(uv: vec2f, k: f32) -> f32 { let q = uv * (1.0 - uv); return pow(16.0 * q.x * q.y, k); }

// ── sources ──────────────────────────────────────────────────────────────────
fn src_solid(uv: vec2f, b: u32) -> vec4f { return vec4f(palc(b, 0u), 1.0); }

fn src_gradient(uv: vec2f, b: u32, angle: f32) -> vec4f {
    let a = radians(angle);
    let d = vec2f(cos(a), sin(a));
    let e = 0.5 * (abs(d.x) * aspect() + abs(d.y));
    return vec4f(pal(b, dot(asp(uv), d) / (2.0 * e) + 0.5), 1.0);
}

fn src_radial(uv: vec2f, b: u32, x: f32, y: f32) -> vec4f {
    let r = length(asp(uv) - asp(vec2f(x, y)));
    return vec4f(pal(b, r / (0.5 * length(vec2f(aspect(), 1.0)))), 1.0);
}

fn src_conic(uv: vec2f, b: u32, x: f32, y: f32) -> vec4f {
    let p = asp(uv) - asp(vec2f(x, y));
    let a = atan2(p.y, p.x) / TAU + 0.5 + u.time * 0.02;
    return vec4f(pal(b, tri(a * 2.0)), 1.0);
}

fn src_mesh(uv: vec2f, b: u32, speed: f32) -> vec4f {
    let p = asp(uv);
    let t = u.time * 0.12 * speed;
    let q = p + (vec2f(noise(p * 1.3 + vec2f(t, 0.0)), noise(p * 1.3 + vec2f(7.1, -t))) - 0.5) * 0.4;
    let n = paln(b);
    var acc = vec3f(0.0); var ws = 0.0;
    for (var i = 0u; i < n; i++) {
        let fi = f32(i);
        let h = hash22(vec2f(fi, 3.0));
        let c = vec2f(sin(t * (0.6 + h.x * 0.5) + fi * 2.39996) * 0.42 * aspect(),
                      cos(t * (0.5 + h.y * 0.5) + fi * 1.7) * 0.38);
        let d = q - c;
        let w = 1.0 / pow(dot(d, d) + 0.02, 1.6);
        acc += u.c[b + i].xyz * w; ws += w;
    }
    return vec4f(oklab_to_srgb(acc / ws), 1.0);
}

fn src_blobs(uv: vec2f, b: u32, count: f32, size: f32) -> vec4f {
    let p = asp(uv);
    let t = u.time * 0.08;
    let w = p + (vec2f(fbm(p * 2.2 + t, 3), fbm(p * 2.2 - t + 7.0, 3)) - 0.5) * 0.18;
    let n = paln(b);
    var col = u.c[b].xyz;
    let k = i32(clamp(count, 1.0, 16.0));
    for (var i = 0; i < k; i++) {
        let fi = f32(i);
        let h = hash22(vec2f(fi, 11.0));
        let r = size * (0.16 + 0.2 * hash11(fi + 40.0));
        let c = vec2f((h.x - 0.5) * aspect() * 1.05 + 0.1 * sin(t * 1.3 + fi * 1.7),
                      (h.y - 0.5) * 1.05 + 0.08 * cos(t * 1.1 + fi * 2.3));
        let f = exp(-dot(w - c, w - c) / (r * r));
        let ci = select(u.c[b].xyz, u.c[b + 1u + u32(i) % max(n - 1u, 1u)].xyz, n > 1u);
        col = mix(col, ci, smoothstep(0.3, 0.7, f));
    }
    return vec4f(oklab_to_srgb(col), 1.0);
}

fn src_noise(uv: vec2f, b: u32, scale: f32, speed: f32) -> vec4f {
    let p = asp(uv) * scale;
    let t = u.time * 0.06 * speed;
    let q = vec2f(fbm(p * 0.5 + vec2f(t, -t), 3), fbm(p * 0.5 + vec2f(3.1, 1.7 + t), 3));
    let v = fbm(p + q * 1.5 + vec2f(t * 0.5, 0.0), 5);
    return vec4f(pal(b, smoothstep(0.2, 0.8, v)), 1.0);
}

fn src_silk(uv: vec2f, b: u32, speed: f32) -> vec4f {
    let p = asp(uv) * 0.9;
    let t = u.time * 0.04 * speed;
    let q = vec2f(fbm(p + vec2f(0.0, t), 3), fbm(p + vec2f(5.2, 1.3) - t, 3));
    let v = fbm(p + 1.6 * q + vec2f(t * 0.7, -t * 0.4), 4);
    // folds: a slow sine through the warped field, lit from the upper left
    let f = v * 16.0 + q.x * 4.0;
    let fold = 0.5 + 0.5 * sin(f);
    let light = 0.5 + 0.5 * cos(f - 0.9);
    var c = pal(b, smoothstep(0.1, 0.9, v * 0.7 + fold * 0.3));
    c = c * (0.78 + 0.3 * light) + vec3f(pow(light, 10.0) * 0.16);
    return vec4f(c, 1.0);
}

fn src_waves(uv: vec2f, b: u32, count: f32, height: f32) -> vec4f {
    let n = i32(clamp(count, 2.0, 16.0));
    let x = uv.x * aspect();
    let t = u.time;
    let px = 1.5 / u.res.y;
    var col = pal(b, 0.0);
    for (var i = 0; i < n; i++) {
        let fi = f32(i);
        let y = 1.0 - (fi + 0.8) / f32(n)
            + height * (0.05 * sin(x * 2.2 + t * 0.5 + fi * 1.3) + 0.025 * sin(x * 5.1 - t * 0.8 + fi * 2.1));
        let d = y - uv.y;
        let c = pal(b, (fi + 1.0) / f32(n)) * (1.0 - 0.22 * exp(-max(d, 0.0) * 28.0));
        col = mix(col, c, smoothstep(-px, px, d));
    }
    return vec4f(col, 1.0);
}

fn src_aurora(uv: vec2f, b: u32, speed: f32) -> vec4f {
    let t = u.time * 0.15 * speed;
    let x = asp(uv).x;
    var col = mix(pal(b, 0.0), pal(b, 0.15), uv.y);
    for (var k = 0; k < 3; k++) {
        let fk = f32(k);
        let centre = 0.42 + fk * 0.08 + (fbm(vec2f(x * 1.1 + fk * 3.1, t), 3) - 0.5) * 0.5;
        let d = uv.y - centre;
        let curtain = exp(-max(d, 0.0) * (5.0 + fk * 2.0)) * exp(-max(-d, 0.0) * 45.0);
        let rays = 0.55 + 0.45 * noise(vec2f(x * 22.0 + fk * 7.0, t * 2.0));
        col += pal(b, 0.4 + 0.18 * fk + 0.15 * (rays - 0.5)) * curtain * rays * 0.5;
    }
    let s = uv * u.res / 3.0;
    let st = step(0.996, hash21(s)) * (0.5 + 0.5 * sin(u.time * 3.0 + hash21(s + 3.0) * 30.0));
    col += vec3f(st * 0.6 * (1.0 - uv.y * 0.3)) * smoothstep(0.3, 1.0, uv.y);
    return vec4f(col, 1.0);
}

fn water_plane(q: vec2f) -> vec2f {
    let z = 1.0 / (1.3 - q.y * 0.75);
    return vec2f((q.x - 0.5) * aspect() * z, z * 2.4) * 2.0;
}
fn water_h(p: vec2f, chop: f32) -> f32 {
    let t = u.time;
    let swell = fbm(p * vec2f(0.8, 2.2) + vec2f(t * 0.04, t * 0.18), 4);
    let ripple = noise(p * vec2f(3.0, 7.5) + vec2f(-t * 0.1, t * 0.5));
    let d = length((p - water_plane(u.mouse)) * vec2f(1.0, 0.55));
    return swell + ripple * 0.18 * chop + sin(d * 24.0 - t * 3.5) * exp(-d * 2.2) * 0.035;
}
fn src_water(uv: vec2f, b: u32, sparkle: f32, chop: f32) -> vec4f {
    let p = water_plane(uv);
    let e = 0.012;
    let h = water_h(p, chop);
    let n = normalize(vec3f(h - water_h(p + vec2f(e, 0.0), chop), h - water_h(p + vec2f(0.0, e), chop), e * 1.6));
    let sun = normalize(vec3f(0.0, 0.75, 0.66));
    let facing = max(dot(reflect(normalize(vec3f(0.0, 0.6, -1.0)), n), sun), 0.0);
    let sheen = pow(facing, 8.0);
    let spark = pow(facing, 160.0);
    let pool = smoothstep(0.35, 0.8, fbm(p * 0.45 + vec2f(0.0, u.time * 0.03), 3));
    var col = pal(b, sat(h * 0.8 - 0.15 + uv.y * 0.25) * 0.6);
    col += pal(b, 0.75) * 0.45 * sheen * (0.1 + 0.5 * pool);
    col += pal(b, 1.0) * spark * (0.3 + 1.2 * pool);
    col = mix(col, pal(b, 0.55), 0.12 * (1.0 - uv.y));
    let gate = smoothstep(0.05, 0.5, sheen * 0.6 + pool * 0.6) * sparkle;
    if (gate > 0.0) {   // only where the water faces the sun
        col += pal(b, 1.0) * glint_field(asp(uv) * 22.0, 1.0) * gate;
    }
    col *= 0.8 + 0.3 * vig(uv, 0.3);
    return vec4f(aces(col * 1.2), 1.0);
}

fn src_cells(uv: vec2f, b: u32, scale: f32, speed: f32) -> vec4f {
    let v = voronoi(asp(uv) * scale, u.time * 0.3 * speed);
    let c = palc(b, u32(v.z * 97.0));
    let edge = smoothstep(0.0, 0.06, v.y - v.x);
    return vec4f(c * (0.82 + 0.18 * edge) * (1.0 - 0.12 * v.x), 1.0);
}

fn src_plasma(uv: vec2f, b: u32, scale: f32, speed: f32) -> vec4f {
    let p = asp(uv) * 3.0 * scale;
    let t = u.time * 0.4 * speed;
    let v = sin(p.x + t) + sin(p.y * 1.3 - t * 1.1) + sin((p.x + p.y) * 0.8 + t * 0.7) + sin(length(p) * 1.4 - t);
    return vec4f(pal(b, tri(v * 0.25 + 1.0)), 1.0);
}

fn src_rings(uv: vec2f, b: u32, count: f32, speed: f32) -> vec4f {
    let r = length(asp(uv)) * count * 2.0 - u.time * 0.25 * speed;
    let i = floor(r);
    let f = fract(r);
    let aa = fwidth(r) * 0.75;
    let a = palc(b, u32(i32(i) & 1023));
    let c = palc(b, u32(i32(i + 1.0) & 1023));
    return vec4f(mix(a, c, smoothstep(1.0 - aa, 1.0, f)), 1.0);
}

fn src_stripes(uv: vec2f, b: u32, count: f32, angle: f32, speed: f32) -> vec4f {
    let a = radians(angle);
    let s = dot(asp(uv), vec2f(cos(a), sin(a))) * count + u.cycle * speed;
    let i = floor(s);
    let aa = fwidth(s) * 0.75;
    let c0 = palc(b, u32(i32(i) & 1023));
    let c1 = palc(b, u32(i32(i + 1.0) & 1023));
    return vec4f(mix(c0, c1, smoothstep(1.0 - aa, 1.0, fract(s))), 1.0);
}

fn src_checker(uv: vec2f, b: u32, count: f32) -> vec4f {
    let p = asp(uv) * count;
    let k = (i32(floor(p.x)) + i32(floor(p.y))) & 1;
    return vec4f(palc(b, u32(k)), 1.0);
}

fn src_dots(uv: vec2f, b: u32, count: f32, size: f32) -> vec4f {
    let p = asp(uv) * count;
    let cell = floor(p);
    let f = fract(p) - 0.5;
    let n = paln(b);
    let pick = select(0u, 1u + u32(hash21(cell) * 97.0) % max(n - 1u, 1u), n > 1u);
    let r = size * 0.5 * (0.85 + 0.15 * sin(u.time + hash21(cell + 5.0) * TAU));
    let aa = fwidth(p.x);
    let m = smoothstep(r + aa, r - aa, length(f));
    return vec4f(mix(palc(b, 0u), palc(b, pick), m), 1.0);
}

fn src_stars(uv: vec2f, density: f32) -> vec4f {
    var a = 0.0;
    for (var k = 0; k < 2; k++) {
        let sc = 40.0 + f32(k) * 50.0;
        let s = asp(uv) * sc;
        let cell = floor(s);
        let h = hash22(cell + f32(k) * 31.0);
        let on = step(1.0 - 0.18 * density, h.x);
        let c = cell + 0.25 + 0.5 * hash22(cell + 9.0);
        let tw = 0.6 + 0.4 * sin(u.time * (1.5 + h.y * 3.0) + h.x * 40.0);
        a += on * tw * glow(length(s - c), 0.05 + h.y * 0.06) * 0.35;
    }
    return vec4f(vec3f(1.0, 0.97, 0.92), sat(a));
}

fn src_glints(uv: vec2f, amount: f32, size: f32) -> vec4f {
    let g = glint_field(asp(uv) * 22.0 / max(size, 0.1), 1.0) * amount;
    return vec4f(vec3f(1.0, 0.94, 0.81), sat(g));
}

/// The k-th of 4 antialiasing samples around plane point `c0`, spaced by the
/// real on-screen pixel footprint — so fractals stay sharp under zoom, dive,
/// kaleido or any other effect that moves coordinates first.
fn aa4(c0: vec2f, dx: vec2f, dy: vec2f, k: i32) -> vec2f {
    let o = vec2f(f32(k & 1), f32(k >> 1u)) - 0.5;
    return c0 + (dx * o.x + dy * o.y) * 0.5;
}

fn fractal_colour(b: u32, n: f32, iters: f32) -> vec3f {
    if (n >= iters) { return palc(b, 0u); }
    return pal(b, 0.1 + 0.9 * tri(sqrt(max(n, 0.0)) * 0.16));
}

// ── fractal speed ────────────────────────────────────────────────────────────
// Three tricks keep fractals fast without changing a pixel of the look:
// 1. points proven inside (closed form, or an orbit caught in a cycle) skip
//    the remaining iterations — inside is where all the time used to go;
// 2. antialiasing is adaptive: one sample, plus three more only where the
//    colour actually changes inside the 2×2 pixel quad (fwidth);
// 3. the power-2 case runs its own loop, free of the general cpow path.

/// True when an orbit has returned *exactly* to a point it visited. A float
/// orbit that repeats exactly is periodic forever, so it can never escape:
/// stopping here gives the very same answer as running every iteration.
fn caught(z: vec2f, old: vec2f) -> bool { return all(z == old); }

/// Does this quad need antialiasing? (Call in uniform control flow.)
fn needs_aa(c: vec3f) -> bool { let e = fwidth(c); return max(e.x, max(e.y, e.z)) > 0.035; }

fn julia_at(b: u32, z0: vec2f, c: vec2f, it: i32) -> vec3f {
    var z = z0;
    var old = z0;
    var lap = 0;
    var win = 8;   // Brent: compare against checkpoints at doubling intervals
    for (var i = 0; i < it; i++) {
        z = vec2f(z.x * z.x - z.y * z.y, 2.0 * z.x * z.y) + c;
        let m = dot(z, z);
        if (m > 256.0) { return fractal_colour(b, f32(i) - log2(log2(m)) + 4.0, f32(it)); }
        if (caught(z, old)) { break; }
        lap += 1;
        if (lap == win) { lap = 0; win = min(win * 2, 1024); old = z; }
    }
    return fractal_colour(b, f32(it), f32(it));
}
fn src_julia(uv: vec2f, b: u32, cx: f32, cy: f32, iters: f32) -> vec4f {
    let it = i32(clamp(iters, 1.0, 5000.0));
    let p0 = plane(uv); let dx = dpdx(p0); let dy = dpdy(p0);
    let c0 = julia_at(b, aa4(p0, dx, dy, 0), vec2f(cx, cy), it);
    if (!needs_aa(c0)) { return vec4f(c0, 1.0); }
    var c = c0;
    for (var k = 1; k < 4; k++) { c += julia_at(b, aa4(p0, dx, dy, k), vec2f(cx, cy), it); }
    return vec4f(c * 0.25, 1.0);
}

fn ship_at(b: u32, c: vec2f, it: i32) -> vec3f {
    var z = vec2f(0.0);
    var old = z;
    var lap = 0;
    var win = 8;   // Brent: compare against checkpoints at doubling intervals
    for (var i = 0; i < it; i++) {
        z = abs(z);
        z = vec2f(z.x * z.x - z.y * z.y, 2.0 * z.x * z.y) + c;
        let m = dot(z, z);
        if (m > 256.0) { return fractal_colour(b, f32(i) - log2(log2(m)) + 4.0, f32(it)); }
        if (caught(z, old)) { break; }
        lap += 1;
        if (lap == win) { lap = 0; win = min(win * 2, 1024); old = z; }
    }
    return fractal_colour(b, f32(it), f32(it));
}
fn src_ship(uv: vec2f, b: u32, iters: f32) -> vec4f {
    let it = i32(clamp(iters, 1.0, 5000.0));
    let p0 = plane(uv); let dx = dpdx(p0); let dy = dpdy(p0);
    let flip = vec2f(1.0, -1.0);
    let c0 = ship_at(b, aa4(p0, dx, dy, 0) * flip, it);
    if (!needs_aa(c0)) { return vec4f(c0, 1.0); }
    var c = c0;
    for (var k = 1; k < 4; k++) { c += ship_at(b, aa4(p0, dx, dy, k) * flip, it); }
    return vec4f(c * 0.25, 1.0);
}
/// z^n for real n. Every orbit starts at z = 0, where atan2(0, 0) is undefined in
/// WGSL (NaN on some GPUs, which would freeze the whole orbit), so 0 is 0.
fn cpow(z: vec2f, n: f32) -> vec2f {
    if (all(z == vec2f(0.0))) { return vec2f(0.0); }
    let r = pow(length(z), n); let a = atan2(z.y, z.x) * n; return r * vec2f(cos(a), sin(a));
}
fn cdiv(a: vec2f, b: vec2f) -> vec2f { return vec2f(a.x * b.x + a.y * b.y, a.y * b.x - a.x * b.y) / dot(b, b); }
fn src_newton(uv: vec2f, b: u32, power: f32) -> vec4f {
    let pw = max(round(power), 2.0);
    var z = plane(uv) * 2.0;
    var it = 0.0;
    for (var i = 0; i < 40; i++) {
        let zn1 = cpow(z, pw - 1.0);
        let f = cpow(z, pw) - vec2f(1.0, 0.0);
        let step = cdiv(f, zn1 * pw);
        z -= step;
        it += 1.0;
        if (dot(step, step) < 1e-8) { break; }
    }
    let k = u32(i32(round(atan2(z.y, z.x) / TAU * pw + pw)) % i32(pw));
    let shade = 0.55 + 0.45 * exp(-it * 0.12);
    return vec4f(palc(b, k) * shade, 1.0);
}

fn src_image(uv: vec2f, t: texture_2d<f32>) -> vec4f {
    let d = vec2f(textureDimensions(t));
    let ia = d.x / d.y;
    let sa = aspect();
    var q = uv;
    if (sa > ia) { q.y = (q.y - 0.5) * ia / sa + 0.5; } else { q.x = (q.x - 0.5) * sa / ia + 0.5; }
    return tx(t, q);
}

// ── domain effects (uv → uv) ─────────────────────────────────────────────────
fn dom_zoom(uv: vec2f, z: f32) -> vec2f { return unasp(asp(uv) / max(z, 1e-4)); }
fn dom_rotate(uv: vec2f, deg: f32) -> vec2f { return unasp(rot(-radians(deg)) * asp(uv)); }
fn dom_spin(uv: vec2f, turns: f32) -> vec2f { return unasp(rot(-u.cycle * turns * TAU) * asp(uv)); }
fn dom_move(uv: vec2f, x: f32, y: f32) -> vec2f { return uv - vec2f(x, y); }
fn dom_scroll(uv: vec2f, x: f32, y: f32) -> vec2f { return uv - vec2f(x, y) * u.cycle; }
fn dom_warp(uv: vec2f, amount: f32, scale: f32) -> vec2f {
    let p = asp(uv) * scale;
    let t = u.time * 0.03;
    let d = vec2f(fbm(p + t, 4), fbm(p + vec2f(5.2, 1.3) - t, 4)) - 0.5;
    return uv + d * amount * 0.6;
}
fn dom_flow(uv: vec2f, amount: f32, speed: f32) -> vec2f {
    let p = asp(uv) * 2.0;
    let t = u.time * 0.12 * speed;
    let q = vec2f(fbm(p + vec2f(t, -t * 0.7), 4), fbm(p + vec2f(1.7 - t * 0.5, 9.2 + t), 4));
    let r = vec2f(fbm(p + q * 1.8 + vec2f(t * 0.6, 3.3), 4), fbm(p + q * 1.8 + vec2f(8.3, -t * 0.4), 4));
    return uv + (r - 0.5) * amount * 0.7;
}
fn dom_swirl(uv: vec2f, amount: f32, radius: f32) -> vec2f {
    let p = asp(uv);
    let k = 1.0 - smoothstep(0.0, max(radius, 1e-3), length(p));
    return unasp(rot(amount * TAU * 0.5 * k * k) * p);
}
fn dom_ripple(uv: vec2f, amount: f32, rings: f32) -> vec2f {
    let p = asp(uv);
    let r = length(p) + 1e-5;
    let d = p / r * sin(r * rings * TAU - u.time * 3.0) * amount * 0.02;
    return unasp(p + d);
}
fn dom_wave(uv: vec2f, amount: f32, count: f32) -> vec2f {
    return uv + vec2f(sin(uv.y * count * TAU + u.time * 1.5), 0.0) * amount * 0.05;
}
fn dom_kaleido(uv: vec2f, sides: f32) -> vec2f {
    let p = asp(uv);
    let seg = TAU / max(round(sides), 1.0);
    var a = atan2(p.y, p.x);
    a = abs((a - seg * floor(a / seg)) - seg * 0.5);
    return unasp(length(p) * vec2f(cos(a), sin(a)));
}
fn dom_mirror(uv: vec2f) -> vec2f { return vec2f(0.5 - abs(uv.x - 0.5), uv.y); }
fn dom_tile(uv: vec2f, count: f32) -> vec2f {
    let p = asp(uv) * max(count, 1.0);
    return unasp(fract(p + 0.5) - 0.5);
}
fn dom_pixelate(uv: vec2f, size: f32) -> vec2f {
    let s = max(size, 1.0);
    return (floor(uv * u.res / s) + 0.5) * s / u.res;
}
fn dom_lens(uv: vec2f, amount: f32, radius: f32) -> vec2f {
    let m = asp(u.mouse);
    let p = asp(uv) - m;
    let k = smoothstep(max(radius, 1e-3), 0.0, length(p));
    return unasp(m + p * (1.0 - amount * 0.6 * k * k));
}
fn dom_polar(uv: vec2f) -> vec2f {
    let p = asp(uv);
    return vec2f(atan2(p.y, p.x) / TAU + 0.5, length(p) * 2.0);
}

// ── colour effects (vec4f → vec4f) ───────────────────────────────────────────
fn col_hue(c: vec4f, deg: f32) -> vec4f {
    var l = srgb_to_oklab(c.rgb);
    l = vec3f(l.x, rot(radians(deg)) * l.yz);
    return vec4f(oklab_to_srgb(l), c.a);
}
fn col_saturate(c: vec4f, k: f32) -> vec4f {
    let l = srgb_to_oklab(c.rgb);
    return vec4f(oklab_to_srgb(vec3f(l.x, l.yz * k)), c.a);
}
fn col_contrast(c: vec4f, k: f32) -> vec4f { return vec4f((c.rgb - 0.5) * k + 0.5, c.a); }
fn col_brightness(c: vec4f, k: f32) -> vec4f { return vec4f(c.rgb + k, c.a); }
fn col_invert(c: vec4f) -> vec4f { return vec4f(1.0 - c.rgb, c.a); }
fn col_tint(c: vec4f, b: u32, amount: f32) -> vec4f {
    let l = srgb_to_oklab(c.rgb);
    let t = palc_lab(b, 0u);
    let m = vec3f(mix(l.x, t.x, amount * 0.3), mix(l.yz, t.yz, sat(amount)));
    return vec4f(oklab_to_srgb(m), c.a);
}
fn col_map(c: vec4f, b: u32) -> vec4f { return vec4f(pal(b, luma(c.rgb)), c.a); }
fn col_posterize(c: vec4f, levels: f32) -> vec4f {
    let l = max(levels, 2.0) - 1.0;
    return vec4f(floor(c.rgb * l + 0.5) / l, c.a);
}
fn col_dither(c: vec4f, uv: vec2f, levels: f32, size: f32) -> vec4f {
    let th = bayer(floor(uv * u.res / max(size, 1.0)));
    let l = max(levels, 2.0) - 1.0;
    return vec4f(clamp(floor(c.rgb * l + th) / l, vec3f(0.0), vec3f(1.0)), c.a);
}
fn col_dither_pal(c: vec4f, uv: vec2f, b: u32, size: f32, spread: f32) -> vec4f {
    let th = bayer(floor(uv * u.res / max(size, 1.0))) - 0.5;
    let q = srgb_to_oklab(c.rgb + th * spread);
    let n = paln(b);
    var best = 0u; var bd = 1e9;
    for (var i = 0u; i < n; i++) {
        let d = q - u.c[b + i].xyz;
        let e = dot(d * d, vec3f(1.0, 0.6, 0.6));
        if (e < bd) { bd = e; best = i; }
    }
    return vec4f(palc(b, best), c.a);
}
fn ht_cell(uv: vec2f, size: f32, angle: f32) -> vec2f {
    let s = max(size, 2.0);
    let r = rot(radians(angle));
    let p = r * (uv * u.res);
    let cell = (floor(p / s) + 0.5) * s;
    return (transpose(r) * cell) / u.res;
}
fn col_halftone(c: vec4f, uv: vec2f, size: f32, angle: f32) -> vec4f {
    let s = max(size, 2.0);
    let p = rot(radians(angle)) * (uv * u.res);
    let f = fract(p / s) - 0.5;
    let r = 0.62 * sqrt(sat(1.0 - luma(c.rgb) * 0.92));
    let aa = 1.0 / s;
    let dotm = smoothstep(r + aa, r - aa, length(f));
    let paper = vec3f(0.965, 0.945, 0.91);
    return vec4f(mix(paper, c.rgb * 0.9, dotm), c.a);
}
fn px_cell(uv: vec2f, size: f32) -> vec2f {
    let s = max(size, 2.0);
    return (floor(uv * u.res / s) + 0.5) * s / u.res;
}
fn glyph(n: u32, p: vec2f) -> f32 {
    let q = floor(p * vec2f(-4.0, 4.0) + 2.5);
    if (q.x < 0.0 || q.x > 4.0 || q.y < 0.0 || q.y > 4.0) { return 0.0; }
    let a = u32(q.x + 5.0 * q.y);
    return f32((n >> a) & 1u);
}
fn col_ascii(c: vec4f, uv: vec2f, size: f32) -> vec4f {
    let s = max(size, 2.0);
    let f = fract(uv * u.res / s) * 2.0 - 1.0;
    let l = luma(c.rgb);
    var n = 0u;
    if (l > 0.08) { n = 4096u; }
    if (l > 0.2) { n = 65600u; }
    if (l > 0.3) { n = 332772u; }
    if (l > 0.4) { n = 15255086u; }
    if (l > 0.5) { n = 23385164u; }
    if (l > 0.6) { n = 15252014u; }
    if (l > 0.7) { n = 13199452u; }
    if (l > 0.8) { n = 11512810u; }
    let on = glyph(n, f);
    let bg = vec3f(0.055, 0.055, 0.063);
    return vec4f(mix(bg, c.rgb * 1.1 + 0.06, on), c.a);
}
fn col_scanlines(c: vec4f, uv: vec2f, amount: f32) -> vec4f {
    let s = 0.5 + 0.5 * sin(uv.y * u.res.y * PI * 0.5);
    return vec4f(c.rgb * (1.0 - amount * s), c.a);
}
fn col_grain(c: vec4f, uv: vec2f, amount: f32) -> vec4f {
    let g = hash21(uv * u.res + fract(u.frame * 0.618) * 911.0) - 0.5;
    return vec4f(c.rgb + g * amount, c.a);
}
/// Three independent hashes of a pixel cell and a seed (pcg3d).
fn pcg3(v0: vec3u) -> vec3u {
    var v = v0 * 1664525u + 1013904223u;
    v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
    v = v ^ (v >> vec3u(16u));
    v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
    return v;
}
/// One noise sample per channel: uniform in [-1, 1], or gaussian with σ = 1 (Box–Muller).
fn noise_cell(cell: vec2f, seed: u32, uniform: bool) -> vec3f {
    let a = pcg3(vec3u(bitcast<vec2u>(vec2i(cell)), seed));
    let r1 = vec3f(a) / 4294967295.0;
    if (uniform) { return r1 * 2.0 - 1.0; }
    let r2 = vec3f(pcg3(a ^ vec3u(0x9e3779b9u))) / 4294967295.0;
    return sqrt(-2.0 * log(max(r1, vec3f(1e-7)))) * cos(TAU * r2);
}
/// Photoshop's Add Noise: amount (uniform spans ±amount, gaussian σ = amount / 2),
/// color 1 = colour noise … 0 = monochromatic, size in pixels (softer clumps above 1,
/// same strength), seed picks the grain. No clock in here, so still noise stays still
/// (and a still picture with it still costs nothing); col_noise adds the motion.
fn col_noise_at(c: vec4f, uv: vec2f, amount: f32, color: f32, size: f32, seed: u32, dist: i32) -> vec4f {
    let uniform = dist == 1;
    let s = max(size, 1.0);
    var n: vec3f;
    if (s <= 1.0) {
        n = noise_cell(floor(uv * u.res), seed, uniform);
    } else {
        // bilinear between cells, renormalised so bigger grain isn't fainter
        let q = uv * u.res / s - 0.5;
        let i = floor(q); let f = q - i;
        let w = vec4f((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);
        n = (noise_cell(i, seed, uniform) * w.x + noise_cell(i + vec2f(1.0, 0.0), seed, uniform) * w.y
           + noise_cell(i + vec2f(0.0, 1.0), seed, uniform) * w.z + noise_cell(i + vec2f(1.0, 1.0), seed, uniform) * w.w)
           / sqrt(dot(w, w));
    }
    let k = select(amount * 0.5, amount, uniform);
    let grain = mix(vec3f(n.x), n, clamp(color, 0.0, 1.0)) * k;
    return vec4f(max(c.rgb + grain, vec3f(0.0)), c.a);
}
/// Moving noise: motion 1 re-rolls the grain 24× a second, like film.
fn col_noise(c: vec4f, uv: vec2f, amount: f32, color: f32, size: f32, motion: f32, dist: i32) -> vec4f {
    let seed = select(0u, u32(floor(u.time * 24.0 * motion)) + 1u, motion > 0.0);
    return col_noise_at(c, uv, amount, color, size, seed, dist);
}
fn col_vignette(c: vec4f, uv: vec2f, amount: f32) -> vec4f {
    return vec4f(c.rgb * mix(1.0, vig(uv, 0.25), amount), c.a);
}
fn col_mask(c: vec4f, uv: vec2f, shape: i32, size: f32, soft: f32) -> vec4f {
    let p = asp(uv);
    var d = length(p) - size;
    switch shape {
        case 1: { let q = abs(p) - vec2f(size); d = length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0); }
        case 2: { d = abs(length(p) - size) - size * 0.18; }
        case 3: { d = (abs(p.x) + abs(p.y)) - size; }
        case 4: { d = length(p) - size * (1.0 + 0.3 * (noise(p * 2.5 + u.time * 0.15) - 0.5)); }
        default: {}
    }
    let e = max(soft, 1.0 / u.res.y);
    return vec4f(c.rgb, c.a * (1.0 - smoothstep(-e, e, d)));
}
fn col_opacity(c: vec4f, a: f32) -> vec4f { return vec4f(c.rgb, c.a * sat(a)); }

// ── texture effects ──────────────────────────────────────────────────────────
/// 13-tap Gaussian along `dir`; radius is a fraction of the screen height.
fn blur_dir(t: texture_2d<f32>, uv: vec2f, dir: vec2f, radius: f32) -> vec4f {
    let st = dir * vec2f(1.0 / aspect(), 1.0) * max(radius, 0.0) / 6.0;
    var acc = vec4f(0.0); var ws = 0.0;
    for (var i = -6; i <= 6; i++) {
        let x = f32(i);
        let w = exp(-x * x / 18.0);
        acc += tx(t, uv + st * x) * w; ws += w;
    }
    return acc / ws;
}
fn bright(t: texture_2d<f32>, uv: vec2f, thr: f32) -> vec4f {
    let o = 1.0 / u.res;
    let c = (tx(t, uv + vec2f(-o.x, -o.y)) + tx(t, uv + vec2f(o.x, -o.y)) + tx(t, uv + vec2f(-o.x, o.y)) + tx(t, uv + o)) * 0.25;
    let k = smoothstep(thr, thr + 0.25, luma(c.rgb));
    return vec4f(c.rgb * k * c.a, 1.0);
}
fn bloom_mix(a: vec4f, b: vec4f, amount: f32) -> vec4f { return vec4f(a.rgb + b.rgb * amount * 1.6, a.a); }
fn glow_mix(a: vec4f, b: vec4f, amount: f32) -> vec4f {
    return vec4f(1.0 - (1.0 - a.rgb) * (1.0 - clamp(b.rgb * amount * 1.4, vec3f(0.0), vec3f(1.0))), a.a);
}
fn chroma(t: texture_2d<f32>, uv: vec2f, amount: f32) -> vec4f {
    let d = (uv - 0.5) * amount * 0.04;
    let r = tx(t, uv + d); let g = tx(t, uv); let b = tx(t, uv - d);
    return vec4f(r.r, g.g, b.b, max(max(r.a, g.a), b.a));
}
fn kuwahara(t: texture_2d<f32>, uv: vec2f, size: f32) -> vec4f {
    let r = (2.0 + max(size, 0.0) * 9.0) / u.res;
    let st = r / 3.0;
    var best = vec4f(0.0); var bv = 1e9;
    for (var q = 0; q < 4; q++) {
        let sx = select(-1.0, 1.0, (q & 1) == 1);
        let sy = select(-1.0, 1.0, (q & 2) == 2);
        var m = vec3f(0.0); var m2 = vec3f(0.0); var a = 0.0;
        for (var j = 0; j < 4; j++) { for (var i = 0; i < 4; i++) {
            let s = tx(t, uv + vec2f(sx * f32(i), sy * f32(j)) * st);
            m += s.rgb; m2 += s.rgb * s.rgb; a += s.a;
        } }
        m /= 16.0; m2 /= 16.0;
        let v = dot(m2 - m * m, vec3f(1.0));
        if (v < bv) { bv = v; best = vec4f(m, a / 16.0); }
    }
    return best;
}
fn trail_mix(a: vec4f, h: vec4f, amount: f32) -> vec4f { return mix(a, h, clamp(amount, 0.0, 0.995)); }

// ── compositing ──────────────────────────────────────────────────────────────
fn blend_layer(a: vec3f, c: vec4f, mode: i32) -> vec3f {
    let b = clamp(c.rgb, vec3f(0.0), vec3f(16.0));
    var r = b;
    switch mode {
        case 1: { r = a + b; }
        case 2: { r = 1.0 - (1.0 - a) * (1.0 - min(b, vec3f(1.0))); }
        case 3: { r = a * b; }
        case 4: { r = select(1.0 - 2.0 * (1.0 - a) * (1.0 - b), 2.0 * a * b, a < vec3f(0.5)); }
        case 5: { r = (1.0 - 2.0 * b) * a * a + 2.0 * b * a; }
        case 6: { r = abs(a - b); }
        default: {}
    }
    return mix(a, r, sat(c.a));
}
/// Clamp and add ±½ LSB of triangular noise, so gradients never band.
fn finish(c: vec3f, px: vec2f) -> vec3f {
    let n = (hash21(px) + hash21(px + vec2f(17.0, 59.0)) - 1.0) / 255.0;
    return clamp(c + n, vec3f(0.0), vec3f(1.0));
}

// ── more sources ─────────────────────────────────────────────────────────────
fn src_caustics(uv: vec2f, b: u32, scale: f32, speed: f32) -> vec4f {
    let t = u.time * 0.18 * speed + 23.0;
    let p = asp(uv) * TAU * 0.9 * scale - 250.0;
    var i = p;
    var c = 1.0;
    let inten = 0.005;
    for (var n = 0; n < 5; n++) {
        let tt = t * (1.0 - 3.5 / f32(n + 1));
        i = p + vec2f(cos(tt - i.x) + sin(tt + i.y), sin(tt - i.y) + cos(tt + i.x));
        c += 1.0 / length(vec2f(p.x / (sin(i.x + tt) / inten), p.y / (cos(i.y + tt) / inten)));
    }
    c /= 5.0;
    c = 1.17 - pow(c, 1.4);
    let v = sat(pow(abs(c), 8.0));
    let depth = fbm(asp(uv) * 1.5 + t * 0.02, 3);
    let base = pal(b, 0.15 + 0.35 * depth);
    return vec4f(mix(base, pal(b, 1.0), v * 0.9), 1.0);
}

fn src_topo(uv: vec2f, b: u32, lines: f32, scale: f32) -> vec4f {
    let p = asp(uv) * scale;
    let t = u.time * 0.02;
    let h = fbm(p + vec2f(t, -t * 0.6) + 0.35 * vec2f(fbm(p * 0.7 + 3.0, 3), fbm(p * 0.7 + 9.0, 3)), 5);
    let x = smoothstep(0.25, 0.8, h);
    let f = x * max(lines, 1.0);
    let d = abs(fract(f - 0.5) - 0.5) / max(fwidth(f), 1e-4);
    let major = select(1.0, 1.7, (i32(round(f)) % 5) == 0);
    let line = 1.0 - smoothstep(0.0, 1.0 * major, d);
    let fill = pal(b, (floor(f) + 0.5) / max(lines, 1.0));
    let ink = pal(b, 0.0) * 0.85;
    return vec4f(mix(fill, ink, line * 0.75), 1.0);
}

fn src_bokeh(uv: vec2f, b: u32, count: f32, size: f32) -> vec4f {
    let p = asp(uv);
    let n = paln(b);
    var col = mix(pal(b, 0.0), pal(b, 0.12), uv.y);
    let k = i32(clamp(count, 1.0, 64.0));
    for (var i = 0; i < k; i++) {
        let fi = f32(i);
        let h = hash22(vec2f(fi, 41.0));
        let h2 = hash22(vec2f(fi, 77.0));
        let c = vec2f((h.x - 0.5) * aspect() * 1.2 + 0.05 * sin(u.time * 0.2 + fi), fract(h.y + u.time * 0.008 * (0.5 + h2.x)) * 1.3 - 0.65);
        let r = size * (0.035 + 0.11 * h2.y * h2.y);
        let d = length(p - c);
        let aa = 1.5 / u.res.y;
        if (d >= r + aa) { continue; }   // most discs are elsewhere: skip the colour maths
        let disc = smoothstep(r + aa, r - aa, d);
        let rim = smoothstep(r * 0.55, r, d) * 0.5 + 0.5;
        let ci = select(pal(b, 1.0), palc(b, 1u + u32(i) % max(n - 1u, 1u)), n > 1u);
        col += ci * disc * rim * (0.35 + 0.45 * h.x) * (0.6 + 0.4 * sin(u.time * 0.5 + fi * 3.1));
    }
    return vec4f(col, 1.0);
}

fn src_marble(uv: vec2f, b: u32, scale: f32, speed: f32) -> vec4f {
    let p = asp(uv) * scale;
    let t = u.time * 0.02 * speed;
    let w = fbm(p * 1.3 + vec2f(t, 0.0), 5);
    let w2 = fbm(p * 2.7 - vec2f(0.0, t) + 7.0, 4);
    // soft clouded ground, mostly light
    let ground = pal(b, 0.62 + 0.38 * smoothstep(0.25, 0.75, w2));
    // veins: thin dark seams where a warped sine crosses zero, with a fainter echo
    let s = sin((p.x * 0.8 - p.y) * 2.2 + w * 10.0);
    let vein = exp(-abs(s) * 28.0) + 0.35 * exp(-abs(sin((p.x + p.y * 0.6) * 3.1 + w2 * 8.0)) * 40.0);
    let c = mix(ground, pal(b, 0.08 + 0.2 * w), sat(vein) * 0.8);
    return vec4f(c, 1.0);
}

fn src_tunnel(uv: vec2f, b: u32, speed: f32, sides: f32) -> vec4f {
    let p = asp(uv);
    let r = length(p) + 1e-4;
    let a = atan2(p.y, p.x) / TAU + 0.5;
    let z = 0.25 / r + u.time * 0.25 * speed;
    let s = floor(a * max(round(sides), 1.0)) + floor(z * 2.0);
    let c = palc(b, u32(i32(s) & 1023));
    let fog = smoothstep(0.0, 0.45, r);
    return vec4f(mix(pal(b, 0.0), c, fog), 1.0);
}

fn src_spot(uv: vec2f, size: f32, soft: f32) -> vec4f {
    let d = length(asp(uv) - asp(u.mouse));
    let a = 1.0 - smoothstep(size * (1.0 - soft), size, d);
    return vec4f(vec3f(1.0), a);
}

fn src_kali(uv: vec2f, b: u32, cx: f32, cy: f32, iters: f32) -> vec4f {
    let it = i32(clamp(iters, 1.0, 64.0));
    var col = vec3f(0.0);
    let p0 = plane(uv); let dx = dpdx(p0); let dy = dpdy(p0);
    for (var k = 0; k < 4; k++) {
        var p = aa4(p0, dx, dy, k);
        var acc = 0.0;
        var last = 0.0;
        for (var i = 0; i < it; i++) {
            p = abs(p) / dot(p, p) - vec2f(cx, cy);
            let m = length(p);
            acc += exp(-4.0 * abs(m - last));
            last = m;
        }
        col += pal(b, tri(acc / f32(it) * 3.0));
    }
    return vec4f(col * 0.25, 1.0);
}

fn tricorn_at(b: u32, c: vec2f, it: i32, density: f32) -> vec3f {
    var z = vec2f(0.0);
    var old = z;
    var lap = 0;
    var win = 8;   // Brent: compare against checkpoints at doubling intervals
    for (var i = 0; i < it; i++) {
        z = vec2f(z.x * z.x - z.y * z.y, -2.0 * z.x * z.y) + c;
        let m = dot(z, z);
        if (m > 256.0) { return fractal_colour_d(b, f32(i) - log2(log2(m)) + 4.0, f32(it), density); }
        if (caught(z, old)) { break; }
        lap += 1;
        if (lap == win) { lap = 0; win = min(win * 2, 1024); old = z; }
    }
    return fractal_colour_d(b, f32(it), f32(it), density);
}
fn src_tricorn(uv: vec2f, b: u32, iters: f32, density: f32) -> vec4f {
    let it = i32(clamp(iters, 1.0, 5000.0));
    let p0 = plane(uv); let dx = dpdx(p0); let dy = dpdy(p0);
    let c0 = tricorn_at(b, aa4(p0, dx, dy, 0), it, density);
    if (!needs_aa(c0)) { return vec4f(c0, 1.0); }
    var c = c0;
    for (var k = 1; k < 4; k++) { c += tricorn_at(b, aa4(p0, dx, dy, k), it, density); }
    return vec4f(c * 0.25, 1.0);
}

/// Like fractal_colour, with palette density (how fast colours cycle).
fn fractal_colour_d(b: u32, n: f32, iters: f32, density: f32) -> vec3f {
    if (n >= iters) { return palc(b, 0u); }
    return pal(b, 0.1 + 0.9 * tri(sqrt(max(n, 0.0)) * 0.16 * density));
}

/// Set by mandel_at: was that sample within a few pixels of the set's edge?
var<private> mandel_near_edge: bool;

/// One sample of the Mandelbrot / multibrot, with distance-estimate shading
/// (edges read as fine, glowing filaments).
fn mandel_at(b: u32, c: vec2f, it: i32, pw: f32, density: f32, px: f32) -> vec3f {
    let fit = f32(it);
    var z = vec2f(0.0);
    var dz = vec2f(0.0);
    var old = z;
    var lap = 0;
    var win = 8;   // Brent: compare against checkpoints at doubling intervals
    var n = fit;
    var m = 0.0;
    if (pw == 2.0) {
        // closed-form interior: the main cardioid and the period-2 bulb
        let x = c.x - 0.25;
        let q = x * x + c.y * c.y;
        if (q * (q + x) <= 0.25 * c.y * c.y || (c.x + 1.0) * (c.x + 1.0) + c.y * c.y <= 0.0625) {
            return palc(b, 0u);
        }
        for (var i = 0; i < it; i++) {
            dz = 2.0 * vec2f(z.x * dz.x - z.y * dz.y, z.x * dz.y + z.y * dz.x) + vec2f(1.0, 0.0);
            z = vec2f(z.x * z.x - z.y * z.y, 2.0 * z.x * z.y) + c;
            m = dot(z, z);
            if (m > 1e4) { n = f32(i) - log2(log2(m) / log2(pw * 2.0 - 2.0 + 1e-3)) + 4.0; break; }
            if (caught(z, old)) { break; }
            lap += 1;
            if (lap == win) { lap = 0; win = min(win * 2, 1024); old = z; }
        }
    } else {
        for (var i = 0; i < it; i++) {
            let zp = cpow(z, pw - 1.0);
            dz = pw * vec2f(zp.x * dz.x - zp.y * dz.y, zp.x * dz.y + zp.y * dz.x) + vec2f(1.0, 0.0);
            z = cpow(z, pw) + c;
            m = dot(z, z);
            if (m > 1e4) { n = f32(i) - log2(log2(m) / log2(pw * 2.0 - 2.0 + 1e-3)) + 4.0; break; }
            if (caught(z, old)) { break; }
            lap += 1;
            if (lap == win) { lap = 0; win = min(win * 2, 1024); old = z; }
        }
    }
    var cc = fractal_colour_d(b, n, fit, density);
    mandel_near_edge = false;
    if (n < fit) {
        // distance estimate → a soft rim of light along the boundary
        let de = 0.5 * sqrt(m) * log(m) / max(length(dz), 1e-9);
        let rim = 1.0 - smoothstep(0.0, 6.0 * px, de);
        cc = mix(cc, pal(b, 1.0), rim * 0.55);
        // filaments thinner than a pixel live here: always antialias them
        mandel_near_edge = de < 8.0 * px;
    }
    return cc;
}
fn src_mandel(uv: vec2f, b: u32, iters: f32, power: f32, density: f32) -> vec4f {
    let it = i32(clamp(iters, 1.0, 5000.0));
    let pw = max(power, 1.01);
    let p0 = plane(uv); let dx = dpdx(p0); let dy = dpdy(p0);
    let px = max(length(dx), length(dy));
    let c0 = mandel_at(b, aa4(p0, dx, dy, 0), it, pw, density, px);
    let edge = mandel_near_edge;
    if (!needs_aa(c0) && !edge) { return vec4f(c0, 1.0); }
    var col = c0;
    for (var k = 1; k < 4; k++) { col += mandel_at(b, aa4(p0, dx, dy, k), it, pw, density, px); }
    return vec4f(col * 0.25, 1.0);
}

// ── more domain effects ──────────────────────────────────────────────────────
/// Push uv down the slope of another picture's brightness (sampled at uv,
/// one step right, one step up): spots become lenses, noise becomes flow.
fn dom_displace(uv: vec2f, c: vec4f, cx: vec4f, cy: vec4f, amount: f32) -> vec2f {
    let l = luma(c.rgb) * c.a;
    let g = vec2f(luma(cx.rgb) * cx.a - l, luma(cy.rgb) * cy.a - l) / 0.004;
    return uv - g * amount * 0.006;
}
fn dom_dive(uv: vec2f, speed: f32, depth: f32) -> vec2f {
    let z = exp2(fract(u.cycle * speed / max(depth, 0.1)) * depth);
    return unasp(asp(uv) / z);
}
fn dom_reeded(uv: vec2f, count: f32, amount: f32) -> vec2f {
    let n = max(count, 1.0);
    let x = uv.x * n;
    let f = fract(x) - 0.5;
    return vec2f((floor(x) + 0.5 + f * (1.0 - amount * 1.6)) / n + f * amount * 0.12, uv.y);
}
fn dom_glass(uv: vec2f, scale: f32, amount: f32) -> vec2f {
    let p = asp(uv) * max(scale, 0.5);
    let i = floor(p);
    let f = fract(p);
    var best = 8.0; var centre = vec2f(0.0);
    for (var y = -1; y <= 1; y++) { for (var x = -1; x <= 1; x++) {
        let g = vec2f(f32(x), f32(y));
        let o = hash22(i + g);
        let d = length(g + o - f);
        if (d < best) { best = d; centre = i + g + o; }
    } }
    let q = centre + (p - centre) * (1.0 - amount);
    return unasp(q / max(scale, 0.5));
}

// ── more colour effects ──────────────────────────────────────────────────────
fn col_reeded(c: vec4f, uv: vec2f, count: f32, amount: f32) -> vec4f {
    let f = fract(uv.x * max(count, 1.0)) - 0.5;
    let shade = 1.0 + amount * (0.18 * f * 2.0 - 0.16 * f * f * 4.0) + amount * 0.12 * pow(1.0 - abs(f + 0.32) * 4.0, 3.0) * step(abs(f + 0.32), 0.25);
    return vec4f(c.rgb * shade, c.a);
}
fn col_glass(c: vec4f, uv: vec2f, scale: f32, amount: f32) -> vec4f {
    let v = voronoi(asp(uv) * max(scale, 0.5), 0.0);
    let edge = 1.0 - smoothstep(0.0, 0.05, v.y - v.x);
    return vec4f(c.rgb * (1.0 + 0.12 * amount) + edge * 0.18 * amount, c.a);
}
fn col_contour(c: vec4f, lines: f32, width: f32) -> vec4f {
    let f = luma(c.rgb) * max(lines, 1.0);
    let d = abs(fract(f - 0.5) - 0.5) / max(fwidth(f), 1e-4);
    let line = 1.0 - smoothstep(0.0, max(width, 0.1), d);
    return vec4f(mix(c.rgb, c.rgb * 0.45, line), c.a);
}
fn col_mask_pic(c: vec4f, m: vec4f, amount: f32) -> vec4f {
    return vec4f(c.rgb, c.a * mix(1.0, luma(m.rgb) * m.a, sat(amount)));
}
fn col_duotone(c: vec4f, b: u32, contrast: f32) -> vec4f {
    let l = sat((luma(c.rgb) - 0.5) * contrast + 0.5);
    return vec4f(oklab_to_srgb(mix(palc_lab(b, 0u), palc_lab(b, paln(b) - 1u), l)), c.a);
}

// ── more texture effects ─────────────────────────────────────────────────────
fn relief(t: texture_2d<f32>, uv: vec2f, amount: f32, angle: f32) -> vec4f {
    // a fixed fraction of the screen, so the look doesn't change with resolution
    let o = vec2f(0.0016 / aspect(), 0.0016);
    let c = tx(t, uv);
    let hx = luma(tx(t, uv + vec2f(o.x, 0.0)).rgb) - luma(tx(t, uv - vec2f(o.x, 0.0)).rgb);
    let hy = luma(tx(t, uv + vec2f(0.0, o.y)).rgb) - luma(tx(t, uv - vec2f(0.0, o.y)).rgb);
    let n = normalize(vec3f(-hx * amount * 6.0, -hy * amount * 6.0, 1.0));
    let a = radians(angle);
    let l = normalize(vec3f(cos(a), sin(a), 0.9));
    let diff = dot(n, l) / l.z;
    let spec = pow(max(dot(reflect(-l, n), vec3f(0.0, 0.0, 1.0)), 0.0), 24.0);
    return vec4f(c.rgb * diff + spec * 0.35 * amount, c.a);
}
fn edges(t: texture_2d<f32>, uv: vec2f, amount: f32) -> vec4f {
    let o = 1.0 / u.res;
    var gx = 0.0; var gy = 0.0;
    for (var j = -1; j <= 1; j++) { for (var i = -1; i <= 1; i++) {
        let l = luma(tx(t, uv + vec2f(f32(i), f32(j)) * o).rgb);
        let wx = f32(i) * select(1.0, 2.0, j == 0);
        let wy = f32(j) * select(1.0, 2.0, i == 0);
        gx += l * wx; gy += l * wy;
    } }
    let e = sat(length(vec2f(gx, gy)) * 3.0);
    let c = tx(t, uv);
    return vec4f(mix(c.rgb, c.rgb * 0.15, e * sat(amount)), c.a);
}

// ── Monet ────────────────────────────────────────────────────────────────────
// Scenes are deliberately simple shapes in Monet's colours; `.brush()` turns
// them into paint. Palette stop order matters for these (see the palettes).

/// Water lilies (Nymphéas): a pond in perspective, willow reflections, pads, blossoms.
/// palette: deep water, mid water, reflection, sky, pad dark, pad light, blossom, blossom light
fn src_lilies(uv: vec2f, b: u32, count: f32, speed: f32) -> vec4f {
    let t = u.time * 0.04 * speed;
    let depth = mix(1.0, 3.2, uv.y);                         // far water is up
    let p = vec2f((uv.x - 0.5) * aspect() * depth, log(depth) * 2.4);
    // water: vertical willow reflections, sky glints, slow ripples
    let sx = uv.x * aspect();
    let refl = fbm(vec2f(sx * 2.6 + t, uv.y * 0.5), 4);           // willows hang straight down
    let ripple = fbm(vec2f(sx * 1.2, p.y * 6.0 + t * 3.0), 3);
    var col = pal(b, 0.05 + 0.32 * smoothstep(0.25, 0.8, refl + (ripple - 0.5) * 0.3));
    col = mix(col, palc(b, 3u), smoothstep(0.6, 0.9, fbm(vec2f(sx * 1.4 - t, uv.y * 2.0 + 4.0), 3)) * 0.6 * (0.3 + uv.y));
    // lily pads: jittered cells, squashed by perspective
    let q = p * vec2f(1.0, 1.8) * max(count, 0.5) * 0.9;
    let i = floor(q);
    for (var y = -1; y <= 1; y++) { for (var x = -1; x <= 1; x++) {
        let c = i + vec2f(f32(x), f32(y));
        let h = hash22(c + 3.1);
        // pads gather in drifts, as in the paintings
        let drift = fbm(c * 0.18 + vec2f(3.0, 7.0), 3);
        if (h.x > smoothstep(0.38, 0.62, drift) * 0.95) { continue; }
        let centre = c + 0.25 + 0.5 * hash22(c + 9.7) + 0.06 * vec2f(sin(t * 3.0 + h.y * 9.0), 0.0);
        let r = 0.28 + 0.22 * h.y;
        let d = q - centre;
        let a = atan2(d.y, d.x);
        // a thin wedge cut toward the centre: the lily pad's notch
        let notch = smoothstep(0.43, 0.47, abs(fract((a - h.x * 20.0) / TAU) - 0.5));
        let shape = length(d) - r * (1.0 - 0.85 * notch);
        let pad = smoothstep(0.02, -0.02, shape);
        let veins = 0.94 + 0.06 * sin(a * 9.0 + h.x * 30.0);
        let green = mix(palc(b, 4u), palc(b, 5u), sat(0.3 + 0.7 * hash21(c + 5.0) - length(d) * 0.8)) * veins;
        col = mix(col, green, pad);
        // a blossom on some pads
        if (h.y > 0.55) {
            let bq = (d - vec2f(0.05, 0.03)) * vec2f(1.0, 1.4);
            let bd = length(bq);
            let petals = 0.75 + 0.25 * cos(atan2(bq.y, bq.x) * 7.0 + h.x * 10.0);
            let rb = 0.07 + 0.07 * petals;
            let bloom = 1.0 - smoothstep(rb - 0.02, rb, bd);
            col = mix(col, mix(palc(b, 6u), palc(b, 7u), 1.0 - smoothstep(0.0, 0.08, bd)), bloom);
        }
    } }
    return vec4f(col, 1.0);
}

/// Poppy field (Coquelicots): sky, a line of trees, a meadow scattered with red.
/// palette: dark green, field green, light field, poppy red, poppy orange, sky, cloud
fn src_poppies(uv: vec2f, b: u32, density: f32, wind: f32) -> vec4f {
    let t = u.time * 0.3 * wind;
    let horizon = 0.56 + 0.04 * (fbm(vec2f(uv.x * 3.0, 1.0), 3) - 0.5);
    // sky with soft clouds
    let cloud = smoothstep(0.45, 0.8, fbm(vec2f(uv.x * 2.0 + u.time * 0.01, uv.y * 4.0), 5));
    var col = mix(palc(b, 5u), palc(b, 6u), cloud * smoothstep(horizon, 1.0, uv.y) + 0.2);
    // tree line
    let trees = horizon + 0.05 + 0.08 * fbm(vec2f(uv.x * 9.0, 3.0), 4);
    col = mix(col, mix(palc(b, 0u), palc(b, 1u), fbm(uv * vec2f(20.0, 8.0), 3)), smoothstep(trees + 0.004, trees - 0.004, uv.y));
    if (uv.y < horizon) {
        // meadow: greens with wind-combed streaks
        let depth = (horizon - uv.y) / horizon;                // 0 far .. 1 near
        let g = fbm(vec2f(uv.x * (8.0 + depth * 6.0) + sin(t + uv.y * 9.0) * 0.1, uv.y * 30.0), 4);
        col = mix(palc(b, 1u), palc(b, 2u), smoothstep(0.3, 0.8, g));
        col = mix(col, palc(b, 0u), smoothstep(0.6, 0.0, depth) * 0.25);
        // poppies: perspective cells, bigger and denser in front
        let s = 10.0 + 70.0 * (1.0 - depth) * (1.0 - depth);
        let q = vec2f(uv.x * aspect() * s, uv.y * s * 1.6);
        let i = floor(q);
        for (var y = -1; y <= 1; y++) { for (var x = -1; x <= 1; x++) {
            let c = i + vec2f(f32(x), f32(y));
            let h = hash22(c);
            if (h.x > density * (0.18 + 0.55 * depth)) { continue; }
            let centre = c + 0.2 + 0.6 * hash22(c + 4.0) + vec2f(0.08 * sin(t * 2.0 + h.y * 7.0), 0.0);
            let d = (q - centre) * vec2f(1.0, 1.25);
            let r = 0.22 + 0.2 * h.y;
            let m = smoothstep(r, r * 0.6, length(d));
            col = mix(col, mix(palc(b, 3u), palc(b, 4u), hash21(c + 2.0)), m);
        }
        }
    }
    return vec4f(col, 1.0);
}

/// Impression, Sunrise: a hazy harbour, an orange sun, its broken reflection, boats.
/// palette: dark, blue grey, haze, light haze, sun, sun light
fn src_sunrise(uv: vec2f, b: u32, sun: f32, haze: f32) -> vec4f {
    let t = u.time * 0.02;
    let horizon = 0.52;
    // haze: horizontal smoky bands
    let smoke = fbm(vec2f(uv.x * 1.5 + t, uv.y * 5.0), 5);
    var col = pal(b, 0.12 + 0.42 * smoothstep(0.2, 0.85, smoke * haze + uv.y * 0.35));
    // ghostly masts and cranes on the far shore
    let mast = smoothstep(0.004, 0.0, abs(fract(uv.x * 7.0 + 0.3) - 0.5) - 0.003) * step(horizon, uv.y) * step(uv.y, horizon + 0.22 * hash11(floor(uv.x * 7.0 + 0.3)));
    col = mix(col, pal(b, 0.15), mast * 0.35);
    // the sun and its glow
    let sp = vec2f(0.43, 0.68);
    let d = length((uv - sp) * vec2f(aspect(), 1.0));
    col = mix(col, palc(b, 4u), smoothstep(0.55, 0.0, d) * 0.3 * sun);
    col = mix(col, palc(b, 5u), smoothstep(0.25, 0.0, d) * 0.25 * sun);
    col = mix(col, mix(palc(b, 4u), palc(b, 5u), 0.4), smoothstep(0.042, 0.036, d) * sun);
    // water: rippled, and a column of broken orange strokes under the sun
    if (uv.y < horizon) {
        let w = fbm(vec2f(uv.x * 3.0, uv.y * 40.0 + t * 20.0), 3);
        col = mix(col, pal(b, 0.2 + 0.25 * w), 0.35);
        let row = floor(uv.y * 55.0);
        let jitter = (hash11(row + floor(t * 30.0)) - 0.5) * 0.05;
        let width = 0.012 + 0.05 * (1.0 - uv.y / horizon) * hash11(row + 7.0);
        let stroke = smoothstep(width, width * 0.5, abs(uv.x - sp.x - jitter)) * step(0.4, hash11(row * 1.7)) * smoothstep(0.0, 0.15, uv.y);
        col = mix(col, mix(palc(b, 4u), palc(b, 5u), hash11(row)), stroke * sun * 0.9);
    }
    // boats: dark little silhouettes with an oarsman
    for (var k = 0; k < 3; k++) {
        let fk = f32(k);
        let bp = vec2f(0.32 + fk * 0.2 + 0.01 * sin(u.time * 0.2 + fk), 0.18 + fk * 0.1);
        let q = (uv - bp) * vec2f(aspect(), 1.0) / (0.9 - fk * 0.25);
        let hull = smoothstep(0.004, 0.0, max(abs(q.x) - 0.06 + q.y * 0.6, abs(q.y + 0.004) - 0.008));
        let man = smoothstep(0.004, 0.0, max(abs(q.x + 0.01) - 0.006, abs(q.y - 0.02) - 0.02));
        col = mix(col, palc(b, 0u), max(hull, man) * 0.85);
    }
    return vec4f(col, 1.0);
}

// ── shapes from album covers ─────────────────────────────────────────────────
/// Hand-drawn orbits: wobbly ellipses around the centre, like an atom.
fn src_orbits(uv: vec2f, b: u32, count: f32, width: f32) -> vec4f {
    let p = asp(uv);
    let n = i32(clamp(count, 1.0, 24.0));
    var a = 0.0;
    var col = palc(b, 0u);
    for (var i = 0; i < n; i++) {
        let fi = f32(i);
        let th = fi * PI / f32(n) + u.time * 0.03 * (0.5 + hash11(fi + 3.0)) + hash11(fi) * 0.4;
        let q = rot(-th) * p;
        let ax = vec2f(0.38 + 0.04 * hash11(fi + 9.0), 0.11 + 0.1 * hash11(fi + 5.0));
        let ang = atan2(q.y / ax.y, q.x / ax.x);
        let wob = 1.0 + 0.035 * (noise(vec2f(ang * 2.0 + fi * 7.0, u.time * 0.2)) - 0.5);
        let d = abs(length(q / (ax * wob)) - 1.0) * min(ax.x, ax.y);
        let w = 0.006 * width * (0.7 + 0.6 * noise(vec2f(ang * 3.0, fi * 13.0)));
        let m = smoothstep(w, w * 0.4, d);
        if (m > a) { col = palc(b, u32(i) % paln(b)); }
        a = max(a, m);
    }
    return vec4f(col, a);
}

/// A starburst of thin ink rays, some tipped with coloured dots.
/// The palette's last stop is the ink; the others colour the dots.
fn src_rays(uv: vec2f, b: u32, count: f32, length_: f32) -> vec4f {
    let p = asp(uv);
    let n = max(round(count), 3.0);
    let a0 = atan2(p.y, p.x) / TAU + 0.5 - u.time * 0.004;
    let px = 1.0 / u.res.y;
    let np = paln(b);
    let ink = palc(b, np - 1u);
    var col = ink;
    var alpha = 0.0;
    for (var j = -1; j <= 1; j++) {
        let k = floor(a0 * n) + f32(j);
        let h = hash11(k + 1.0);
        let ang = ((k + 0.5) / n - 0.5 + u.time * 0.004) * TAU;
        let dir = vec2f(cos(ang), sin(ang));
        let len = length_ * (0.16 + 0.3 * h);
        let along = dot(p, dir);
        let perp = abs(dot(p, vec2f(-dir.y, dir.x)));
        let w = max(0.0012, px * 0.9);
        let line = smoothstep(w + px, w, perp) * smoothstep(0.03, 0.05, along) * step(along, len);
        if (line > alpha) { alpha = line; col = ink; }
        if (hash11(k + 5.0) > 0.35) {
            let r = 0.012 + 0.012 * hash11(k + 8.0);
            let dd = length(p - dir * len);
            let dotm = smoothstep(r + px, r - px, dd);
            if (dotm > 0.0) {
                col = mix(col, palc(b, u32(k + 64.0) % max(np - 1u, 1u)), dotm);
                alpha = max(alpha, dotm);
            }
        }
    }
    return vec4f(col, alpha);
}

// ── painterly & cover effects ────────────────────────────────────────────────
/// Cut the picture into a grid and swap some tiles (a new shuffle every 1/rate cycles).
fn dom_shuffle(uv: vec2f, count: f32, amount: f32, rate: f32) -> vec2f {
    let n = vec2f(max(round(count * aspect()), 1.0), max(round(count), 1.0));
    let q = uv * n;
    let cell = floor(q);
    let seed = floor(u.cycle * rate);
    let h = hash22(cell + seed * 17.0);
    if (h.x < amount) {
        let dest = floor(hash22(cell * 3.1 + seed * 7.0 + 11.0) * n);
        return (dest + fract(q)) / n;
    }
    return uv;
}
/// Droste: the picture repeats inside itself, forever, optionally zooming in.
fn dom_droste(uv: vec2f, scale: f32, speed: f32) -> vec2f {
    let s = clamp(scale, 0.05, 0.95);
    var n = (uv - 0.5) * 2.0;
    n *= pow(s, fract(u.cycle * speed));
    let m = max(max(abs(n.x), abs(n.y)), 1e-5);
    let level = floor(log(m) / log(s));
    n = n / pow(s, level);
    return n * 0.5 + 0.5;
}
fn col_canvas(c: vec4f, uv: vec2f, amount: f32) -> vec4f {
    let q = uv * u.res / 2.6;   // threads every ~2.6 px: crisp, never aliased
    let wx = 0.5 + 0.5 * sin(q.x * PI) * sign(sin(q.y * PI * 0.5));
    let wy = 0.5 + 0.5 * sin(q.y * PI) * sign(sin(q.x * PI * 0.5 + 1.0));
    let weave = mix(wx, wy, step(0.5, fract(floor(q.x) * 0.5 + floor(q.y) * 0.5)));
    let fibre = noise(q * vec2f(0.15, 2.0)) * 0.5 + noise(q * vec2f(2.0, 0.15)) * 0.5;
    let k = (weave * 0.7 + fibre * 0.3 - 0.5) * amount * 0.18;
    return vec4f(c.rgb * (1.0 + k) + k * 0.15, c.a);
}

/// Impressionist brushwork: three layers of oriented strokes (big to small),
/// each following the picture's contours, with broken colour and bristles.
fn brush(t: texture_2d<f32>, uv: vec2f, size: f32, chaos: f32, angle: f32) -> vec4f {
    let base = tx(t, uv);
    var col = base.rgb * 0.9;
    for (var layer = 0; layer < 3; layer++) {
        let fl = f32(layer);
        let s = max(size, 0.05) * 0.045 * (1.35 - 0.3 * fl);
        let off = vec2f(fl * 0.37, fl * 0.71);
        let q = asp(uv) / s + off;
        let cell = floor(q);
        for (var y = -1; y <= 1; y++) { for (var x = -1; x <= 1; x++) {
            let c = cell + vec2f(f32(x), f32(y));
            let h = hash22(c + fl * 17.0);
            let centre = c + h;
            // A stroke reaches at most ~1.03 cells (length ≤ 0.95, plus
            // bristle wobble): skip the rest before any texture work.
            let d0 = q - centre;
            if (dot(d0, d0) > 1.1025) { continue; }
            let cuv = unasp((centre - off) * s);
            // stroke direction: along the contour, nudged by a flow field
            let e = s * 0.8;
            let ex = vec2f(e / aspect(), 0.0);
            let ey = vec2f(0.0, e);
            let gx = luma(tx(t, cuv + ex).rgb) - luma(tx(t, cuv - ex).rgb);
            let gy = luma(tx(t, cuv + ey).rgb) - luma(tx(t, cuv - ey).rgb);
            let flow = (noise(centre * 0.12 + fl * 5.0) - 0.5) * TAU * chaos;
            let g = length(vec2f(gx, gy));
            // (atan2(0, 0) is NaN on some GPUs, and NaN survives mix() at weight 0)
            let along_contour = select(0.0, atan2(gx, -gy), g > 1e-6);
            let ang = mix(radians(angle) + flow, along_contour + flow * 0.25, smoothstep(0.01, 0.06, g));
            let d = rot(-ang) * (q - centre);
            let len = 0.6 + 0.35 * h.x;   // ≤ 1 cell: never clipped by the 3×3 search
            let wid = 0.2 + 0.1 * h.y;
            let k = (d.x / len) * (d.x / len) + (d.y / wid) * (d.y / wid);
            let bristle = noise(vec2f(d.x * 2.5 + h.x * 9.0, d.y * 22.0));
            let a = smoothstep(1.0, 0.7, k + (bristle - 0.5) * 0.35);
            // broken colour: each stroke's own small shift in tone and hue
            var sc = srgb_to_oklab(tx(t, cuv).rgb);
            sc = vec3f(sc.x * (0.93 + 0.14 * h.y), sc.yz * 1.08 + (hash22(c + 31.0 + fl) - 0.5) * 0.05);
            let stroke = oklab_to_srgb(sc) * (0.93 + 0.1 * bristle);
            col = mix(col, stroke, a);
        } }
    }
    return vec4f(col, base.a);
}
fn zoomblur(t: texture_2d<f32>, uv: vec2f, amount: f32, x: f32, y: f32) -> vec4f {
    let c = vec2f(x, y);
    var acc = vec4f(0.0);
    for (var i = 0; i < 24; i++) {
        let k = f32(i) / 23.0;
        acc += tx(t, mix(uv, c, k * amount * 0.25));
    }
    return acc / 24.0;
}


// ── riso & posters ───────────────────────────────────────────────────────────
/// Stochastic stipple: each pixel is one ink or another, chosen by a fixed
/// random threshold — the grain of a riso print or an engraving. With more
/// than two stops, it stipples between neighbouring inks.
fn col_stipple(c: vec4f, uv: vec2f, b: u32, size: f32, contrast: f32) -> vec4f {
    let px = floor(uv * u.res / max(size, 1.0));
    let r = hash21(px + vec2f(0.5, 17.5));
    let l = sat((luma(c.rgb) - 0.5) * contrast + 0.5);
    let n = paln(b);
    if (n < 2u) { return vec4f(palc(b, 0u), c.a); }
    let t = l * f32(n - 1u);
    let i = min(u32(t), n - 2u);
    let k = select(i, i + 1u, t - f32(i) > r);
    return vec4f(palc(b, k), c.a);
}

/// Graph paper: the first stop is the paper, the second the ruling.
fn src_grid(uv: vec2f, b: u32, count: f32, width: f32) -> vec4f {
    let p = asp(uv) * max(count, 1.0);
    let fw = fwidth(p);
    let d = abs(fract(p + 0.5) - 0.5) / max(fw, vec2f(1e-5));
    let line = 1.0 - smoothstep(width * 0.5, width * 0.5 + 1.0, min(d.x, d.y));
    return vec4f(mix(palc(b, 0u), palc(b, 1u), line), 1.0);
}

/// A sun on the horizon: banded sky, the disc half-sunk, light streaking the water.
fn src_sun(uv: vec2f, b: u32, height: f32, size: f32) -> vec4f {
    let horizon = 0.42;
    let p = asp(uv);
    let sc = vec2f(0.0, horizon - 0.5 + height * 0.18);
    let r = 0.3 * size;
    let d = length(p - sc);
    var t = 0.38 + 0.3 * smoothstep(horizon, 1.0, uv.y);            // sky: lighter up high
    let disc = smoothstep(r + 0.004, r - 0.004, d);
    let above = step(horizon, uv.y);
    t = mix(t, 1.0, disc * above);
    if (uv.y < horizon) {
        // water: darker, with broken horizontal reflections under the sun
        t = 0.3 + 0.08 * noise(vec2f(uv.x * 4.0, uv.y * 30.0));
        let row = floor(uv.y * 70.0);
        let w = r * (0.4 + 0.9 * hash11(row)) * (0.6 + 0.4 * uv.y / horizon);
        let streak = smoothstep(w, w * 0.6, abs(p.x - sc.x + (hash11(row + 3.0) - 0.5) * 0.04)) * step(0.35, hash11(row * 1.3));
        t = mix(t, 0.92, streak * smoothstep(0.0, 0.25, uv.y));
    }
    return vec4f(pal(b, t), 1.0);
}

/// Peonies: lush, ruffled blooms in layered petal rings over dark foliage,
/// shaded like an engraving (made to be stippled).
///
/// Each bloom is up to six rings of petals, outside in. A pixel belongs to the
/// innermost ring that covers it; its tone comes from where it sits across
/// that petal's *visible band* — shadowed at the base, where the next ring
/// overlaps, lit at the curled rim — with dark gaps between petals.
fn petal_edge(a: f32, k: f32, rings: f32, seed: f32) -> f32 {
    let ring = 1.0 - k / rings * 0.86;
    let m = 5.0 + k * 1.7;
    let lobe = abs(cos((a * m + hash11(seed + k * 7.0) * TAU) * 0.5));
    let ruffle = 0.045 * (noise(vec2f(a * 8.0 + k * 3.1, seed * 0.37 + k)) - 0.5);
    return ring * (0.7 + 0.3 * pow(lobe, 0.45)) + ruffle;
}
fn src_peonies(uv: vec2f, b: u32, count: f32, size: f32) -> vec4f {
    let p = asp(uv) * max(count, 0.3) * 2.3;
    let t = u.time * 0.03;
    // foliage: deep shadow, then broad leaves in mid tones with dark veins
    var shade = 0.07 + 0.08 * fbm(p * 0.9 + 3.0, 3);
    let lp = p * 1.15;
    let lc = floor(lp);
    for (var y = -1; y <= 1; y++) { for (var x = -1; x <= 1; x++) {
        let c = lc + vec2f(f32(x), f32(y));
        if (hash21(c + 11.0) > 0.9) { continue; }
        let centre = c + 0.2 + 0.6 * hash22(c + 5.0);
        let th = hash21(c + 2.0) * TAU;
        let q = rot(-th) * (lp - centre);
        let len = 0.75 + 0.3 * hash21(c + 9.0);
        let u1 = q.x / len;
        if (abs(u1) > 1.0) { continue; }
        let half = 0.24 * (1.0 - u1 * u1) * (0.8 + 0.2 * sin(u1 * 9.0 + th));   // serrated
        if (abs(q.y) < half) {
            let across = abs(q.y) / half;
            let vein = smoothstep(0.02, 0.0, abs(q.y)) + 0.5 * smoothstep(0.015, 0.0, abs(fract(u1 * 4.0 + abs(q.y) * 6.0) - 0.5) * 0.12);
            let lit = select(0.4, 0.6, q.y > 0.0);
            shade = lit * (1.0 - 0.45 * across * across) + 0.05 * noise(q * vec2f(3.0, 40.0)) - vein * 0.13;
        }
    } }
    // blooms: the nearest-on-top flower wins each pixel
    let ci = floor(p);
    var top = -1.0;
    for (var y = -1; y <= 1; y++) { for (var x = -1; x <= 1; x++) {
        let cell = ci + vec2f(f32(x), f32(y));
        if (hash21(cell + 7.0) < 0.14) { continue; }               // a few gaps let the leaves show
        let h = hash22(cell);
        let z = hash21(cell + 1.7);
        if (z <= top) { continue; }
        let bud = hash21(cell + 21.0) < 0.22;
        // centres stay within 0.3..0.7 of their cell and radii ≤ 1.25 cells,
        // so the 3×3 search always sees every bloom that reaches this pixel
        let centre = cell + 0.3 + 0.4 * hash22(cell + 3.0) + 0.02 * vec2f(sin(t + h.x * 6.0), cos(t * 1.3 + h.y * 6.0));
        let big = min(select((0.62 + 0.3 * h.x) * size, (0.26 + 0.1 * h.x) * size, bud), 1.25);
        let th = h.y * TAU;
        var q = rot(-th) * (p - centre);
        let tilt = select(0.62 + 0.38 * hash21(cell + 4.0), 0.75, bud);
        q.y /= tilt;                                                   // seen a little from the side
        let r = length(q) / big;
        if (r > 1.08) { continue; }
        let a = atan2(q.y, q.x);
        let seed = cell.x * 13.0 + cell.y * 7.0;
        let rings = select(6.0, 3.0, bud);
        // innermost ring covering this pixel
        var k = -1.0;
        var outer = 0.0;
        var inner = 0.0;
        for (var i = 0; i < 6; i++) {
            let fi = f32(i);
            if (fi >= rings) { break; }
            let e = petal_edge(a, fi, rings, seed);
            if (r < e) { k = fi; outer = e; }
        }
        if (k < 0.0) { continue; }
        if (k + 1.0 < rings) { inner = petal_edge(a, k + 1.0, rings, seed); }
        // position across the petal's visible band: 0 at its shadowed base, 1 at its rim
        let band = sat((r - inner) / max(outer - inner, 1e-3));
        let gap = 1.0 - smoothstep(0.0, 0.35, abs(cos((a * (5.0 + k * 1.7) + hash11(seed + k * 7.0) * TAU) * 0.5)));
        var s = 0.16 + 0.78 * pow(band, 0.7);                         // base shadow → lit rim
        s *= 1.0 - 0.55 * gap;                                         // dark between petals
        s -= 0.25 * smoothstep(0.93, 1.0, band);                       // a fine dark rim line
        s += 0.08 * (noise(vec2f(a * 46.0 + k * 9.0, r * 7.0)) - 0.5); // engraved striations
        s *= 1.0 - 0.06 * k;
        // the heart: dense, dark, tightly ruffled
        let heart = smoothstep(0.2, 0.0, r) * select(1.0, 0.0, bud);
        s = mix(s, 0.12 + 0.3 * noise(q * 48.0 / big), heart * 0.85);
        shade = sat(s);
        top = z;
    } }
    return vec4f(pal(b, sat(shade)), 1.0);
}

/// Put the picture in a card: map it into a rectangle (x, y = lower-left,
/// w, h — all fractions of the screen) with rounded corners and an optional border.
fn dom_place(uv: vec2f, x: f32, y: f32, w: f32, h: f32) -> vec2f {
    card_aspect = (u.res.x * max(w, 1e-4)) / (u.res.y * max(h, 1e-4));
    return (uv - vec2f(x, y)) / vec2f(max(w, 1e-4), max(h, 1e-4));
}
fn col_place(c: vec4f, uv: vec2f, x: f32, y: f32, w: f32, h: f32, radius: f32, border: f32, b: u32) -> vec4f {
    card_aspect = 0.0;   // back to the screen for everything after this card
    let px = uv * u.res;
    let lo = vec2f(x, y) * u.res;
    let half = vec2f(w, h) * u.res * 0.5;
    let rr = min(radius * u.res.y, min(half.x, half.y));
    let q = abs(px - lo - half) - half + rr;
    let d = length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - rr;
    let inside = 1.0 - smoothstep(-0.5, 0.5, d);
    var col = c.rgb;
    if (border > 0.0) {
        let ring = 1.0 - smoothstep(border - 0.5, border + 0.5, -d);
        col = mix(col, palc(b, 0u), ring);
    }
    return vec4f(col, c.a * inside);
}
