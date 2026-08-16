// A single quantized object for the repeated synth phrase between the fractal
// and lens fields: black, metal, one cube, one warm response per note.

fn signal_lead_tick() -> f32 {
    // Measured from the isolated synth stem. Full source onset 219.776s minus
    // the soundtrack cut's 176s start, with a 0.514s early-note cadence.
    return max((u.time - 43.776) / 0.514, 0.0);
}

fn signal_rot_z(a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat3x3<f32>(
        vec3<f32>(c, -s, 0.0),
        vec3<f32>(s, c, 0.0),
        vec3<f32>(0.0, 0.0, 1.0),
    );
}

fn signal_rotation_at(p: vec3<f32>, tick: f32) -> vec3<f32> {
    let whole = floor(tick);
    let snap = smoothstep(0.0, 0.20, fract(tick));
    let turn = whole + snap;
    let cycle = floor(turn / 3.0);
    let within = turn - cycle * 3.0;
    let x = (cycle + clamp(within, 0.0, 1.0)) * PI * 0.5;
    let y = (cycle + clamp(within - 1.0, 0.0, 1.0)) * PI * 0.5;
    let z = (cycle + clamp(within - 2.0, 0.0, 1.0)) * PI * 0.5;
    return signal_rot_z(-z) * (rot_y(-y) * (rot_x(-x) * p));
}

fn signal_rotation(p: vec3<f32>) -> vec3<f32> {
    return signal_rotation_at(p, signal_lead_tick());
}

fn signal_cube_sdf(p: vec3<f32>) -> f32 {
    let response = u.audio.w;
    return sd_round_box(
        signal_rotation(p),
        vec3<f32>(0.78 + response * 0.018),
        0.105 + response * 0.032,
    );
}

fn signal_shape(p: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(signal_cube_sdf(p), 0.0);
}

fn signal_cube_normal(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(0.0015, -0.0015);
    return normalize(
        e.xyy * signal_shape(p + e.xyy).x
        + e.yyx * signal_shape(p + e.yyx).x
        + e.yxy * signal_shape(p + e.yxy).x
        + e.xxx * signal_shape(p + e.xxx).x
    );
}

fn signal_background(ro: vec3<f32>, rd: vec3<f32>) -> vec3<f32> {
    let tick = signal_lead_tick();
    let pulse = exp(-fract(tick) * 6.5);
    let radius = ROOM_RADIUS;
    let b = dot(ro, rd);
    let h2 = b * b - (dot(ro, ro) - radius * radius);
    if h2 < 0.0 {
        return vec3<f32>(0.006, 0.008, 0.016);
    }

    let p = ro + rd * (-b + sqrt(h2));
    let dir = p / radius;
    // The blob rooms living field, spun around the hero and recolored.
    let snap = smoothstep(0.0, 0.20, fract(tick));
    // The room turns continuously, then counter-kicks when the cube locks to
    // its new axis. Their opposed motion is the interaction in this shot.
    let counter_kick = (floor(tick) + snap) * 0.105;
    let spun = rot_y(u.time * 0.16 - counter_kick)
        * (rot_x(sin(u.time * 0.11) * 0.42 + counter_kick * 0.32) * dir);
    let height = room_height(spun);
    let ridge = 1.0 - abs(height * 2.0 - 1.0);
    let vein = pow(smoothstep(0.84, 1.0, ridge), 2.25);
    let light = max(dot(-dir, normalize(SUN_DIR)), 0.0);
    let cool = mix(vec3<f32>(0.018, 0.028, 0.060), vec3<f32>(0.18, 0.25, 0.48), smoothstep(0.22, 0.82, height));
    let warm = mix(vec3<f32>(0.68, 0.12, 0.035), LENS_PEACH, pulse * 0.42);
    return cool * (0.20 + light * 0.82) + warm * vein * (0.20 + pulse * 0.42);
}

fn render_signal_scene(ro: vec3<f32>, rd: vec3<f32>) -> RayScene {
    var out: RayScene;
    out.color = signal_background(ro, rd);
    out.depth = 1.0;

    var t = 0.0;
    var hit = vec2<f32>(-1.0);
    for (var i = 0; i < 80; i = i + 1) {
        let sample = signal_shape(ro + rd * t);
        if sample.x < 0.001 {
            hit = vec2<f32>(t, sample.y);
            break;
        }
        t = t + sample.x * 0.75;
        if t > 20.0 {
            break;
        }
    }
    if hit.x < 0.0 {
        let axis = abs(dot(rd, normalize(-ro)));
        out.color = out.color + LENS_PEACH * pow(axis, 90.0) * u.audio.w * 0.055;
        return out;
    }

    let p = ro + rd * hit.x;
    let n = signal_cube_normal(p);
    let light = normalize(vec3<f32>(-0.6, 0.9, 0.45));
    let diffuse = max(dot(n, light), 0.0);
    let view = max(dot(-rd, n), 0.0);
    let fresnel = pow(1.0 - view, 4.0);
    let spec = pow(max(dot(reflect(rd, n), light), 0.0), 72.0);
    let reflected = signal_background(p + n * 0.02, reflect(rd, n));
    let metal = vec3<f32>(0.09, 0.105, 0.135) * (0.30 + diffuse * 0.72);
    let edge_light = vec3<f32>(0.38, 0.43, 0.52) * (fresnel * 0.46 + spec * 0.75);
    let weld = LENS_PEACH * u.audio.w * (fresnel * 0.34 + spec * 0.46);
    out.color = metal + reflected * (0.10 + fresnel * 0.20) + edge_light + weld;
    out.depth = clip_depth(p);
    return out;
}
