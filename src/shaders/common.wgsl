// Shared prelude — prepended to every WGSL module by src/shader.rs.

struct Uniforms {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    resolution: vec2<f32>,
    time: f32,
    particle_count: f32,
    /// Band envelopes and beat pulse: (low, mid, high, beat).
    audio: vec4<f32>,
    /// Song position: (row, pattern, beats elapsed, phase within the bar).
    music: vec4<f32>,
    /// FFT spectrum, 16 log-spaced bands packed four to a vector.
    bands: array<vec4<f32>, 4>,
    /// Debug switches: (band overlay, unused, unused, unused).
    debug: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

// Spectrum band `i` (0 = ~40Hz, 15 = ~16kHz).
fn band(i: u32) -> f32 {
    return u.bands[i / 4u][i % 4u];
}

const PI: f32 = 3.14159265;
const SPHERE_RADIUS: f32 = 1.0;

// --- fullscreen triangle ------------------------------------------------

struct FullscreenOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Single oversized triangle covering the viewport; uv is in NDC (-1..1).
fn fullscreen_vertex(vi: u32) -> FullscreenOut {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = 1.0 - f32(vi & 2u) * 2.0;
    var out: FullscreenOut;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

// Depth-buffer value for a world-space point, so the raymarch can write into
// the same depth buffer rasterized geometry uses.
fn clip_depth(p: vec3<f32>) -> f32 {
    let clip = u.view_proj * vec4<f32>(p, 1.0);
    return clip.z / clip.w;
}

// Reconstruct a world-space ray direction for an NDC position.
fn camera_ray(ndc: vec2<f32>) -> vec3<f32> {
    let far = u.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    return normalize(far.xyz / far.w - u.camera_pos.xyz);
}

// --- sdf toolbox --------------------------------------------------------

fn sd_sphere(p: vec3<f32>, r: f32) -> f32 {
    return length(p) - r;
}

fn sd_round_box(p: vec3<f32>, b: vec3<f32>, r: f32) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0) - r;
}

// Polynomial smooth minimum — the metaball blend.
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

// Cubic smooth minimum. Fatter, rounder fillet than the polynomial version —
// this is what reads as surface tension when two blobs neck together.
fn smin_cubic(a: f32, b: f32, k: f32) -> f32 {
    let h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * h * k / 6.0;
}

fn rot_y(a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat3x3<f32>(vec3<f32>(c, 0.0, -s), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(s, 0.0, c));
}

fn rot_x(a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, c, s), vec3<f32>(0.0, -s, c));
}

// --- shared scene description -------------------------------------------

// Ray/sphere intersection. Returns (t_near, t_far), or (-1, -1) on a miss.
fn intersect_sphere(ro: vec3<f32>, rd: vec3<f32>, r: f32) -> vec2<f32> {
    let b = dot(ro, rd);
    let c = dot(ro, ro) - r * r;
    let h = b * b - c;
    if h < 0.0 {
        return vec2<f32>(-1.0);
    }
    let sh = sqrt(h);
    return vec2<f32>(-b - sh, -b + sh);
}

// --- the blobs ----------------------------------------------------------
//
// Centre positions live here rather than in scene.wgsl because the particles
// need to feel the same masses the raymarcher draws.

const BLOB_COUNT: u32 = 6u;

fn blob_center(i: u32, t: f32) -> vec3<f32> {
    let fi = f32(i);
    return vec3<f32>(
        sin(t * 0.53 + fi * 1.7) * 0.40,
        cos(t * 0.41 + fi * 2.3) * 0.34,
        sin(t * 0.61 + fi * 0.9) * 0.40,
    );
}

/// Roughly where a blob's surface sits: radius plus the blend fillet.
const BLOB_SURFACE: f32 = 0.55;
/// How far past the surface a blob still deflects passing beads.
const BLOB_WAKE: f32 = 0.40;
/// Extra push on the beat, as a multiple of the resting deflection.
const BLOB_BEAT_GAIN: f32 = 1.6;

// How the blobs shove the particles around.
//
// Two parts: a standing deflection, so beads visibly flow around the blobs
// instead of sailing through them, and a swell of that same deflection on each
// beat. Deliberately not an inverse-square force — that grows without bound
// near the centre, and a bead displaced past a blob flips its force direction
// and chatters back and forth.
fn blob_gravity(p: vec3<f32>, t: f32) -> vec3<f32> {
    let swell = 1.0 + u.audio.w * BLOB_BEAT_GAIN;

    var offset = vec3<f32>(0.0);
    for (var i = 0u; i < BLOB_COUNT; i = i + 1u) {
        let d = p - blob_center(i, t);
        let r = max(length(d), 1.0e-4);
        let dir = d / r;

        // Hard exclusion inside the surface, plus a soft wake outside it that
        // fades with distance.
        let exclusion = max(BLOB_SURFACE - r, 0.0);
        let wake = BLOB_WAKE * exp(-max(r - BLOB_SURFACE, 0.0) / BLOB_WAKE);

        // Never displace a bead further than it is from the centre, so it can
        // be pushed away but never dragged through.
        offset = offset + dir * min((exclusion + wake * 0.45) * swell, r * 0.85);
    }
    return offset;
}

