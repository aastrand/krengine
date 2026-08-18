// --- marching liquid-metal tunnel -------------------------------------

fn tunnel_centerline(z: f32) -> vec2<f32> {
    return vec2<f32>(sin(z * 0.24) * 0.38, cos(z * 0.19) * 0.30);
}

fn tunnel_twist(z: f32) -> f32 {
    return z * 0.18 + sin(z * 0.12) * 0.38;
}

fn tunnel_wall_radius(z: f32, angle: f32) -> f32 {
    // A rotating, asymmetric three-lobed section. A circular tube cannot show
    // twist however much its texture rotates; this changes the silhouette and
    // therefore produces actual corkscrewing parallax as the camera advances.
    // Two low-frequency pressure waves make the metal wall breathe and trade
    // mass between its sides. They travel more slowly than the camera, so this
    // reads as living material rather than procedural vibration.
    let slow_time = u.music.z * PI * 0.20;
    let breathing = sin(z * 0.31 - slow_time) * 0.10;
    let traveling_bulge = sin(angle * 2.0 - z * 0.22 + slow_time * 0.73) * 0.13;
    let beat_squeeze = pow(max(sin(z * 0.48 - u.music.z * PI), 0.0), 3.0)
        * u.audio.x * 0.11;
    return 2.05
        + sin(angle * 3.0 + tunnel_twist(z)) * 0.27
        + sin(angle * 5.0 - tunnel_twist(z) * 0.7) * 0.08
        + sin(z * 0.53) * 0.08
        + breathing
        + traveling_bulge
        - beat_squeeze;
}

fn tentacle_curve(a: vec3<f32>, b: vec3<f32>, side: vec3<f32>, t: f32, gate: f32) -> vec3<f32> {
    let arch = sin(t * PI);
    let sideways = arch * 0.68 + sin(t * PI * 2.0 + gate) * arch * 0.18;
    let downstream = -arch * (0.78 + u.audio.x * 0.16);
    return mix(a, b, t) + side * sideways + vec3<f32>(0.0, 0.0, downstream);
}

fn tentacle_radius(t: f32, grow: f32, gate: f32) -> f32 {
    // Continuous pressure along the strand, with a genuinely rounded growing
    // tip. This modulates one tube surface; it does not create separate beads.
    let pressure = sin(t * 8.0 - u.music.z * PI * 2.0 + gate)
        * (0.012 + u.audio.x * 0.020);
    let body = mix(0.23, 0.10, smoothstep(0.0, 1.0, t));
    let tip = smoothstep(0.0, 0.16, grow - t);
    return max((body + pressure) * mix(0.30, 1.0, tip), 0.032);
}

