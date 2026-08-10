// Renders the dye field.
//
// Schlieren-style: shade by the *gradient* of the dye rather than by its value.
// Density alone reads as flat fog; its slope is what exposes the thin shear
// layers between vortices, which is where all the visible structure lives.

@group(1) @binding(0) var dye_tex: texture_2d<f32>;
@group(1) @binding(1) var dye_sampler: sampler;
@group(1) @binding(2) var scene_depth: texture_depth_2d;

/// The sheet is solved in screen space, but it is given a place in the world:
/// a plane through the origin facing the camera. Everything nearer than that
/// plane covers it. Without this the fluid floats over the whole frame and
/// reads as a filter rather than as part of the scene.
struct LayerParams {
    offset: f32,
    width: f32,
    pad0: f32,
    pad1: f32,
};
@group(1) @binding(3) var<uniform> layer: LayerParams;

/// How strongly slope translates into shading.
const RELIEF: f32 = 30.0;
/// Direction the relief is lit from.
const LIGHT: vec3<f32> = vec3<f32>(-0.55, -0.55, 0.65);
/// Neutral grey, and dark: this is smoke, not cloud. Anything approaching
/// white reads as billowing steam against the blue room.
const TINT_LIT: vec3<f32> = vec3<f32>(0.32, 0.33, 0.35);
const TINT_SHADE: vec3<f32> = vec3<f32>(0.015, 0.015, 0.02);
/// Overall presence of the layer over the scene.
const OPACITY: f32 = 0.52;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> FullscreenOut {
    return fullscreen_vertex(vi);
}

@fragment
fn fs_main(in: FullscreenOut) -> @location(0) vec4<f32> {
    let uv = in.uv * vec2<f32>(0.5, -0.5) + 0.5;
    let texel = 1.0 / vec2<f32>(textureDimensions(dye_tex));

    let dye = textureSample(dye_tex, dye_sampler, uv).x;
    if dye < 0.004 {
        discard;
    }

    // Where the sheet sits along this ray, and what the scene put in front.
    let ro = u.camera_pos.xyz;
    let rd = camera_ray(in.uv);
    let sheet = ro + rd * (length(ro) + layer.offset);
    let scene = textureLoad(scene_depth, vec2<i32>(floor(in.pos.xy)), 0);

    // Soft rather than a hard cut, so the blob's silhouette doesn't slice the
    // smoke with an aliased edge.
    let occlusion = smoothstep(-0.0015, 0.0015, scene - clip_depth(sheet));
    if occlusion < 0.01 {
        discard;
    }

    // Central differences give the slope of the dye sheet.
    let gradient = vec2<f32>(
        textureSample(dye_tex, dye_sampler, uv + vec2<f32>(texel.x, 0.0)).x
            - textureSample(dye_tex, dye_sampler, uv - vec2<f32>(texel.x, 0.0)).x,
        textureSample(dye_tex, dye_sampler, uv + vec2<f32>(0.0, texel.y)).x
            - textureSample(dye_tex, dye_sampler, uv - vec2<f32>(0.0, texel.y)).x,
    );

    // Treat the sheet as a height field and light it, which turns slope into
    // the bright/dark banding that makes filaments legible.
    let normal = normalize(vec3<f32>(-gradient * RELIEF, 1.0));
    let lit = clamp(dot(normal, normalize(LIGHT)) * 0.5 + 0.5, 0.0, 1.0);

    // Biased toward the shaded end so the body of the smoke stays dark and
    // only the shear layers catch light.
    let color = mix(TINT_SHADE, TINT_LIT, pow(lit, 2.2));

    let coverage = clamp(dye * 1.1, 0.0, 1.0) * OPACITY * occlusion * u.intro.z;
    return vec4<f32>(color * coverage, coverage);
}
