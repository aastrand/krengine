// Pass 3 — resolve the HDR target to the swapchain: tonemap, vignette, grain.
// Bloom will slot in ahead of this without either neighbour changing.

@group(1) @binding(0) var hdr_tex: texture_2d<f32>;
@group(1) @binding(1) var hdr_sampler: sampler;
@group(2) @binding(0) var bloom_tex: texture_2d<f32>;
@group(2) @binding(1) var bloom_sampler: sampler;

/// How much of the blurred highlights to mix back in.
const BLOOM_STRENGTH: f32 = 0.55;

/// Vignette: where the darkening starts and how deep it goes. A gentle,
/// wide falloff bands more readily than a strong narrow one, so this is
/// deliberately paired with the dither below.
const VIGNETTE_START: f32 = 0.52;
const VIGNETTE_STRENGTH: f32 = 0.68;


// Narkowicz ACES approximation.
fn tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Debug overlay: the 16 FFT bands as bars, so a band can be picked by eye.
// Toggled with B. Drawn after tonemapping, so the bars aren't graded.
const OVERLAY_PAD: f32 = 18.0;
const OVERLAY_BAR: f32 = 22.0;
const OVERLAY_HEIGHT: f32 = 150.0;
/// Bar height is scaled so this level reaches the top of the panel.
const OVERLAY_FULL_SCALE: f32 = 1.5;

fn band_overlay(pixel: vec2<f32>, color: vec3<f32>) -> vec3<f32> {
    if u.debug.x < 0.5 {
        return color;
    }

    let x0 = OVERLAY_PAD;
    let x1 = x0 + OVERLAY_BAR * 16.0;
    let bottom = u.resolution.y - OVERLAY_PAD;
    let top = bottom - OVERLAY_HEIGHT;

    if pixel.x < x0 || pixel.x > x1 || pixel.y < top || pixel.y > bottom {
        return color;
    }

    // Dim backing panel so bars read over a bright scene.
    var out = mix(color, vec3<f32>(0.02, 0.02, 0.03), 0.72);

    let slot = (pixel.x - x0) / OVERLAY_BAR;
    let index = u32(floor(slot));
    // Gap between bars.
    if fract(slot) > 0.82 {
        return out;
    }

    // Gridline at the level the onset detector's floor sits near.
    let level = clamp(band(index), 0.0, OVERLAY_FULL_SCALE) / OVERLAY_FULL_SCALE;
    let bar_top = bottom - level * OVERLAY_HEIGHT;

    if pixel.y >= bar_top {
        // Bands 0-2 drive the onset detector; every 4th is a ruler mark.
        var tint = vec3<f32>(0.30, 0.65, 1.0);
        if index < 3u {
            tint = vec3<f32>(1.0, 0.45, 0.25);
        } else if index % 4u == 0u {
            tint = vec3<f32>(1.0, 0.85, 0.35);
        }
        out = tint;
    }

    // Beat pulse as a strip along the bottom of the panel.
    if pixel.y > bottom - 4.0 {
        out = mix(vec3<f32>(0.1), vec3<f32>(1.0, 0.3, 0.2), u.audio.w);
    }
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> FullscreenOut {
    return fullscreen_vertex(vi);
}

@fragment
fn fs_main(in: FullscreenOut) -> @location(0) vec4<f32> {
    let uv = in.uv * vec2<f32>(0.5, -0.5) + 0.5;

    // The scene target is supersampled, so a single linear tap here averages
    // the extra samples — that's the anti-aliasing.
    var color = textureSample(hdr_tex, hdr_sampler, uv).rgb;

    // Bloom is added before tonemapping, so highlights roll off together with
    // everything else rather than clipping to white.
    color += textureSample(bloom_tex, bloom_sampler, uv).rgb * BLOOM_STRENGTH;

    color = tonemap(color * 1.15);

    // Vignette, corrected for aspect so it stays circular on a wide window.
    let aspect = u.resolution.x / max(u.resolution.y, 1.0);
    let centered = vec2<f32>(in.uv.x * aspect, in.uv.y) / aspect;
    let falloff = smoothstep(VIGNETTE_START, 1.15, length(centered));
    color *= 1.0 - falloff * VIGNETTE_STRENGTH;

    // Edges also lose a little saturation, which reads as depth rather than
    // as a dark ring drawn over the image.
    let grey = dot(color, vec3<f32>(0.299, 0.587, 0.114));
    color = mix(color, vec3<f32>(grey), falloff * 0.35);

    // Dither before the swapchain quantises to 8 bits. Smooth gradients — the
    // vignette above all — step visibly without it, since 8 bits cannot
    // resolve a slow ramp across a thousand pixels.
    //
    // Two independent samples make a triangular distribution rather than a
    // uniform one: the same amount of noise hides banding far better and reads
    // as texture instead of as static.
    let n1 = fract(sin(dot(in.pos.xy + u.time, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    let n2 = fract(sin(dot(in.pos.xy - u.time, vec2<f32>(39.3467, 11.135))) * 24634.6345);
    color = color + (n1 + n2 - 1.0) * (1.6 / 255.0);

    return vec4<f32>(band_overlay(in.pos.xy, color), 1.0);
}
