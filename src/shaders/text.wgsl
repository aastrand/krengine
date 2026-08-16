// Scrolling text, and the fireflies that light it.
//
// Glyphs are instanced quads sampling a font atlas. Fireflies are a second
// swarm of additive points that drift along the text; where one passes close to
// a letter it brightens that letter, so the text blinks because something is
// visibly lighting it rather than because a global brightness was animated.

@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct Glyph {
    rect: vec4<f32>,
    uv: vec4<f32>,
    index: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
};
@group(1) @binding(2) var<storage, read> glyphs: array<Glyph>;

/// Height of the text as a fraction of the screen.
const TEXT_SCALE: f32 = 0.13;
/// How fast the text drifts, in screen widths per second.
const SCROLL_SPEED: f32 = 0.055;
/// How fast the highlight travels across the letters, in sweeps per second.
const SHEEN_SPEED: f32 = 0.64;
/// Narrowness of that highlight.
const SHEEN_WIDTH: f32 = 1.6;

const QUAD: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(0.0, 1.0),
);

/// A glint travelling across the letterforms.
///
/// Not lights in the scene — the letters themselves catch the light, the way
/// polished type does when it turns. A broad sweep provides the movement and a
/// fine sparkle rides on top of it, both keyed to the music.
fn shimmer(p: vec2<f32>) -> f32 {
    // One pass per card, not a repeating loop: the highlight crosses the word
    // once while the card is up. u.card.y is how far through its life it is.
    let travel = mix(-1.8, 2.4, smoothstep(0.05, 0.75, u.card.y));
    let across = p.x * 0.55 + p.y * 0.35;
    let sweep = exp(-pow((across - travel) * SHEEN_WIDTH, 2.0));

    // Fine grain that twinkles with the top of the mix.
    let grain = vnoise(vec3<f32>(p * 9.0, u.time * 0.7));
    // Sparkle rides the sweep rather than running the whole time, so the
    // letters twinkle as the glint passes and are still otherwise.
    let sparkle = pow(smoothstep(0.62, 1.0, grain), 3.0)
        * (0.35 + band(12u) * 2.2)
        * (0.15 + sweep * 1.6);

    // Body brightness follows the mids, so the word breathes with the music.
    let body = 0.35 + band(7u) * 0.8;

    return body + sweep * (0.7 + band(3u) * 1.9) + sparkle;
}

/// Text-space to clip-space. Text space has y up, one unit is the cap height.
fn text_to_clip(p: vec2<f32>) -> vec4<f32> {
    let aspect = u.resolution.x / max(u.resolution.y, 1.0);
    let scrolled = p + vec2<f32>(u.intro.w, 0.0);
    // u.card.x is how near the card reads: bigger is closer.
    let scale = TEXT_SCALE * u.card.x;
    // u.card.z shifts the card in clip space, so its height does not change
    // with its size.
    return vec4<f32>(
        scrolled.x * scale / aspect * 2.0 + u.card.z,
        (scrolled.y - 0.35) * scale * 2.0 + u.card.w,
        0.0,
        1.0,
    );
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) text_pos: vec2<f32>,
};

@vertex
fn vs_text(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let glyph = glyphs[ii];
    let corner = QUAD[vi];

    let local = glyph.rect.xy + corner * glyph.rect.zw;

    var out: VsOut;
    out.pos = text_to_clip(local);
    out.uv = mix(glyph.uv.xy, glyph.uv.zw, vec2<f32>(corner.x, 1.0 - corner.y));
    out.text_pos = local;
    return out;
}

@fragment
fn fs_text(in: VsOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(atlas, atlas_sampler, in.uv).r;
    if coverage < 0.01 {
        discard;
    }

    let glint = shimmer(in.text_pos);

    // Kept above 1.0 where it glints, so bloom picks the highlights up.
    let bright = vec3<f32>(0.30, 0.31, 0.35) + vec3<f32>(1.0, 0.90, 0.72) * glint;

    // Credits shimmer the other way: near-black already, they cannot glint
    // lighter, so the sweep drives them to pure black instead. Against a lit
    // room that reads as a highlight passing over — just an inverted one.
    let dark = mix(
        vec3<f32>(0.085, 0.088, 0.10),
        vec3<f32>(0.0),
        clamp(glint * 0.85, 0.0, 1.0),
    );

    // The attribution and six fractal greetings are quiet dark inscriptions.
    // The final two cards return to the bright opening treatment against black.
    let credits = step(2.5, u.intro.x) * (1.0 - step(9.5, u.intro.x));
    let ink = mix(bright, dark, credits);

    // Credits also thicken slightly as the sweep passes, so the shimmer is
    // legible on letters this small.
    let alpha = coverage * u.intro.y * mix(1.0, 0.8 + clamp(glint, 0.0, 1.0) * 0.2, credits);
    return vec4<f32>(ink * alpha, alpha);
}
