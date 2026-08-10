// Pass 1 — the liquid-metal blob against the room. Outputs HDR colour, and
// writes depth from the hit distance so rasterized geometry sorts against it.

struct SceneOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

// Bounding sphere for the blob, so rays skip straight to it instead of
// marching empty space.
const BOUND_RADIUS: f32 = 1.6;
/// Extra reach once the spikes are out, so their tips are not clipped away.
const SPIKE_BOUND: f32 = 0.95;
/// How much further the arms reach once the body has gathered.
const SPIKE_GROWTH: f32 = 0.45;

// How wide the merge fillet is. Large k = strong surface tension: surfaces
// reach for each other and neck together long before they actually touch.
const BLEND_K: f32 = 0.40;

// A single blob. Each one breathes on its own axes and carries a slow surface
// ripple, so no two are the same shape at the same moment.
fn sd_blob(q: vec3<f32>, r: f32, seed: f32) -> f32 {
    let t = u.time;

    let stretch = vec3<f32>(
        1.0 + 0.24 * sin(t * 0.83 + seed * 2.1),
        1.0 + 0.24 * sin(t * 0.71 + seed * 3.7 + 1.3),
        1.0 + 0.24 * sin(t * 0.61 + seed * 1.9 + 2.6),
    );
    let squash = min(stretch.x, min(stretch.y, stretch.z));
    let ellipsoid = (length(q / stretch) - r) * squash;

    let ripple = sin(q.x * 6.5 + t * 1.1 + seed)
        * sin(q.y * 6.5 - t * 0.9 + seed * 2.0)
        * sin(q.z * 6.5 + t * 0.7);

    // Spikes push the surface out along the direction from this blob's centre,
    // in a frame that lags further behind the body the further out it is.
    //
    // A point at radius r feels the rotation the body had a moment ago, so
    // sampling the spike field at angle (spin - lag * r) sweeps the arms back
    // into trailing curves — the way anything flexible behaves when spun in
    // water.
    let radius = max(length(q), 1.0e-4);
    let lag = max(radius - BLOB_RADIUS, 0.0) * SPIKE_LAG * u.motion.x;

    // Both axes trail, the tilt a little less than the yaw, so the arms sweep
    // rather than sitting in one plane.
    let unrotated = q / radius;
    let direction = rot_x(-(u.motion.z - lag * 0.55)) * (rot_y(-(u.motion.y - lag)) * unrotated);
    // Far more reactive than the blob's own breathing: the bass drives the
    // length directly and the beat pulse kicks it further.
    // Kept in check: every unit of reach costs march distance, and the steps
    // are already short because of the twist.
    let drive = 0.3 + u.audio.x * 0.85 + u.audio.w * 0.55;
    // The arms lengthen as the body gathers, so merging and reaching read as
    // one gesture.
    let reach = SPIKE_LENGTH * (1.0 + u.motion.x * SPIKE_GROWTH);
    let bristle = spikes(direction) * u.scene.x * drive * reach;

    return ellipsoid - ripple * 0.03 - bristle;
}

// The liquid-metal blob suspended inside the sphere.
// Returns (distance, seam) where seam is how much of this point is fillet
// rather than plain surface — i.e. how hard the field is being pulled here.
fn inner_field(p: vec3<f32>) -> vec2<f32> {
    let t = u.time;
    var soft = 1.0e9;
    var hard = 1.0e9;

    // Blobs on lissajous paths that deliberately pass through each other, so
    // there's a constant cycle of merging and pinching off.
    for (var i = 0u; i < BLOB_COUNT; i = i + 1u) {
        let fi = f32(i);
        let c = blob_center(i, t);
        // Swell on the beat pulse rather than the raw spectrum: the pulse is a
        // clean exponential decay, where a band is noisy enough to make the
        // surface shimmer instead of breathe.
        // Shrinks to nothing as it bleeds out into the water.
        let r = (0.32 + 0.06 * sin(t * 1.3 + fi) + u.audio.w * 0.05) * (1.0 - u.collapse.y);
        let d = sd_blob(p - c, r, fi);
        soft = smin_cubic(soft, d, BLEND_K);
        hard = min(hard, d);
    }

    return vec2<f32>(soft, clamp((hard - soft) / (BLEND_K * 0.35), 0.0, 1.0));
}

