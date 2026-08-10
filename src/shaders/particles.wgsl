// Pass 2 — orbiting particles as camera-facing quads, blended additively into
// the HDR target. Positions are analytic (see particle_pos), so there are no
// vertex or instance buffers: draw(0..6, 0..N).

@group(1) @binding(0) var scene_depth: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) tint: vec3<f32>,
    @location(2) view_dist: f32,
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
    let size = 0.018 + 0.022 * fract(fi * 0.31);
    let world = center + (u.camera_right.xyz * corner.x + u.camera_up.xyz * corner.y) * size;

    var out: VsOut;
    out.pos = u.view_proj * vec4<f32>(world, 1.0);
    out.local = corner;
    // Shades of dark only — no hue, just how black each bead sits.
    out.tint = vec3<f32>(0.012, 0.014, 0.022) * (0.4 + 1.6 * fract(fi * 0.517));
    out.view_dist = length(center - u.camera_pos.xyz);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Soft round falloff instead of a hard-edged quad.
    let r = length(in.local);
    if r > 1.0 {
        discard;
    }
    // Solid core with a soft edge, so the beads read as ink dots, not blobs.
    let mask = smoothstep(1.0, 0.55, r);

    // Manual occlusion against the blob: these quads have no depth test, so we
    // compare against the scene's distance buffer instead.
    let coord = vec2<i32>(floor(in.pos.xy));
    let scene_dist = textureLoad(scene_depth, coord, 0).r;
    let visible = smoothstep(-0.04, 0.02, scene_dist - in.view_dist);

    let alpha = mask * visible * 0.92;
    return vec4<f32>(in.tint, alpha);
}