fn tunnel_tentacles(p: vec3<f32>) -> vec2<f32> {
    var nearest = 1.0e9;
    var root_heat = 0.0;
    let spacing = 5.2;
    let first_gate = floor((-u.camera_pos.z) / spacing) + 1.0;
    // Gate influence windows do not overlap, so a point can only belong to
    // one of the five visible tentacles. Resolve that gate directly instead
    // of rebuilding all five curves at every ray-march and normal sample.
    let nearest_gate = floor((-p.z - 0.9) / spacing + 0.5);
    let gate = clamp(nearest_gate, first_gate, first_gate + 4.0);
    let fi = gate - first_gate;
    if abs(nearest_gate - gate) < 0.5 {
        let base_z = -gate * spacing;
        let angle = gate * 2.399963;
        let opposite = angle + PI + sin(gate * 1.7) * 0.24;
        let a_radius = tunnel_wall_radius(base_z, angle) - 0.08;
        let b_z = base_z - 0.95;
        let b_radius = tunnel_wall_radius(b_z, opposite) - 0.08;
        let a = vec3<f32>(tunnel_centerline(base_z), base_z)
            + vec3<f32>(cos(angle) * a_radius, sin(angle) * a_radius, 0.0);
        let b = vec3<f32>(tunnel_centerline(b_z), b_z)
            + vec3<f32>(cos(opposite) * b_radius, sin(opposite) * b_radius, 0.0);
        let chord = b - a;
        let side = safe_direction(vec3<f32>(-chord.y, chord.x, 0.0), vec3<f32>(1.0, 0.0, 0.0));

        // Each gate gets its own two-bar cycle, offset by a half-beat. It grows
        // decisively on a beat, holds across the bore, then melts back before
        // appearing at the next gate farther down the march.
        let cycle = fract((u.tunnel.w + fi * 1.65) / 8.0);
        let attack = smoothstep(0.02, 0.24, cycle);
        let release = 1.0 - smoothstep(0.76, 0.98, cycle);
        let grow = attack * release * u.tunnel.z;

        // Gates are separated by more than five units. Bounding by depth means
        // several can exist in the shot while each march sample evaluates the
        // expensive liquid curve for at most one nearby gate.
        if grow > 0.001 && abs(p.z - (base_z - 0.9)) < 2.35 {
            // Join adjacent curve samples into a continuously tapered sweep.
            // Previously each sample contributed a sphere; close shots exposed
            // those individual lobes as a string of pearls.
            var strand = 1.0e9;
            // Sixteen swept capsules remain visually continuous at the
            // strand's radius, while avoiding five extra curve evaluations
            // per field query compared with the original tessellation.
            for (var sample = 0u; sample < 16u; sample = sample + 1u) {
                let t0 = f32(sample) / 16.0;
                if t0 < grow {
                    let t1 = min(f32(sample + 1u) / 16.0, grow);
                    let p0 = tentacle_curve(a, b, side, t0, gate);
                    let p1 = tentacle_curve(a, b, side, t1, gate);
                    let axis = p1 - p0;
                    let h = clamp(dot(p - p0, axis) / max(dot(axis, axis), 0.00001), 0.0, 1.0);
                    let radius = mix(
                        tentacle_radius(t0, grow, gate),
                        tentacle_radius(t1, grow, gate),
                        h,
                    );
                    let sweep = length(p - mix(p0, p1, h)) - radius;
                    strand = min(strand, sweep);
                }
            }
            nearest = min(nearest, strand);

            // A short-lived hot meniscus travels with the growing end. It is
            // brighter than the body and therefore blooms exactly when the
            // focus rack arrives, without turning every strand orange.
            let live_tip = tentacle_curve(a, b, side, grow, gate);
            let tip_flash = smoothstep(0.03, 0.14, grow)
                * (1.0 - smoothstep(0.34, 0.58, grow));
            root_heat = max(root_heat, exp(-length(p - live_tip) * 5.6) * tip_flash);
        }
        let receiving = smoothstep(0.82, 1.0, grow);
        root_heat = max(
            root_heat,
            max(exp(-length(p - a) * 3.8), exp(-length(p - b) * 3.8) * receiving) * grow,
        );
    }
    return vec2<f32>(nearest, root_heat);
}

fn tunnel_field(p: vec3<f32>) -> vec3<f32> {
    let q = p.xy - tunnel_centerline(p.z);
    let radial = length(q);
    let angle = atan2(q.y, q.x);
    let wall = tunnel_wall_radius(p.z, angle) - radial;
    // Fine liquid ripples ride the large twisted silhouette. No luminous
    // hoops: depth comes from changing form and parallax, not target rings.
    let wall_pressure = pow(max(sin(p.z * 0.82 - u.music.z * PI + angle), 0.0), 3.0)
        * u.audio.x;
    var liquid_wall = wall
        + sin(p.z * 1.37 + angle * 2.0 - u.time * 0.15) * 0.035
        + sin(p.z * 2.91 - angle) * 0.014
        + wall_pressure * 0.09;
    let tentacle = tunnel_tentacles(p);
    // Pull the wall inward around both attachment points before joining the
    // fields. The wide smooth union then creates a liquid neck rather than a
    // tentacle merely intersecting a separate tunnel surface.
    liquid_wall = liquid_wall - tentacle.y * (0.18 + u.audio.x * 0.10);
    let join = 0.30 + u.audio.x * 0.10;
    let distance = smin_cubic(liquid_wall, tentacle.x, join);
    let material = smoothstep(-join, join, liquid_wall - tentacle.x);
    return vec3<f32>(distance, material, tentacle.y);
}

