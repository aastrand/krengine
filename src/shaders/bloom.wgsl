// Bloom, as a mip chain: threshold the bright parts, halve the resolution a few
// times, then add the levels back on the way up. Blurring by downsampling costs
// a fraction of a wide gaussian and the result is smoother.

@group(1) @binding(0) var src: texture_2d<f32>;
@group(1) @binding(1) var src_sampler: sampler;

/// Below this, nothing blooms. The veins and speculars are written above 1.0
/// specifically to clear it.
const THRESHOLD: f32 = 1.0;
/// Width of the soft shoulder around the threshold, so bloom fades in rather
/// than switching on and crawling along edges.
const KNEE: f32 = 0.6;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> FullscreenOut {
    return fullscreen_vertex(vi);
}

fn uv_of(in: FullscreenOut) -> vec2<f32> {
    return in.uv * vec2<f32>(0.5, -0.5) + 0.5;
}

// Keep only what's brighter than the threshold, with a quadratic knee.
@fragment
fn fs_prefilter(in: FullscreenOut) -> @location(0) vec4<f32> {
    let color = textureSample(src, src_sampler, uv_of(in)).rgb;
    let brightness = max(color.r, max(color.g, color.b));
    // The lens section is built from transparent highlights rather than solid
    // emissive veins, so let its orange rims enter bloom sooner.
    let threshold = mix(THRESHOLD, 0.72, u.lens.z);

    let soft = clamp(brightness - threshold + KNEE, 0.0, 2.0 * KNEE);
    let contribution = max(
        soft * soft / (4.0 * KNEE + 0.0001),
        brightness - threshold,
    );

    return vec4<f32>(color * (contribution / max(brightness, 0.0001)), 1.0);
}

// Four bilinear taps at the corners of the source texel quad: each tap is
// already an average of 2x2, so this is a 4x4 box for the price of four.
@fragment
fn fs_downsample(in: FullscreenOut) -> @location(0) vec4<f32> {
    let uv = uv_of(in);
    let texel = 1.0 / vec2<f32>(textureDimensions(src));
    var sum = textureSample(src, src_sampler, uv + texel * vec2<f32>(-1.0, -1.0)).rgb;
    sum += textureSample(src, src_sampler, uv + texel * vec2<f32>(1.0, -1.0)).rgb;
    sum += textureSample(src, src_sampler, uv + texel * vec2<f32>(-1.0, 1.0)).rgb;
    sum += textureSample(src, src_sampler, uv + texel * vec2<f32>(1.0, 1.0)).rgb;
    return vec4<f32>(sum * 0.25, 1.0);
}

// 3x3 tent on the way back up, additively blended onto the larger level.
@fragment
fn fs_upsample(in: FullscreenOut) -> @location(0) vec4<f32> {
    let uv = uv_of(in);
    let texel = 1.0 / vec2<f32>(textureDimensions(src));
    let d = texel * 1.0;

    var sum = textureSample(src, src_sampler, uv + vec2<f32>(-d.x, -d.y)).rgb * 1.0;
    sum += textureSample(src, src_sampler, uv + vec2<f32>(0.0, -d.y)).rgb * 2.0;
    sum += textureSample(src, src_sampler, uv + vec2<f32>(d.x, -d.y)).rgb * 1.0;
    sum += textureSample(src, src_sampler, uv + vec2<f32>(-d.x, 0.0)).rgb * 2.0;
    sum += textureSample(src, src_sampler, uv).rgb * 4.0;
    sum += textureSample(src, src_sampler, uv + vec2<f32>(d.x, 0.0)).rgb * 2.0;
    sum += textureSample(src, src_sampler, uv + vec2<f32>(-d.x, d.y)).rgb * 1.0;
    sum += textureSample(src, src_sampler, uv + vec2<f32>(0.0, d.y)).rgb * 2.0;
    sum += textureSample(src, src_sampler, uv + vec2<f32>(d.x, d.y)).rgb * 1.0;

    return vec4<f32>(sum / 16.0, 1.0);
}
