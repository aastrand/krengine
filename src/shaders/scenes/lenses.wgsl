// --- living lens field -------------------------------------------------

const LENS_PEARL: vec3<f32> = vec3<f32>(0.72, 0.66, 0.61);
const LENS_IVORY: vec3<f32> = vec3<f32>(0.64, 0.52, 0.43);
const LENS_PEACH: vec3<f32> = vec3<f32>(1.00, 0.34, 0.08);
const LENS_SILVER: vec3<f32> = vec3<f32>(0.28, 0.30, 0.35);
const LENS_SHADOW: vec3<f32> = vec3<f32>(0.07, 0.09, 0.14);
const LENS_INK: vec3<f32> = vec3<f32>(0.012, 0.015, 0.025);
const LENS_HOT: vec3<f32> = vec3<f32>(1.0, 0.94, 0.87);

// Arrangement envelope authored from the isolated synth stem. The lead drains
// away over beats 176-184, holds sparse through 192, then charges back into
// the tunnel cut at beat 200. Live FFT remains layered on top of this shape.
fn lens_stem_phrase() -> vec2<f32> {
    let hush_in = smoothstep(176.0, 184.0, u.music.z);
    let rebuild = smoothstep(192.0, 200.0, u.music.z);
    let presence = mix(1.0 - hush_in * 0.72, 1.0, rebuild);
    return vec2<f32>(presence, rebuild);
}

fn lens_environment(ro: vec3<f32>, rd: vec3<f32>) -> vec3<f32> {
    // A revised version of the first room: the camera again sits inside a
    // large sphere, now darker silver with sparse orange seams. Its structure
    // gives refraction visible material to displace without returning to the
    // exact blue wall treatment of the opening scenes.
    let shell = intersect_sphere(ro, rd, 12.0);
    let p = ro + rd * max(shell.y, 0.0);
    let dir = normalize(p);
    let horizon = smoothstep(-0.75, 0.80, dir.y);
    var color = mix(LENS_SHADOW, LENS_SILVER, horizon);

    let field = sin(dir.x * 7.0 + sin(dir.y * 4.0 + u.time * 0.05) * 1.4)
        * sin(dir.y * 6.0 - dir.z * 3.5)
        * sin(dir.z * 7.5 + dir.x * 2.0);
    let panels = smoothstep(-0.25, 0.75, field) * 0.12;
    let seam = pow(clamp(1.0 - abs(field) * 2.2, 0.0, 1.0), 10.0);
    color = color + LENS_PEARL * panels;
    color = color + LENS_PEACH * seam * (0.18 + band(8u) * 0.22);

    let pearl_haze = pow(max(dot(rd, normalize(vec3<f32>(-0.35, 0.28, -0.90))), 0.0), 7.0);
    let warm_haze = pow(max(dot(rd, normalize(vec3<f32>(0.62, 0.42, -0.65))), 0.0), 22.0);
    color = color + LENS_PEARL * pearl_haze * 0.16 + LENS_PEACH * warm_haze * 0.24;
    return color;
}


/// Union sample: distance and which membrane supplied it.
fn lens_field(p: vec3<f32>) -> vec2<f32> {
    var nearest = vec2<f32>(1.0e9, 0.0);
    for (var i = 0u; i < LENS_COUNT; i = i + 1u) {
        let q = p - lens_center(i);
        let length_q = max(length(q), 1.0e-5);
        let d = length_q - lens_shape_radius(q / length_q, i);
        if d < nearest.x {
            nearest = vec2<f32>(d, f32(i));
        }
    }
    return nearest;
}

fn lens_normal(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(0.004, -0.004);
    return normalize(
        e.xyy * lens_field(p + e.xyy).x
            + e.yyx * lens_field(p + e.yyx).x
            + e.yxy * lens_field(p + e.yxy).x
            + e.xxx * lens_field(p + e.xxx).x,
    );
}

/// Understepped because the directional radius field is intentionally not a
/// perfect SDF. Returns distance, membrane index, or a negative distance on a
/// miss.
fn march_lenses(ro: vec3<f32>, rd: vec3<f32>) -> vec2<f32> {
    var t = 0.05;
    for (var step = 0; step < 80; step = step + 1) {
        let sample = lens_field(ro + rd * t);
        if abs(sample.x) < 0.0012 * t {
            return vec2<f32>(t, sample.y);
        }
        t = t + max(sample.x * 0.48, 0.004);
        if t > 18.0 {
            break;
        }
    }
    return vec2<f32>(-1.0, 0.0);
}

