// Pass 1 — the liquid-metal blob against the room. Outputs HDR colour, and
// writes depth from the hit distance so rasterized geometry sorts against it.

struct SceneOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

// Bounding sphere for the blob, so rays skip straight to it instead of
// marching empty space.
const BOUND_RADIUS: f32 = 1.6;

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

    return ellipsoid - ripple * 0.03;
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
        let r = 0.32 + 0.06 * sin(t * 1.3 + fi) + u.audio.w * 0.05;
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
        // Understep: the ripple makes the field non-Lipschitz, so full steps
        // would punch through thin surfaces.
        t = t + d * 0.7;
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

    let spec = pow(max(dot(refl_dir, l), 0.0), 220.0) * 5.0 * shadow;
    let sheen = pow(max(dot(n, l), 0.0), 2.0) * 0.12 * shadow;

    // Seams also darken slightly at their deepest point, like a meniscus.
    let meniscus = 1.0 - seam * 0.25;

    // Reflections are only partly occluded — a mirror in a corner still shows
    // the room, just less of it.
    let reflected = refl * fres * tint * meniscus * mix(1.0, ao, 0.6);

    return reflected + vec3<f32>(1.0, 0.94, 0.86) * spec + tint * sheen + glow * ao;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> FullscreenOut {
    return fullscreen_vertex(vi);
}

@fragment
fn fs_main(in: FullscreenOut) -> SceneOut {
    let ro = u.camera_pos.xyz;
    let rd = camera_ray(in.uv);

    var out: SceneOut;
    out.color = vec4<f32>(environment(rd), 1.0);
    out.depth = 1.0; // background sits at the far plane

    let bounds = intersect_sphere(ro, rd, BOUND_RADIUS);
    if bounds.y < 0.0 {
        return out;
    }

    let t = march_inner(ro, rd, max(bounds.x, 0.0), bounds.y);
    if t < 0.0 {
        return out;
    }

    let hit = ro + rd * t;
    out.color = vec4<f32>(shade_inner(hit, rd), 1.0);
    out.depth = clip_depth(hit);
    return out;
}