// Particles are beads strung along a handful of closed space curves. Each curve
// is a sum of circles at 1x, 3x, 9x, 27x frequency on tilted planes — an
// epicycle series, so the path is spline-smooth but self-similar at every
// scale, and each harmonic drifts at its own rate as time advances.
const CURVE_COUNT: u32 = 5u;
const HARMONICS: i32 = 4;

fn epicycle(s: f32, seed: f32, t: f32) -> vec3<f32> {
    var p = vec3<f32>(0.0);
    var amp = 1.0;
    var freq = 1.0;

    for (var k = 0; k < HARMONICS; k = k + 1) {
        let fk = f32(k);
        // Each harmonic spins in its own plane, defined by an axis pair.
        let axis = normalize(vec3<f32>(
            sin(fk * 2.1 + seed * 3.0),
            cos(fk * 1.7 + seed * 1.1),
            sin(fk * 0.9 + seed * 1.7) + 0.3,
        ));
        let co_axis = normalize(cross(axis, vec3<f32>(0.0, 1.0, 0.13)));

        // Integer frequency ratios keep the curve closed, so beads flow around
        // it forever without a seam.
        let phase = 2.0 * PI * freq * s + t * (0.25 + 0.19 * fk) + seed * 6.2831 + fk * 1.7;
        p = p + amp * (cos(phase) * axis + sin(phase) * co_axis);

        amp = amp * 0.42;
        freq = freq * 3.0;
    }
    return p;
}

// Where particle `i` sits at time `t`.
fn particle_pos(i: u32, t: f32) -> vec3<f32> {
    let curve = i % CURVE_COUNT;
    let along = f32(i / CURVE_COUNT);
    let per_curve = max(u.particle_count / f32(CURVE_COUNT), 1.0);

    // Beads slide along their curve at slightly different speeds per curve.
    let s = fract(along / per_curve + t * 0.014 * (1.0 + f32(curve) * 0.23));
    let seed = f32(curve) * 0.37;

    let base = rot_y(t * 0.06) * (epicycle(s, seed, t) * 1.15);
    return base + blob_gravity(base, t);
}

// --- environment --------------------------------------------------------

const SUN_DIR: vec3<f32> = vec3<f32>(0.5, 0.72, -0.48);

// --- procedural noise ---------------------------------------------------

fn hash13(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q = q + dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = mix(hash13(i + vec3<f32>(0.0, 0.0, 0.0)), hash13(i + vec3<f32>(1.0, 0.0, 0.0)), w.x);
    let b = mix(hash13(i + vec3<f32>(0.0, 1.0, 0.0)), hash13(i + vec3<f32>(1.0, 1.0, 0.0)), w.x);
    let c = mix(hash13(i + vec3<f32>(0.0, 0.0, 1.0)), hash13(i + vec3<f32>(1.0, 0.0, 1.0)), w.x);
    let d = mix(hash13(i + vec3<f32>(0.0, 1.0, 1.0)), hash13(i + vec3<f32>(1.0, 1.0, 1.0)), w.x);
    return mix(mix(a, b, w.y), mix(c, d, w.y), w.z);
}

fn fbm(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i = i + 1) {
        sum = sum + amp * vnoise(q);
        q = q * 2.03;
        amp = amp * 0.5;
    }
    return sum;
}

// --- the room -----------------------------------------------------------
//
// The whole scene sits inside a large sphere. The camera is inside it, so this
// is both the backdrop and what the glossy sphere reflects.

const ROOM_RADIUS: f32 = 9.0;

/// The one warm accent in an otherwise cold scene.
const VEIN_COLOR: vec3<f32> = vec3<f32>(1.0, 0.26, 0.03);
/// Hotter core, so the thinnest filaments read white-hot rather than flat.
const VEIN_CORE: vec3<f32> = vec3<f32>(1.0, 0.72, 0.35);
/// Raise to widen the veins, lower to make them rarer and finer.
const VEIN_THRESHOLD: f32 = 0.87;
const VEIN_INTENSITY: f32 = 0.9;
/// Direction the pulses of energy travel through the vein network.
const VEIN_FLOW_DIR: vec3<f32> = vec3<f32>(0.35, 1.0, 0.2);
/// Surges per beat. Fractions give a surge every N beats.
const VEIN_SURGES_PER_BEAT: f32 = 0.5;

