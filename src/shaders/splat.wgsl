// Injects every bead into the fluid by rasterising a small quad per bead.
//
// The per-cell alternative loops over emitters inside each of 330k cells, so
// its cost is cells x beads and only a sample of the swarm is affordable.
// Scattering costs beads x splat-area instead, which is nothing, so all of
// them can participate.
//
// Blending does the work: for velocity, straight alpha gives
// dst*(1-a) + src*a, which is exactly mix(fluid, bead_velocity, grip) — the
// obstacle constraint, not an injected jet. For dye it is simply additive.

struct LayerParams {
    offset: f32,
    width: f32,
    pad0: f32,
    pad1: f32,
};
@group(1) @binding(0) var<uniform> layer: LayerParams;

/// Bead radius in grid space. Matches EMITTER_RADIUS in fluid.wgsl.
const SPLAT_RADIUS: f32 = 0.020;
/// Grid aspect, so a bead stays round on a non-square grid.
const GRID_ASPECT: f32 = 768.0 / 432.0;
/// How strongly a bead's motion is imposed on the fluid touching it.
const COUPLING: f32 = 22.0;
/// How much dye a bead sheds. Far lower than the sampled version used: every
/// bead emits now, not one in eight.
const EMISSION: f32 = 0.32;
/// Interval used to difference the bead's path into a velocity.
const VELOCITY_DT: f32 = 0.03;
/// Only spikes reaching past this shed smoke. Without a gate every direction
/// emits, including the majority whose tip is still on the body, which reads
/// as the whole blob steaming rather than as wisps off the arms.
///
/// The lobe field is raised to the fourth power, so its values are small: it
/// peaks between 0.15 and 0.65 depending on the moment, and only a tenth of
/// directions clear 0.1. This sits just under that, or nothing emits at all.
const EMIT_LIMIT: f32 = 0.045;
const EMIT_RAMP: f32 = 0.11;
/// With only the longest arms emitting, each has to shed more to add up.
const EMIT_BOOST: f32 = 5.0;
/// How much harder the arms shed while the body is bleeding out into the water.
const BLEED_EMISSION: f32 = 2.5;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) velocity: vec2<f32>,
    @location(2) weight: f32,
};

const QUAD: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, 1.0),
);

/// World point to grid UV, or far offscreen when it is behind the camera.
fn project(p: vec3<f32>) -> vec2<f32> {
    let clip = u.view_proj * vec4<f32>(p, 1.0);
    if clip.w <= 0.0 {
        return vec2<f32>(-10.0);
    }
    let ndc = clip.xy / clip.w;
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

/// What stirs the fluid: the beads to begin with, the spike tips once the
/// ferrofluid has taken over. Crossfaded, so the handover is not a jump.
fn stirrer(i: u32, t: f32) -> vec3<f32> {
    let takeover = smoothstep(0.15, 0.85, u.scene.x);
    return mix(particle_pos(i, t), spike_tip(i, t), takeover);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) bead: u32) -> VsOut {
    let world = stirrer(bead, u.time);
    let previous = stirrer(bead, u.time - VELOCITY_DT);

    let here = project(world);
    let before = project(previous);

    // Each bead belongs to the sheet whose depth band it falls in, softly, so
    // one drifting between bands hands over instead of popping.
    let relative = length(world - u.camera_pos.xyz) - length(u.camera_pos.xyz);
    let d = (relative - layer.offset) / layer.width;

    // In the second scene, only the arms that are actually out contribute.
    let extended = smoothstep(EMIT_LIMIT, EMIT_LIMIT + EMIT_RAMP, spike_strength(bead));
    let gate = mix(
        mix(1.0, extended, smoothstep(0.15, 0.85, u.scene.x)),
        1.0,
        u.collapse.y,
    );

    let corner = QUAD[vi];
    // Arms shed a broader plume than a bead's thin wake.
    let radius = SPLAT_RADIUS * mix(1.0, 1.7, smoothstep(0.15, 0.85, u.scene.x));
    let uv = here + corner * vec2<f32>(radius / GRID_ASPECT, radius);

    var out: VsOut;
    out.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.local = corner;
    out.velocity = (here - before) / VELOCITY_DT;
    out.weight = exp(-d * d) * gate;
    return out;
}

@fragment
fn fs_velocity(in: VsOut) -> @location(0) vec4<f32> {
    let squared = dot(in.local, in.local);
    if squared > 1.0 {
        discard;
    }
    let falloff = exp(-squared);
    let grip = clamp(
        falloff * in.weight * COUPLING * (1.0 + u.scene.z * 1.5) * u.frame.x,
        0.0,
        1.0,
    );
    return vec4<f32>(in.velocity, 0.0, grip);
}

@fragment
fn fs_dye(in: VsOut) -> @location(0) vec4<f32> {
    let squared = dot(in.local, in.local);
    if squared > 1.0 {
        discard;
    }
    // u.scene.z floods the field during a transition, so the dissolve has a
    // dense front to wipe with rather than the beads' thin trails.
    let flood = 1.0 + u.scene.z * 14.0;
    // Bleeding out: the body puts everything it has into the water.
    let boost = mix(1.0, EMIT_BOOST, smoothstep(0.15, 0.85, u.scene.x))
        * (1.0 + u.collapse.y * BLEED_EMISSION);
    let amount =
        exp(-squared) * in.weight * EMISSION * flood * boost * u.scene.w * u.frame.x;
    return vec4<f32>(amount, 0.0, 0.0, 0.0);
}