fn render_lens_scene(ro: vec3<f32>, rd: vec3<f32>) -> RayScene {
    var out: RayScene;
    out.color = lens_environment(ro, rd);
    out.depth = 1.0;

    let hit = march_lenses(ro, rd);
    if hit.x < 0.0 {
        return out;
    }

    let chosen = u32(hit.y);
    let center = lens_center(chosen);
    let radius = lens_radius(chosen);
    let p = ro + rd * hit.x;
    let n = lens_normal(p);
    let fi = f32(chosen);
    let phrase = lens_stem_phrase();
    let tangent = safe_direction(cross(n, vec3<f32>(0.12, 1.0, 0.08)), vec3<f32>(1.0, 0.0, 0.0));
    let wave_phase = dot(p - center, tangent) * 10.0 - u.music.z * PI * 2.0 + fi * 1.7;
    let wave = sin(wave_phase) * (0.010 + u.audio.y * 0.038) * mix(0.42, 1.0, phrase.x);

    let facing = max(dot(-rd, n), 0.0);
    let fresnel = pow(1.0 - facing, 3.0);
    // A single sampled surface only bends a ray weakly. Approximate the exit
    // surface as well by drawing each channel toward the lens's optical axis:
    // background panels are visibly magnified and displaced inside the shape,
    // while the small eta difference leaves a restrained spectral fringe.
    let optical_axis = safe_direction(center - ro, rd);
    let radial_bend = 0.075
        + (1.0 - facing) * 0.20
        + u.audio.y * 0.035
        + phrase.y * 0.025;
    let physical_r = safe_direction(refract(rd, n, 0.76) + tangent * wave * 2.4, rd);
    let physical_g = safe_direction(refract(rd, n, 0.74) + tangent * wave * 2.4, rd);
    let physical_b = safe_direction(refract(rd, n, 0.72) + tangent * wave * 2.4, rd);
    let refracted_r = safe_direction(mix(physical_r, optical_axis, radial_bend * 0.94), rd);
    let refracted_g = safe_direction(mix(physical_g, optical_axis, radial_bend), rd);
    let refracted_b = safe_direction(mix(physical_b, optical_axis, radial_bend * 1.06), rd);
    let refracted = vec3<f32>(
        lens_environment(ro, refracted_r).r,
        lens_environment(ro, refracted_g).g,
        lens_environment(ro, refracted_b).b,
    );
    let thickness = radius * facing * 1.6;

    let caustic = pow(max(sin(wave_phase * 0.5), 0.0), 8.0)
        * (0.15 + u.audio.y * 0.85)
        * mix(0.35, 1.0, phrase.x);
    var membrane = mix(refracted, LENS_PEARL, 0.012 + thickness * 0.006);
    membrane = mix(membrane, LENS_IVORY, fresnel * 0.10);
    // Transparency here means seeing a displaced background, not retaining the
    // undisplaced ray. Make that optical image dominant across the membrane.
    let optical_mix = clamp(0.66 + u.audio.y * 0.06 - fresnel * 0.06, 0.62, 0.74);
    var color = mix(out.color, membrane, optical_mix);
    let interference = pow(0.5 + 0.5 * sin(wave_phase * 1.35 + thickness * 2.8), 18.0);
    color = color + LENS_PEACH
        * (fresnel * mix(0.72, 1.45, phrase.x)
            + caustic * 0.42
            + interference * (0.06 + u.audio.z * 0.18)
            + phrase.y * fresnel * 0.34);

    let light = normalize(vec3<f32>(0.45, 0.72, 0.52));
    let spec = pow(max(dot(reflect(rd, n), light), 0.0), 180.0) * 4.0;
    color = color + LENS_HOT * spec
        * (0.30 + phrase.x * 0.15 + band(14u) * 0.55 + phrase.y * 0.24);

    out.color = color;
    out.depth = clip_depth(p);
    return out;
}
