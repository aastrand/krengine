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

const LENS_SATELLITE_COUNT: u32 = 420u;

fn lens_satellite_pos(i: u32) -> vec3<f32> {
    let lens = i % LENS_COUNT;
    let slot = i / LENS_COUNT;
    let count = u32(ceil(u.particle_count / f32(LENS_COUNT)));
    let fi = f32(i);
    let li = f32(lens);
    let scrambled = (slot * 37u + lens * 11u) % max(count, 1u);
    let base = sphere_direction(scrambled, max(count, 1u));
    let direction = rot_y(u.time * (0.025 + li * 0.003) + li * 1.7)
        * (rot_x(li * 0.43) * base);
    let tangent = safe_direction(cross(direction, vec3<f32>(0.1, 1.0, 0.07)), vec3<f32>(1.0, 0.0, 0.0));
    let voice = band(4u + (lens * 2u) % 12u);
    // A broad, irregular cloud rather than a constant-radius orbit. Some
    // droplets disappear behind the transparent body; others drift between
    // neighbouring lenses, breaking any readable planetary ring.
    let shell = lens_radius(lens) * (0.72 + fract(fi * 0.371) * 1.08);
    let drift = tangent * sin(u.time * 0.19 + fi * 1.13) * (0.045 + voice * 0.075);
    return lens_center(lens) + direction * shell + drift;
}

fn cube_impact_particle(i: u32) -> vec4<f32> {
    let mote = i % 4u;
    let slot = i / 4u;
    let x = i32(slot % 10u) - 5;
    let z = i32(slot / 10u) + 1;
    let spacing = 1.42;
    let anchor = vec2<f32>(
        floor(u.camera_pos.x / spacing) + f32(x),
        floor(u.camera_pos.z / spacing) - f32(z),
    );
    let phase = length(anchor) * 0.075
        + dot(anchor, normalize(vec2<f32>(1.0, 0.63))) * 0.035;
    let cycle = fract(u.cubes.w * 0.125 - phase);
    let life = clamp((cycle - 0.74) / 0.25, 0.0, 1.0);
    let alive = smoothstep(0.72, 0.78, cycle) * (1.0 - smoothstep(0.94, 0.995, cycle));
    let seed = fract(f32(i) * 0.6180339);
    let angle = seed * PI * 2.0 + f32(mote) * 1.7;
    // Launch from just outside the top edges. Emitting at the cell centre put
    // every mote inside its cube, where the scene depth correctly hid it.
    let radial = 0.52 + life * (0.24 + seed * 0.24);
    let spread = vec2<f32>(cos(angle), sin(angle)) * radial;
    let lift = 4.0 * life * (1.0 - life) * (0.52 + seed * 0.58);
    let position = vec3<f32>(
        anchor.x * spacing + spread.x,
        0.58 + lift,
        anchor.y * spacing + spread.y,
    );
    return vec4<f32>(position, alive * u.cubes.y);
}

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

        // The opaque membrane owns the handoff. Fade the strings before it
        // covers the frame, swap their positions while hidden, then introduce
        // only the sparse satellite set after the new room is revealed. The
        // former camera-space ring made the transition look like a HUD effect.
        if u.lens.y < 0.5 {
            visible = visible * (1.0 - smoothstep(0.06, 0.42, u.lens.y));
        } else {
            center = lens_satellite_pos(ii);
            center = clear_of_lenses(center);
            visible = step(f32(ii), f32(LENS_SATELLITE_COUNT - 1u)) * u.lens.w;
        }
        if u.tunnel.x > 0.0 {
            visible = visible * (1.0 - u.tunnel.y);
        }
    }
    if u.cubes.x > 0.0 {
        let impact = cube_impact_particle(ii);
        center = impact.xyz;
        visible = impact.w;
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
    let fractal_size = mix(4.0 * (0.55 + arrival * 0.45), 1.65, u.lens.w);
    let cube_size = mix(1.0, 0.92, u.cubes.y);
    let size = (0.018 + 0.022 * fract(fi * 0.31)) * pulse
        * mix(1.0, fractal_size, fractal_scene) * cube_size;
    let world = center + (u.camera_right.xyz * corner.x + u.camera_up.xyz * corner.y) * size;

    var out: VsOut;
    out.pos = u.view_proj * vec4<f32>(world, 1.0);
    out.visible = visible;
    out.scene = fractal_scene;
    out.arrival = arrival;
    out.local = corner;
    // Most satellites stay ink-dark; a minority become pearl droplets that
    // catch the same warm rim as the membranes.
    let ink = vec3<f32>(0.012, 0.014, 0.022) * (0.4 + 1.6 * fract(fi * 0.517));
    let pearl = vec3<f32>(1.00, 0.42, 0.13) * (0.45 + fract(fi * 0.193) * 0.40);
    let satellite = mix(ink, pearl, step(0.76, fract(fi * 0.731)));
    let previous_tint = mix(ink, satellite, u.lens.w);
    let impact_tint = mix(VEIN_COLOR, VEIN_CORE, 0.58)
        * (1.15 + fract(fi * 0.417) * 0.75);
    out.tint = mix(previous_tint, impact_tint, u.cubes.y);
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

/// Invisible central depth for the non-occluding particle pass. The broad,
/// soft bead remains alpha blended, but its solid centre tells post-processing
/// where the string actually is instead of exposing the wall or far plane
/// behind it to depth of field.
@fragment
fn fs_focus_depth(in: VsOut) -> @location(0) vec4<f32> {
    let r = length(in.local);
    let arrived = mix(1.0, in.arrival, in.scene);
    let presence = max(1.0 - u.scene.x, in.scene) * in.visible * arrived;
    if r > 0.52 || presence < 0.08 {
        discard;
    }
    return vec4<f32>(0.0);
}
