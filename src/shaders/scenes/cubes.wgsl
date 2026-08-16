// --- gravitational cube sea --------------------------------------------

const CUBE_SPACING: f32 = 1.42;

fn cube_hash(cell: vec2<f32>) -> f32 {
    return fract(sin(dot(cell, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn cube_motion(cell: vec2<f32>) -> vec4<f32> {
    // Concentric launches meet a diagonal travelling wave, so the field has a
    // legible large-scale pattern without rows moving as rigid strips.
    let distance_phase = length(cell) * 0.075;
    let diagonal = dot(cell, normalize(vec2<f32>(1.0, 0.63))) * 0.035;
    let cycle = fract(u.cubes.w * 0.125 - distance_phase - diagonal);
    let airborne = smoothstep(0.02, 0.12, cycle) * (1.0 - smoothstep(0.62, 0.90, cycle));
    let flight = clamp((cycle - 0.06) / 0.78, 0.0, 1.0);
    // A parabola is the visual signature of gravity: fast launch, suspended
    // apex, accelerating fall. Bass increases height, never phase.
    let height = 4.0 * flight * (1.0 - flight)
        * airborne * (1.15 + u.audio.x * 1.35) * u.cubes.z;
    let impact = (1.0 - smoothstep(0.86, 0.96, cycle)) * smoothstep(0.79, 0.90, cycle)
        * u.cubes.z;
    return vec4<f32>(height, flight, airborne, impact);
}

fn cube_at(p: vec3<f32>, cell: vec2<f32>) -> vec3<f32> {
    let motion = cube_motion(cell);
    let random = cube_hash(cell);
    let center = vec3<f32>(cell.x * CUBE_SPACING, 0.48 + motion.x, cell.y * CUBE_SPACING);
    var q = p - center;
    let turn = motion.y * (PI * (1.0 + random * 2.0)) * motion.z;
    q = rot_y(turn + random * 0.18) * rot_x(turn * (0.38 + random * 0.34)) * q;
    let size = 0.43 + sin(dot(cell, vec2<f32>(0.71, 1.13))) * 0.035;
    let distance = sd_round_box(q, vec3<f32>(size), 0.055);
    return vec3<f32>(distance, motion.w, random);
}

fn cube_field(p: vec3<f32>) -> vec3<f32> {
    let base = floor(p.xz / CUBE_SPACING + 0.5);
    var nearest = vec3<f32>(1.0e5, 0.0, 0.0);
    for (var z = -1; z <= 1; z = z + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let sample = cube_at(p, base + vec2<f32>(f32(x), f32(z)));
            if sample.x < nearest.x { nearest = sample; }
        }
    }
    // A nearly black metallic floor catches the cubes and anchors their fall.
    let floor_distance = p.y;
    if floor_distance < nearest.x {
        return vec3<f32>(floor_distance, 0.0, -1.0);
    }
    return nearest;
}

fn cube_normal(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(0.003, -0.003);
    return normalize(
        e.xyy * cube_field(p + e.xyy).x
            + e.yyx * cube_field(p + e.yyx).x
            + e.yxy * cube_field(p + e.yxy).x
            + e.xxx * cube_field(p + e.xxx).x,
    );
}

fn render_cube_scene(ro: vec3<f32>, rd: vec3<f32>) -> RayScene {
    var out: RayScene;
    let horizon = pow(max(1.0 - abs(rd.y), 0.0), 7.0);
    out.color = (vec3<f32>(0.004, 0.006, 0.011) + LENS_PEACH * horizon * 0.018)
        * (1.0 - u.outro.x);
    out.depth = 1.0;

    var t = 0.04;
    var hit = vec3<f32>(-1.0);
    for (var step = 0; step < 112; step = step + 1) {
        let sample = cube_field(ro + rd * t);
        if sample.x < 0.0011 * t {
            hit = vec3<f32>(t, sample.yz);
            break;
        }
        t = t + max(sample.x * 0.60, 0.004);
        if t > 42.0 { break; }
    }
    if hit.x < 0.0 { return out; }

    let p = ro + rd * hit.x;
    let n = cube_normal(p);
    let view = max(dot(-rd, n), 0.0);
    let fresnel = pow(1.0 - view, 4.0);
    let key_dir = normalize(vec3<f32>(-0.45, 0.78, 0.35));
    let key = max(dot(n, key_dir), 0.0);
    let spec = pow(max(dot(reflect(rd, n), key_dir), 0.0), 72.0);
    let is_floor = step(hit.z, -0.5);
    let metal = mix(
        vec3<f32>(0.09, 0.105, 0.135) * (0.30 + key * 0.72),
        vec3<f32>(0.012, 0.016, 0.024),
        is_floor,
    );
    let edge_light = vec3<f32>(0.38, 0.43, 0.52) * (fresnel * 0.46 + spec * 0.75);
    let impact_light = LENS_PEACH * hit.y * (2.2 + u.audio.w * 1.8);
    // The landing flash also spreads over the floor beneath each impact.
    let cell = floor(p.xz / CUBE_SPACING + 0.5);
    let floor_pulse = cube_motion(cell).w
        * exp(-length(p.xz - cell * CUBE_SPACING) * 2.8)
        * is_floor;
    let scene_surface = metal + edge_light + impact_light;
    // At the end, every part of the cube scene fades except its own travelling
    // floor-impact wave. Keep that original signal dimly alive under the cards;
    // no separate screen-space beam is introduced.
    let wave = LENS_PEACH * floor_pulse
        * mix(1.7, 0.46 * u.outro.y, u.outro.x);
    out.color = scene_surface * (1.0 - u.outro.x) + wave;
    out.color = mix(
        vec3<f32>(0.004, 0.006, 0.011) * (1.0 - u.outro.x),
        out.color,
        exp(-hit.x * 0.032),
    );
    out.depth = clip_depth(p);
    return out;
}