fn inner_sdf(p: vec3<f32>) -> f32 {
    return inner_field(p).x;
}

fn inner_normal(p: vec3<f32>) -> vec3<f32> {
    let e = vec2<f32>(1.0, -1.0) * 0.0015;
    return normalize(
        e.xyy * inner_sdf(p + e.xyy) + e.yyx * inner_sdf(p + e.yyx) + e.yxy * inner_sdf(p + e.yxy) + e.xxx * inner_sdf(p + e.xxx),
    );
}

// March the blob between the bounding sphere's walls. Returns t, or -1 on miss.
fn march_inner(ro: vec3<f32>, rd: vec3<f32>, t_min: f32, t_max: f32) -> f32 {
    var t = t_min;
    for (var i = 0; i < 96; i = i + 1) {
        let d = inner_sdf(ro + rd * t);
        if d < 0.0015 * t {
            return t;
        }
        // Understep: the ripple and the spikes make the field non-Lipschitz,
        // so a full step would punch through them. Spikes are much worse than
        // the ripple, so the factor tightens as they extend.
        // The twist shears the field on top of everything else, so the step
        // shortens again once the arms are trailing.
        t = t + d * mix(0.7, 0.46, u.scene.x * (0.4 + u.motion.x * 0.6));
        if t > t_max {
            break;
        }
    }
    return -1.0;
}

// Cone-traced ambient occlusion: step out along the normal and compare how far
// the field says we are from the surface against how far we actually moved.
// Where a crevice closes in, the field lags behind and the gap is the occlusion.
fn blob_ao(p: vec3<f32>, n: vec3<f32>) -> f32 {
    var occlusion = 0.0;
    var weight = 1.0;
    for (var i = 0; i < 5; i = i + 1) {
        let step = 0.02 + 0.11 * f32(i);
        occlusion = occlusion + (step - inner_sdf(p + n * step)) * weight;
        weight = weight * 0.72;
    }
    return clamp(1.0 - 2.2 * occlusion, 0.0, 1.0);
}

// Soft shadow by marching toward the light: the closest the ray passes to the
// surface, relative to how far it has travelled, gives the penumbra for free.
fn blob_shadow(p: vec3<f32>, light: vec3<f32>) -> f32 {
    var result = 1.0;
    var t = 0.04;
    for (var i = 0; i < 28; i = i + 1) {
        let d = inner_sdf(p + light * t);
        if d < 0.001 {
            return 0.0;
        }
        result = min(result, 10.0 * d / t);
        t = t + clamp(d, 0.02, 0.25);
        if t > 2.5 {
            break;
        }
    }
    return clamp(result, 0.0, 1.0);
}