fn tunnel_normal(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(0.003, -0.003);
    return normalize(
        e.xyy * tunnel_field(p + e.xyy).x
            + e.yyx * tunnel_field(p + e.yyx).x
            + e.yxy * tunnel_field(p + e.yxy).x
            + e.xxx * tunnel_field(p + e.xxx).x,
    );
}

fn render_tunnel_scene(ro: vec3<f32>, rd: vec3<f32>) -> RayScene {
    var out: RayScene;
    let vanishing = pow(max(dot(rd, vec3<f32>(0.0, 0.0, -1.0)), 0.0), 18.0);
    // Rays through the open bore disappear into warm-black distance. A bright
    // miss color looked like a flat circular end-cap rather than continuation.
    out.color = vec3<f32>(0.005, 0.007, 0.012)
        + LENS_PEACH * vanishing * 0.012;
    out.depth = 1.0;
    var t = 0.04;
    var hit = vec3<f32>(-1.0);
    for (var step = 0; step < 104; step = step + 1) {
        let sample = tunnel_field(ro + rd * t);
        if sample.x < 0.00135 * t {
            hit = vec3<f32>(t, sample.yz);
            break;
        }
        t = t + max(sample.x * 0.58, 0.0045);
        if t > 24.0 { break; }
    }
    if hit.x < 0.0 { return out; }

    let p = ro + rd * hit.x;
    let n = tunnel_normal(p);
    let tunnel_q = p.xy - tunnel_centerline(p.z);
    let tunnel_angle = atan2(tunnel_q.y, tunnel_q.x);
    let fresnel = 0.48 + 0.52 * pow(1.0 - max(dot(-rd, n), 0.0), 4.0);
    let graphite = mix(vec3<f32>(0.115, 0.125, 0.150), vec3<f32>(0.20, 0.21, 0.24), hit.y);
    let spec = pow(max(dot(reflect(rd, n), normalize(vec3<f32>(0.35, 0.55, 0.75))), 0.0), 90.0);
    // Sparse longitudinal veins wind with the rotating section. Their pattern
    // is fixed in world space, so the camera visibly overtakes it; only the
    // hot pulse travels, and it travels more slowly on the beat grid.
    let vein_phase = tunnel_angle * 4.0
        + tunnel_twist(p.z) * 1.35
        + sin(p.z * 0.31) * 0.75;
    let vein_ridge = 1.0 - abs(sin(vein_phase));
    let vein_line = pow(smoothstep(0.91, 0.995, vein_ridge), 2.4);
    let vein_breaks = smoothstep(
        0.18,
        0.72,
        0.5 + 0.5 * sin(p.z * 1.73 + tunnel_angle * 2.1),
    );
    let vein_surge = 0.28
        + 0.72 * pow(
            0.5 + 0.5 * sin(p.z * 0.72 - u.music.z * PI * 0.5),
            4.0,
        );
    let wall_vein = vein_line * vein_breaks * vein_surge * (1.0 - hit.y * 0.72);
    let orange = LENS_PEACH
        * (wall_vein * (0.34 + u.audio.y * 0.26)
            + hit.z * (0.30 + u.audio.x * 0.32));
    let depth_fade = exp(-hit.x * 0.075);
    let twist_sheen = 0.5 + 0.5 * sin(atan2(n.y, n.x) * 3.0 + tunnel_twist(p.z));
    let key = 0.32 + 0.50 * max(dot(n, normalize(vec3<f32>(-0.45, 0.62, 0.64))), 0.0);
    // Broad alternating ribs make the rotating cross-section readable without
    // returning to luminous target hoops.
    let rib = 0.5 + 0.5 * sin(p.z * 0.72 + tunnel_twist(p.z) + atan2(n.y, n.x) * 1.5);
    let surface = (graphite * (key + fresnel * 0.32 + twist_sheen * 0.10 + rib * 0.08)
        + vec3<f32>(0.42, 0.46, 0.54) * spec * 0.32
        + orange);
    out.color = mix(vec3<f32>(0.005, 0.007, 0.012), surface, depth_fade)
        + orange * (1.0 - depth_fade) * 0.18;
    out.depth = clip_depth(p);
    return out;
}
