// Pass 1 — the liquid-metal blob against the room.
// Outputs HDR color plus a linear "distance along the ray" buffer that the
// particle pass uses for occlusion.

struct SceneOut {
    @location(0) color: vec4<f32>,
    @location(1) depth: vec4<f32>,
};

const MISS_DEPTH: f32 = 1.0e9;
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
    for (var i = 0u; i < 6u; i = i + 1u) {
        let fi = f32(i);
        let c = vec3<f32>(
            sin(t * 0.53 + fi * 1.7) * 0.40,
            cos(t * 0.41 + fi * 2.3) * 0.34,
            sin(t * 0.61 + fi * 0.9) * 0.40,
        );
        let d = sd_blob(p - c, 0.32 + 0.06 * sin(t * 1.3 + fi), fi);
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

    // Cool steel over the body, warmer where surfaces are pulling together.
    let steel = vec3<f32>(0.62, 0.68, 0.78);
    let molten = vec3<f32>(0.95, 0.62, 0.35);
    let tint = mix(steel, molten, seam * 0.85);

    let spec = pow(max(dot(refl_dir, l), 0.0), 220.0) * 5.0;
    let sheen = pow(max(dot(n, l), 0.0), 2.0) * 0.12;

    // Seams also darken slightly at their deepest point, like a meniscus.
    let meniscus = 1.0 - seam * 0.25;

    return (refl * fres * tint * meniscus) + vec3<f32>(1.0, 0.94, 0.86) * spec + tint * sheen;
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
    out.depth = vec4<f32>(MISS_DEPTH);

    let bounds = intersect_sphere(ro, rd, BOUND_RADIUS);
    if bounds.y < 0.0 {
        return out;
    }

    let t = march_inner(ro, rd, max(bounds.x, 0.0), bounds.y);
    if t < 0.0 {
        return out;
    }

    out.color = vec4<f32>(shade_inner(ro + rd * t, rd), 1.0);
    out.depth = vec4<f32>(t);
    return out;
}