// Liquid metal: almost pure reflection, tinted, with the merge seams running
// hotter — that's what sells it as a fluid rather than a chrome solid.
fn shade_inner(p: vec3<f32>, rd: vec3<f32>) -> vec3<f32> {
    let n = inner_normal(p);
    let l = normalize(SUN_DIR);
    let seam = inner_field(p).y;

    let refl_dir = reflect(rd, n);
    let refl = environment(refl_dir);

    // Metals have high reflectance even head-on, rising to 1.0 at grazing.
    let fres = 0.72 + 0.28 * pow(1.0 - max(dot(-rd, n), 0.0), 5.0);

    // Cool steel over the body, molten where surfaces are pulling together —
    // the same warm accent as the room's veins, so the palette stays coherent.
    let steel = vec3<f32>(0.62, 0.68, 0.78);
    let tint = mix(steel, VEIN_CORE, seam * 0.9);

    // Seams glow a little on their own, brightest as the necks pinch.
    let glow = VEIN_COLOR * pow(seam, 2.5) * (0.25 + u.audio.w * 0.45);

    // Contact shading. Without these the blob reads as pasted onto the room:
    // occlusion darkens the crevices between merging lobes, and the shadow term
    // stops lobes from being lit through one another.
    let ao = blob_ao(p, n);
    let shadow = blob_shadow(p, l);

    // Spike tips glint: the further out along a spike a point is, the more it
    // catches, and the highs make them twinkle.
    let from_center = length(p);
    // Only the last fifth of a spike catches: a sheen spread down the flanks
    // reads as a wet coating rather than as points catching the light.
    let grown = SPIKE_LENGTH * (1.0 + u.motion.x * SPIKE_GROWTH);
    let tip = smoothstep(
        BLOB_RADIUS + grown * 0.62,
        BLOB_RADIUS + grown * 0.95,
        from_center,
    ) * u.scene.x;
    let sparkle = pow(
        smoothstep(0.55, 1.0, vnoise(normalize(p) * 24.0 + vec3<f32>(0.0, u.time * 1.6, 0.0))),
        2.0,
    );
    let glint = tip * (0.10 + sparkle * 0.45) * (0.3 + band(13u) * 0.7);

    let spec = pow(max(dot(refl_dir, l), 0.0), 220.0) * 5.0 * shadow;
    let sheen = pow(max(dot(n, l), 0.0), 2.0) * 0.12 * shadow;

    // Seams also darken slightly at their deepest point, like a meniscus.
    let meniscus = 1.0 - seam * 0.25;

    // Reflections are only partly occluded — a mirror in a corner still shows
    // the room, just less of it.
    let reflected = refl * fres * tint * meniscus * mix(1.0, ao, 0.6);

    return reflected
        + vec3<f32>(1.0, 0.94, 0.86) * spec
        + tint * sheen
        + glow * ao
        // Tinted rather than white: a white highlight this large washes the
        // steel out entirely.
        + mix(vec3<f32>(0.75, 0.82, 1.0), VEIN_CORE, 0.35) * glint;
}

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

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> FullscreenOut {
    return fullscreen_vertex(vi);
}

@fragment
fn fs_main(in: FullscreenOut) -> SceneOut {
    let ro = u.camera_pos.xyz;
    let rd = camera_ray(in.uv);

    // u.intro.z fades the scene up out of black. It is applied here rather
    // than in post, or it would fade the intro text along with the scene.
    let fade = u.intro.z;

    var out: SceneOut;
    out.color = vec4<f32>(environment(rd) * fade, 1.0);
    out.depth = 1.0; // background sits at the far plane

    // Only once the room has actually gone. Swapping at half way meant the
    // collapse was over before it could be seen.
    if u.collapse.x > 0.9 {
        let hit = march_fractal(ro, rd);
        if hit.x > 0.0 {
            let p = ro + rd * hit.x;
            // Re-evaluate at the hit for its orbit; one extra sample is far
            // cheaper than carrying a vec4 out of the march loop.
            let orbit = fractal(p).orbit;
            out.color = vec4<f32>(shade_fractal(p, rd, orbit, hit.z, hit.x), 1.0);
            out.depth = clip_depth(p);
        }
        return out;
    }

    let bounds = intersect_sphere(
        ro,
        rd,
        BOUND_RADIUS + SPIKE_BOUND * u.scene.x * (1.0 + u.motion.x * SPIKE_GROWTH),
    );
    if bounds.y < 0.0 {
        return out;
    }

    let t = march_inner(ro, rd, max(bounds.x, 0.0), bounds.y);
    if t < 0.0 {
        return out;
    }

    let hit = ro + rd * t;
    out.color = vec4<f32>(shade_inner(hit, rd) * fade, 1.0);
    out.depth = clip_depth(hit);
    return out;
}
