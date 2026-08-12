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

// --- living lens field -------------------------------------------------

const LENS_PEARL: vec3<f32> = vec3<f32>(0.72, 0.66, 0.61);
const LENS_IVORY: vec3<f32> = vec3<f32>(0.64, 0.52, 0.43);
const LENS_PEACH: vec3<f32> = vec3<f32>(1.00, 0.34, 0.08);
const LENS_SILVER: vec3<f32> = vec3<f32>(0.28, 0.30, 0.35);
const LENS_SHADOW: vec3<f32> = vec3<f32>(0.07, 0.09, 0.14);
const LENS_INK: vec3<f32> = vec3<f32>(0.012, 0.015, 0.025);
const LENS_HOT: vec3<f32> = vec3<f32>(1.0, 0.94, 0.87);

struct RayScene {
    color: vec3<f32>,
    depth: f32,
};

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
    let tangent = safe_direction(cross(n, vec3<f32>(0.12, 1.0, 0.08)), vec3<f32>(1.0, 0.0, 0.0));
    let wave_phase = dot(p - center, tangent) * 10.0 - u.music.z * PI * 2.0 + fi * 1.7;
    let wave = sin(wave_phase) * (0.010 + u.audio.y * 0.038);

    let facing = max(dot(-rd, n), 0.0);
    let fresnel = pow(1.0 - facing, 3.0);
    // A single sampled surface only bends a ray weakly. Approximate the exit
    // surface as well by drawing each channel toward the lens's optical axis:
    // background panels are visibly magnified and displaced inside the shape,
    // while the small eta difference leaves a restrained spectral fringe.
    let optical_axis = safe_direction(center - ro, rd);
    let radial_bend = 0.075 + (1.0 - facing) * 0.20 + u.audio.y * 0.035;
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

    let caustic = pow(max(sin(wave_phase * 0.5), 0.0), 8.0) * (0.15 + u.audio.y * 0.85);
    var membrane = mix(refracted, LENS_PEARL, 0.012 + thickness * 0.006);
    membrane = mix(membrane, LENS_IVORY, fresnel * 0.10);
    // Transparency here means seeing a displaced background, not retaining the
    // undisplaced ray. Make that optical image dominant across the membrane.
    let optical_mix = clamp(0.66 + u.audio.y * 0.06 - fresnel * 0.06, 0.62, 0.74);
    var color = mix(out.color, membrane, optical_mix);
    let interference = pow(0.5 + 0.5 * sin(wave_phase * 1.35 + thickness * 2.8), 18.0);
    color = color + LENS_PEACH
        * (fresnel * 1.45 + caustic * 0.42 + interference * (0.06 + u.audio.z * 0.18));

    let light = normalize(vec3<f32>(0.45, 0.72, 0.52));
    let spec = pow(max(dot(reflect(rd, n), light), 0.0), 180.0) * 4.0;
    color = color + LENS_HOT * spec * (0.45 + band(14u) * 0.55);

    out.color = color;
    out.depth = clip_depth(p);
    return out;
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
    //
    // This threshold sits inside the white wash's plateau — see WASH_HOLD and
    // WASH_BACK in timeline.rs, which bracket it — so the hard change from one
    // geometry to the other happens on a frame that is solid white. Move it
    // outside that window and the swap becomes a visible cut again.
    if u.collapse.x > 0.9 {
        if u.lens.z > 0.999 {
            let lenses = render_lens_scene(ro, rd);
            out.color = vec4<f32>(lenses.color * fade, 1.0);
            out.depth = lenses.depth;
            return out;
        }

        // One circular aperture seals over the fractal. It first bends the old
        // room, then expands across the camera while revealing the new field
        // inside the same physical boundary.
        let aspect = u.resolution.x / max(u.resolution.y, 1.0);
        let membrane_center = vec2<f32>(0.13, -0.04);
        let q = vec2<f32>((in.uv.x - membrane_center.x) * aspect, in.uv.y - membrane_center.y);
        let radial = length(q);
        let radius = 0.58 + u.lens.y * 1.75;
        let inside = 1.0 - smoothstep(radius - 0.025, radius + 0.025, radial);

        var sample_uv = in.uv;
        if inside > 0.0 && radial > 1.0e-4 {
            let bend = (1.0 - clamp(radial / radius, 0.0, 1.0))
                * u.lens.x * (1.0 - u.lens.z) * 0.12;
            sample_uv = sample_uv + vec2<f32>(q.x / aspect, q.y) / radial * bend;
        }
        let old = render_fractal_scene(ro, camera_ray(sample_uv));
        let lenses = render_lens_scene(ro, rd);
        let reveal = inside * u.lens.z;
        out.color = vec4<f32>(mix(old.color, lenses.color, reveal) * fade, 1.0);
        out.depth = select(old.depth, lenses.depth, reveal > 0.5);

        // Peach interference along the sealing edge makes it read as a film,
        // rather than as a circular crossfade painted over the frame.
        let rim = exp(-abs(radial - radius) * 85.0) * u.lens.x * (1.0 - u.lens.z * 0.65);
        out.color = vec4<f32>(out.color.rgb + LENS_PEACH * rim * (0.55 + u.audio.z * 0.45), 1.0);
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
