fn fractal_normal(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(1.0, -1.0) * 0.0007;
    return normalize(
        e.xyy * fractal(p + e.xyy).distance + e.yyx * fractal(p + e.yyx).distance
            + e.yxy * fractal(p + e.yxy).distance + e.xxx * fractal(p + e.xxx).distance,
    );
}

/// Dark steel, keyed to how close the orbit came to the origin.
fn shade_fractal(
    p: vec3<f32>,
    rd: vec3<f32>,
    orbit: vec4<f32>,
    steps: f32,
    travelled: f32,
) -> vec3<f32> {
    let n = fractal_normal(p);
    let l = normalize(SUN_DIR);

    let refl = environment(reflect(rd, n));
    let fres = 0.55 + 0.45 * pow(1.0 - max(dot(-rd, n), 0.0), 5.0);

    // Where the orbit came closest tells one part of the structure from
    // another: broad sphere faces and the thin struts between them trap
    // differently, so the two can be coloured apart.
    let strut = clamp(6.0 * orbit.y, 0.0, 1.0);
    let core = pow(clamp(1.0 - 2.0 * orbit.z, 0.0, 1.0), 8.0);

    // Steel spheres and molten struts: the same two colours the whole demo has
    // used, on a structure made of nothing else.
    // Darker than it looks it should be: against a white void, mid-grey metal
    // tonemaps up to near-white, so the material has to sit low to read as
    // metal at all.
    let metal = mix(vec3<f32>(0.26, 0.30, 0.38), vec3<f32>(0.14, 0.20, 0.36), strut);
    let ember = VEIN_COLOR * core * 1.35;

    // Deep in the packing the orbit stayed near the origin, which is also
    // where light would not reach.
    let occlusion = pow(clamp(orbit.w * 2.0, 0.0, 1.0), 1.2);

    let diff = max(dot(n, l), 0.0);
    let spec = pow(max(dot(reflect(rd, n), l), 0.0), 90.0) * 2.2;

    // Steps taken stands in for occlusion: crevices need more of them, and
    // they are the parts that should sit dark.
    let cavity = clamp(1.0 - steps * 0.011, 0.25, 1.0);

    let lit = (refl * fres * 0.22 + metal * (0.12 + diff * 0.55)) * cavity * occlusion
        + vec3<f32>(0.8, 0.88, 1.0) * spec * 0.6 * cavity
        + ember * cavity;

    // Aerial perspective: near surfaces read as metal, far ones dissolve into
    // the white. This is what separates depth from noise.
    let fog = 1.0 - exp(-travelled * FRACTAL_FOG);
    return mix(lit, environment(rd), fog);
}

/// March the fractal. Steps are returned so shading can use them as a cheap
/// ambient occlusion.
fn march_fractal(ro: vec3<f32>, rd: vec3<f32>) -> vec3<f32> {
    var t = 0.02;
    var steps = 0.0;

    for (var i = 0; i < 110; i = i + 1) {
        let p = ro + rd * t;
        let hit = fractal(p);

        // Threshold grows with distance, so far detail is not marched forever.
        if hit.distance < 0.0006 * t {
            return vec3<f32>(t, hit.trap, steps);
        }
        t = t + hit.distance * 0.85;
        steps = steps + 1.0;

        if t > 14.0 {
            break;
        }
    }
    return vec3<f32>(-1.0, 0.0, steps);
}

fn render_fractal_scene(ro: vec3<f32>, rd: vec3<f32>) -> RayScene {
    var out: RayScene;
    out.color = environment(rd);
    out.depth = 1.0;

    let hit = march_fractal(ro, rd);
    if hit.x > 0.0 {
        let p = ro + rd * hit.x;
        let orbit = fractal(p).orbit;
        out.color = shade_fractal(p, rd, orbit, hit.z, hit.x);
        out.depth = clip_depth(p);
    }
    return out;
}
