// Pass 2 — orbiting particles as camera-facing quads. Positions are analytic
// (see particle_pos), so there are no vertex or instance buffers: the whole
// swarm is draw(0..6, 0..N). Depth-tested against the blob the scene pass wrote.

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) tint: vec3<f32>,
    @location(2) visible: f32,
    @location(3) scene: f32,
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

    // The swarm returns for the fractal, walking its surface instead of
    // orbiting the space in front of it.
    // Gated on the same threshold the fractal itself appears at. Fading them
    // in across the whole collapse showed the stream over the octopus, in a
    // scene that has already sent its particles away.
    let fractal_scene = smoothstep(0.88, 1.0, u.collapse.x);

    var bead = fractal_flow_bead(ii);
    if u.debug.y > 0.5 {
        let forward = camera_ray(vec2<f32>(0.0, 0.0));
        let right = u.camera_right.xyz;
        bead = vec4<f32>(
            u.camera_pos.xyz + forward * 4.0 + right * (f32(ii) * 0.05 - 5.0),
            1.0,
        );
    }
    let center = mix(particle_pos(ii, u.time), bead.xyz, fractal_scene);

    let fi = f32(ii);
    // Bigger against the fractal, which is a much bigger object than the blob,
    // but only enough that the beads touch at the string's spacing — larger and
    // the string fuses into a solid tube with no beads left in it.
    // The beat barely moves them in the fractal scene; nothing there is meant
    // to punch.
    let pulse = 1.0 + u.audio.w * mix(0.45, 0.06, fractal_scene);
    let size = (0.018 + 0.022 * fract(fi * 0.31)) * pulse * mix(1.0, 1.5, fractal_scene);
    let world = center + (u.camera_right.xyz * corner.x + u.camera_up.xyz * corner.y) * size;

    var out: VsOut;
    out.pos = u.view_proj * vec4<f32>(world, 1.0);
    out.visible = mix(1.0, bead.w, fractal_scene);
    out.scene = fractal_scene;
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

    // Gone through the ferrofluid, back for the fractal.
    let presence = max(1.0 - u.scene.x, in.scene) * in.visible;
    return vec4<f32>(in.tint, smoothstep(1.0, 0.55, r) * 0.92 * u.intro.z * presence);
}
