// Pass 2 — orbiting particles as camera-facing quads. Positions are analytic
// (see particle_pos), so there are no vertex or instance buffers: the whole
// swarm is draw(0..6, 0..N). Depth-tested against the blob the scene pass wrote.

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) tint: vec3<f32>,
    @location(2) visible: f32,
    @location(3) scene: f32,
    @location(4) arrival: f32,
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

    // Branched rather than mixed with a zero weight. Outside the fractal scene
    // there is no corridor in the uniform to sample — it is left at zero — and
    // mixing a bead position from it at weight zero is not a no-op if that
    // position is not a number.
    var center = particle_pos(ii, u.time);
    var visible = 1.0;
    if fractal_scene > 0.0 {
        let bead = fractal_flow_bead(ii);
        center = mix(center, bead.xyz, fractal_scene);
        visible = mix(1.0, bead.w, fractal_scene);
    }
    if u.debug.y > 0.5 {
        let forward = camera_ray(vec2<f32>(0.0, 0.0));
        let right = u.camera_right.xyz;
        center = u.camera_pos.xyz + forward * 4.0 + right * (f32(ii) * 0.05 - 5.0);
        visible = 1.0;
    }

    let fi = f32(ii);
    // A reveal sweeps from each string's leading bead to its tail. The global
    // arrival is deliberately delayed until after the transition; scaling it
    // past one gives the last bead enough time to finish its soft pop-in.
    let per_string = max(u.particle_count / f32(STRINGS), 1.0);
    let order = (f32(ii / STRINGS) + 0.5) / per_string;
    let arrival = smoothstep(order, order + 0.18, u.frame.w * 1.2);
    // Much bigger against the fractal. The structure fills the frame and the
    // camera is metres from the strings, so beads at the blob's size came out
    // as a few dark pixels — the string was there and could not be seen. At
    // this size they are beads on a cord, close to as wide as the tunnels they
    // thread.
    // The beat barely moves them in the fractal scene; nothing there is meant
    // to punch.
    let pulse = 1.0 + u.audio.w * mix(0.45, 0.06, fractal_scene);
    let size = (0.018 + 0.022 * fract(fi * 0.31)) * pulse
        * mix(1.0, 4.0 * (0.55 + arrival * 0.45), fractal_scene);
    let world = center + (u.camera_right.xyz * corner.x + u.camera_up.xyz * corner.y) * size;

    var out: VsOut;
    out.pos = u.view_proj * vec4<f32>(world, 1.0);
    out.visible = visible;
    out.scene = fractal_scene;
    out.arrival = arrival;
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
    //
    // The new scene gets a clean beat before a strand-by-strand reveal. In
    // the first scene `in.scene` is zero, so this remains fully present.
    let arrived = mix(1.0, in.arrival, in.scene);
    let presence = max(1.0 - u.scene.x, in.scene) * in.visible * arrived;
    return vec4<f32>(in.tint, smoothstep(1.0, 0.55, r) * 0.92 * u.intro.z * presence);
}
