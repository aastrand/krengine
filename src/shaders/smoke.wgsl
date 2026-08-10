// Pass 3 — liquid smoke.
//
// Each bead drags a short tail of soft puffs sampled from its own past
// positions. Because particle_pos is analytic, "where was this bead 200ms ago"
// is just a call with a different time — no simulation, no history buffer, and
// the smoke is bound to the particles by construction rather than by guesswork.
//
// The puffs are then advected through a swirling direction field, which is what
// turns a trail of blobs into something that curls like fluid.

/// Puffs trailing each bead.
const PUFFS: u32 = 6u;
/// Spacing between puffs, in seconds of the bead's past.
const PUFF_SPACING: f32 = 0.09;
/// How far the swirl can carry a puff by the end of its life.
const SWIRL: f32 = 0.22;
const SIZE_NEAR: f32 = 0.05;
const SIZE_FAR: f32 = 0.20;
const SMOKE_OPACITY: f32 = 0.13;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) tint: vec3<f32>,
    @location(2) fade: f32,
    @location(3) seed: f32,
};

const QUAD: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, 1.0),
);

// A smooth, slowly turning direction field. Proper curl noise needs nine noise
// samples for the finite differences; two are enough to sell the motion here.
fn swirl_dir(p: vec3<f32>, t: f32) -> vec3<f32> {
    let a = vnoise(p * 1.3 + vec3<f32>(0.0, t * 0.10, 0.0)) * PI * 2.0;
    let b = vnoise(p * 1.1 + vec3<f32>(17.3, 0.0, -t * 0.07)) * PI;
    return vec3<f32>(cos(a) * sin(b), cos(b), sin(a) * sin(b));
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let bead = ii / PUFFS;
    let puff = ii % PUFFS;

    // Age of this puff, 0 at the bead and 1 at the tail's end.
    let age_seconds = f32(puff) * PUFF_SPACING;
    let age = f32(puff) / f32(PUFFS - 1u);

    // Where the bead actually was, then carried along by the swirl.
    var center = particle_pos(bead, u.time - age_seconds);
    center = center + swirl_dir(center, u.time) * SWIRL * age * age;

    let size = mix(SIZE_NEAR, SIZE_FAR, age);
    let corner = QUAD[vi];
    let world = center + (u.camera_right.xyz * corner.x + u.camera_up.xyz * corner.y) * size;

    var out: VsOut;
    out.pos = u.view_proj * vec4<f32>(world, 1.0);
    out.local = corner;

    // Cool at rest, warming toward the vein colour as the music drives it, so
    // the smoke belongs to the same palette as the room.
    out.tint = mix(vec3<f32>(0.42, 0.52, 0.72), VEIN_CORE, u.audio.w * 0.35);

    // Thins out along the tail, and lifts a little with the hats so the smoke
    // breathes with the mix rather than sitting at one density.
    out.fade = (1.0 - age) * (1.0 - age) * (0.75 + band(13u) * 0.5);
    out.seed = f32(bead) * 0.379;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let r = length(in.local);
    if r > 1.0 {
        discard;
    }

    // Soft round core, then broken up by noise so the puff has structure
    // instead of reading as an airbrushed dot.
    let falloff = pow(1.0 - r, 2.2);
    let wisp = fbm(vec3<f32>(in.local * 1.9, in.seed + u.time * 0.25) * 1.6);

    let density = falloff * mix(0.45, 1.35, wisp) * in.fade * SMOKE_OPACITY;
    return vec4<f32>(in.tint, clamp(density, 0.0, 1.0));
}
