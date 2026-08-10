// Pass 2 — orbiting particles as camera-facing quads. Positions are analytic
// (see particle_pos), so there are no vertex or instance buffers: the whole
// swarm is draw(0..6, 0..N). Depth-tested against the blob the scene pass wrote.

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) tint: vec3<f32>,
};

const QUAD: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, 1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let corner = QUAD[vi];
    let center = particle_pos(ii, u.time);

    let fi = f32(ii);
    let size = (0.018 + 0.022 * fract(fi * 0.31)) * (1.0 + u.audio.w * 0.45);
    let world = center + (u.camera_right.xyz * corner.x + u.camera_up.xyz * corner.y) * size;

    var out: VsOut;
    out.pos = u.view_proj * vec4<f32>(world, 1.0);
    out.local = corner;
    // Shades of dark only — no hue, just how black each bead sits.
    out.tint = vec3<f32>(0.012, 0.014, 0.022) * (0.4 + 1.6 * fract(fi * 0.517));
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Solid core with a soft edge, so the beads read as ink dots, not blobs.
    let r = length(in.local);
    if r > 1.0 {
        discard;
    }

    return vec4<f32>(in.tint, smoothstep(1.0, 0.55, r) * 0.92 * u.intro.z);
}