// Height field on the shell's surface. Domain-warped by time, which is what
// makes the walls look like they're slowly morphing.
fn room_height(dir: vec3<f32>) -> f32 {
    let t = u.time * 0.08;
    let warp = vec3<f32>(
        fbm(dir * 1.3 + vec3<f32>(t, 0.0, 0.0)),
        fbm(dir * 1.3 + vec3<f32>(0.0, t, 5.2)),
        fbm(dir * 1.3 + vec3<f32>(3.7, 0.0, t)),
    );
    return fbm(dir * 3.4 + warp * 1.6 + vec3<f32>(0.0, t * 0.5, 0.0));
}

// Where a ray leaves the room shell, seen from inside.
fn room_hit(ro: vec3<f32>, rd: vec3<f32>) -> f32 {
    let b = dot(ro, rd);
    let c = dot(ro, ro) - ROOM_RADIUS * ROOM_RADIUS;
    return -b + sqrt(max(b * b - c, 0.0));
}

// Shade the inside of the shell along `rd`.
fn environment(rd: vec3<f32>) -> vec3<f32> {
    let ro = u.camera_pos.xyz;
    let p = ro + rd * room_hit(ro, rd);
    let dir = p / ROOM_RADIUS;
    let n = -dir; // inward-facing

    // Bump mapping: perturb the normal by the height field's tangent gradient.
    // This is the modern stand-in for an emboss map — no texture, just noise.
    let t1 = normalize(cross(select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9), n));
    let t2 = cross(n, t1);
    let e = 0.035;
    let h = room_height(dir);
    let dh1 = (room_height(dir + t1 * e) - h) / e;
    let dh2 = (room_height(dir + t2 * e) - h) / e;
    let nb = normalize(n - (t1 * dh1 + t2 * dh2) * 0.14);

    // Albedo and roughness both driven by the same field, so the raised areas
    // read as polished and the recesses as matte.
    let albedo = mix(vec3<f32>(0.14, 0.17, 0.25), vec3<f32>(0.48, 0.55, 0.72), smoothstep(0.30, 0.78, h));
    let rough = clamp(0.85 - h * 0.7, 0.08, 0.95);

    let l = normalize(SUN_DIR);
    let v = -rd;
    let hv = normalize(l + v);

    let diff = max(dot(nb, l), 0.0);
    let gloss = exp2(mix(3.0, 11.0, 1.0 - rough));
    let spec = pow(max(dot(nb, hv), 0.0), gloss) * (1.0 - rough) * 3.0;
    let fres = pow(1.0 - max(dot(nb, v), 0.0), 5.0) * (1.0 - rough);

    // Cool ambient from "above" plus a warm bounce, so the walls have depth
    // even where the key light doesn't reach.
    let ambient = mix(vec3<f32>(0.10, 0.13, 0.20), vec3<f32>(0.30, 0.35, 0.48), nb.y * 0.5 + 0.5);

    var color = albedo * (ambient + diff * vec3<f32>(0.75, 0.85, 1.05));
    color = color + vec3<f32>(0.7, 0.8, 1.0) * spec;
    color = color + vec3<f32>(0.25, 0.4, 0.8) * fres * 0.5;

    // Molten veins running through the walls. The whole scene is cool blue, so
    // these sit opposite it on the wheel and are the only warm thing in frame.
    //
    // Ridged noise: folding the height field at its midline turns smooth blobs
    // into sharp creases, which is what gives filaments rather than patches.
    let ridge = 1.0 - abs(h * 2.0 - 1.0);
    let vein = pow(smoothstep(VEIN_THRESHOLD, 1.0, ridge), 2.5);

    // Energy travelling through the network. The phase runs on beats rather
    // than seconds, so surges arrive with the music instead of drifting past it.
    let beats = u.music.z;
    let sweep = dot(dir, normalize(VEIN_FLOW_DIR)) * 5.0 - beats * PI * 2.0 * VEIN_SURGES_PER_BEAT;
    let flow = 0.35 + 0.65 * pow(0.5 + 0.5 * sin(sweep), 3.0);

    // The network is also a spectrum: veins low in the room answer to the bass,
    // veins overhead to the hats, so different parts light with different parts
    // of the mix.
    let band_index = u32(clamp((dir.y * 0.5 + 0.5) * 16.0, 0.0, 15.0));
    let energy = 0.3 + 1.3 * band(band_index);

    // Fine flicker keyed off the height field, so neighbouring filaments don't
    // breathe in lockstep.
    let flicker = 0.85 + 0.15 * sin(h * 28.0 + beats * 3.0);

    // Deep orange body with a hotter core, kept above 1.0 in the brightest
    // filaments so they still read as emissive through the tonemap.
    let heat = VEIN_COLOR * vein + VEIN_CORE * pow(vein, 4.0);
    color = color + heat * VEIN_INTENSITY * flow * flicker * energy * (0.8 + u.audio.w * 0.7);

    return color;
}
