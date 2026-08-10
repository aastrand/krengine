// Pass 3 — resolve the HDR target to the swapchain: tonemap, vignette, grain.
// Bloom will slot in ahead of this without either neighbour changing.

@group(1) @binding(0) var hdr_tex: texture_2d<f32>;
@group(1) @binding(1) var hdr_sampler: sampler;

// Narkowicz ACES approximation.
fn tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> FullscreenOut {
    return fullscreen_vertex(vi);
}

@fragment
fn fs_main(in: FullscreenOut) -> @location(0) vec4<f32> {
    let uv = in.uv * vec2<f32>(0.5, -0.5) + 0.5;
    var color = textureSample(hdr_tex, hdr_sampler, uv).rgb;

    color = tonemap(color * 1.15);

    // Vignette, then a touch of grain to break up the gradients.
    let v = 1.0 - 0.35 * dot(in.uv, in.uv);
    color = color * v;

    let grain = fract(sin(dot(in.pos.xy + u.time, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    color = color + (grain - 0.5) * 0.012;

    return vec4<f32>(color, 1.0);
}
