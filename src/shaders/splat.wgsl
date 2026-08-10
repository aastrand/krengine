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

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) bead: u32) -> VsOut {
    let world = particle_pos(bead, u.time);
    let previous = particle_pos(bead, u.time - VELOCITY_DT);

    let here = project(world);
    let before = project(previous);

    // Each bead belongs to the sheet whose depth band it falls in, softly, so
    // one drifting between bands hands over instead of popping.
    let relative = length(world - u.camera_pos.xyz) - length(u.camera_pos.xyz);
    let d = (relative - layer.offset) / layer.width;

    let corner = QUAD[vi];
    let uv = here + corner * vec2<f32>(SPLAT_RADIUS / GRID_ASPECT, SPLAT_RADIUS);

    var out: VsOut;
    out.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.local = corner;
    out.velocity = (here - before) / VELOCITY_DT;
    out.weight = exp(-d * d);
    return out;
}

@fragment
fn fs_velocity(in: VsOut) -> @location(0) vec4<f32> {
    let squared = dot(in.local, in.local);
    if squared > 1.0 {
        discard;
    }
    let falloff = exp(-squared);
    let grip = clamp(falloff * in.weight * COUPLING * u.frame.x, 0.0, 1.0);
    return vec4<f32>(in.velocity, 0.0, grip);
}

@fragment
fn fs_dye(in: VsOut) -> @location(0) vec4<f32> {
    let squared = dot(in.local, in.local);
    if squared > 1.0 {
        discard;
    }
    let amount = exp(-squared) * in.weight * EMISSION * u.frame.x;
    return vec4<f32>(amount, 0.0, 0.0, 0.0);
}
