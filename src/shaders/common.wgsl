// Shared prelude — prepended to every WGSL module by src/shader.rs.

struct Uniforms {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    resolution: vec2<f32>,
    time: f32,
    particle_count: f32,
    /// Band envelopes and beat pulse: (low, mid, high, beat).
    audio: vec4<f32>,
    /// Song position: (row, pattern, beats elapsed, phase within the bar).
    music: vec4<f32>,
    /// FFT spectrum, 16 log-spaced bands packed four to a vector.
    bands: array<vec4<f32>, 4>,
    /// Debug switches: (band overlay, unused, unused, unused).
    debug: vec4<f32>,
    /// Per-frame values: (delta time, white wash, softened beat, bead arrival).
    frame: vec4<f32>,
    /// Intro state: (card index or -1, card opacity, scene fade, drift).
    intro: vec4<f32>,
    /// Card presentation: (scale, progress, clip x, clip y).
    card: vec4<f32>,
    /// Scene state: (spike amount, dissolve, unused, unused).
    scene: vec4<f32>,
    /// Body motion: (merge, yaw, tilt, palette shift).
    motion: vec4<f32>,
    /// Living-lens transition: (seal, crossing, field, particle release).
    lens: vec4<f32>,
    /// Tunnel scene: (covered transition, field, tentacle growth, beats of
    /// forward travel).
    tunnel: vec4<f32>,
    /// Cube sea: (covered transition, field, gravity, beats in scene).
    cubes: vec4<f32>,
    /// Outro: (cube fade, remaining cube-wave glow, final black, unused).
    outro: vec4<f32>,
    /// Room collapse: (amount, bleed, camera's position along the path, the
    /// radius it is gliding at).
    collapse: vec4<f32>,
    /// Cinematic depth of field: (focus distance, aperture strength, unused,
    /// unused). Distance is measured from the camera in world units.
    dof: vec4<f32>,
    /// The traced corridors the bead strings run along, laid end to end:
    /// STRINGS * TRACK_POINTS entries. Both counts must match fractal.rs — see
    /// the uniform_arrays_match_the_cpu test in shader.rs.
    track: array<vec4<f32>, 1536>,
    /// A perpendicular at each corridor point, for the curl to wind around.
    track_frame: array<vec4<f32>, 1536>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

// Spectrum band `i` (0 = ~40Hz, 15 = ~16kHz).
fn band(i: u32) -> f32 {
    return u.bands[i / 4u][i % 4u];
}

/// Opening titles get a deliberately dim glimpse of the vein network behind
/// them. Credits use the same card machinery later, so explicitly exclude
/// their indices rather than keying this only on card opacity.
fn intro_title_presence() -> f32 {
    let opening_card = step(-0.5, u.intro.x) * (1.0 - step(2.5, u.intro.x));
    return opening_card * smoothstep(0.06, 0.62, u.intro.y);
}

const PI: f32 = 3.14159265;
const SPHERE_RADIUS: f32 = 1.0;

// --- fullscreen triangle ------------------------------------------------

struct FullscreenOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Single oversized triangle covering the viewport; uv is in NDC (-1..1).
fn fullscreen_vertex(vi: u32) -> FullscreenOut {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = 1.0 - f32(vi & 2u) * 2.0;
    var out: FullscreenOut;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

// Depth-buffer value for a world-space point, so the raymarch can write into
// the same depth buffer rasterized geometry uses.
fn clip_depth(p: vec3<f32>) -> f32 {
    let clip = u.view_proj * vec4<f32>(p, 1.0);
    return clip.z / clip.w;
}

// Reconstruct a world-space ray direction for an NDC position.
fn camera_ray(ndc: vec2<f32>) -> vec3<f32> {
    let far = u.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    return normalize(far.xyz / far.w - u.camera_pos.xyz);
}

// --- sdf toolbox --------------------------------------------------------

fn sd_sphere(p: vec3<f32>, r: f32) -> f32 {
    return length(p) - r;
}

fn sd_round_box(p: vec3<f32>, b: vec3<f32>, r: f32) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0) - r;
}

// Polynomial smooth minimum — the metaball blend.
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

// Cubic smooth minimum. Fatter, rounder fillet than the polynomial version —
// this is what reads as surface tension when two blobs neck together.
fn smin_cubic(a: f32, b: f32, k: f32) -> f32 {
    let h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * h * k / 6.0;
}

fn rot_y(a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat3x3<f32>(vec3<f32>(c, 0.0, -s), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(s, 0.0, c));
}

fn rot_x(a: f32) -> mat3x3<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, c, s), vec3<f32>(0.0, -s, c));
}

// --- shared scene description -------------------------------------------

// Ray/sphere intersection. Returns (t_near, t_far), or (-1, -1) on a miss.
fn intersect_sphere(ro: vec3<f32>, rd: vec3<f32>, r: f32) -> vec2<f32> {
    let b = dot(ro, rd);
    let c = dot(ro, ro) - r * r;
    let h = b * b - c;
    if h < 0.0 {
        return vec2<f32>(-1.0);
    }
    let sh = sqrt(h);
    return vec2<f32>(-b - sh, -b + sh);
}

// --- the blobs ----------------------------------------------------------
//
// Centre positions live here rather than in scene.wgsl because the particles
// need to feel the same masses the raymarcher draws.

const BLOB_COUNT: u32 = 6u;

fn blob_center(i: u32, t: f32) -> vec3<f32> {
    let fi = f32(i);
    let orbit = vec3<f32>(
        sin(t * 0.53 + fi * 1.7) * 0.40,
        cos(t * 0.41 + fi * 2.3) * 0.34,
        sin(t * 0.61 + fi * 0.9) * 0.40,
    );
    // u.motion.x draws them together into one body. They keep their own
    // breathing and ripple, so the result is lumpy rather than a plain sphere.
    return orbit * (1.0 - u.motion.x);
}

/// Roughly where a blob's surface sits: radius plus the blend fillet.
const BLOB_SURFACE: f32 = 0.55;
/// How far past the surface a blob still deflects passing beads.
const BLOB_WAKE: f32 = 0.40;
/// Extra push on the beat, as a multiple of the resting deflection.
const BLOB_BEAT_GAIN: f32 = 1.6;

// How the blobs shove the particles around.
//
// Two parts: a standing deflection, so beads visibly flow around the blobs
// instead of sailing through them, and a swell of that same deflection on each
// beat. Deliberately not an inverse-square force — that grows without bound
// near the centre, and a bead displaced past a blob flips its force direction
// and chatters back and forth.
fn blob_gravity(p: vec3<f32>, t: f32) -> vec3<f32> {
    let swell = 1.0 + u.audio.w * BLOB_BEAT_GAIN;

    var offset = vec3<f32>(0.0);
    for (var i = 0u; i < BLOB_COUNT; i = i + 1u) {
        let d = p - blob_center(i, t);
        let r = max(length(d), 1.0e-4);
        let dir = d / r;

        // Hard exclusion inside the surface, plus a soft wake outside it that
        // fades with distance.
        let exclusion = max(BLOB_SURFACE - r, 0.0);
        let wake = BLOB_WAKE * exp(-max(r - BLOB_SURFACE, 0.0) / BLOB_WAKE);

        // Never displace a bead further than it is from the centre, so it can
        // be pushed away but never dragged through.
        offset = offset + dir * min((exclusion + wake * 0.45) * swell, r * 0.85);
    }
    return offset;
}

/// Ferrofluid spikes.
///
/// A field of lobes over the direction from a blob's centre, so the surface
/// bristles outward along its own normals the way a magnetised fluid does.
/// Driven by the bass, so the spikes stand up on the beat.
fn spikes(direction: vec3<f32>) -> f32 {
    let t = u.time * 0.4;
    // Three offset lobe fields, so the spacing does not read as a grid.
    let a = sin(direction.x * 7.0 + t) * sin(direction.y * 7.0 - t) * sin(direction.z * 7.0);
    let b = sin(direction.x * 4.3 - t) * sin(direction.y * 4.3) * sin(direction.z * 4.3 + t);
    let lobes = abs(a) * 0.65 + abs(b) * 0.35;

    // A high power turns rounded bumps into points.
    return pow(lobes, 4.0);
}

/// How far the spikes reach at full strength.
const SPIKE_LENGTH: f32 = 0.31;
/// Nominal blob radius, for placing things on the surface.
const BLOB_RADIUS: f32 = 0.34;
/// Radians of lag per unit of distance from the centre — what curves the arms
/// back as the body turns.
const SPIKE_LAG: f32 = 2.6;

/// Evenly spread directions over a sphere, by golden angle.
fn sphere_direction(i: u32, count: u32) -> vec3<f32> {
    let fi = f32(i) + 0.5;
    let y = 1.0 - 2.0 * fi / f32(count);
    let radius = sqrt(max(1.0 - y * y, 0.0));
    let theta = fi * 2.399963;
    return vec3<f32>(cos(theta) * radius, y, sin(theta) * radius);
}

// --- living lenses -----------------------------------------------------
// Shared by the ray-traced membranes and their satellite particles.

const LENS_COUNT: u32 = 7u;

fn lens_base_center(i: u32) -> vec3<f32> {
    switch i {
        case 0u: { return vec3<f32>(-2.70, 0.50, 0.40); }
        case 1u: { return vec3<f32>(2.50, -0.80, -0.40); }
        case 2u: { return vec3<f32>(0.00, 2.70, -2.00); }
        case 3u: { return vec3<f32>(-3.80, -1.80, -3.70); }
        case 4u: { return vec3<f32>(4.00, 1.70, -4.20); }
        case 5u: { return vec3<f32>(0.60, -2.90, -5.50); }
        default: { return vec3<f32>(-0.80, 0.50, -8.00); }
    }
}

fn lens_radius(i: u32) -> f32 {
    switch i {
        case 0u: { return 1.72; }
        case 1u: { return 1.48; }
        case 2u: { return 1.08; }
        case 3u: { return 1.68; }
        case 4u: { return 1.88; }
        case 5u: { return 1.28; }
        default: { return 2.45; }
    }
}

fn lens_center(i: u32) -> vec3<f32> {
    let fi = f32(i);
    // Large forms barely drift. Bass shifts the suspended mass rather than
    // scaling the whole object, so the response reads as weight, not bounce.
    let drift = vec3<f32>(
        sin(u.time * 0.13 + fi * 1.9),
        cos(u.time * 0.11 + fi * 2.3),
        sin(u.time * 0.09 + fi * 0.7),
    ) * (0.025 + u.audio.x * 0.035);
    return lens_base_center(i) + drift;
}

/// Radius of one membrane in a given direction. Shared with the particle pass
/// so satellites can respect the exact same audio-deformed boundary.
fn lens_shape_radius(direction: vec3<f32>, i: u32) -> f32 {
    let fi = f32(i);
    let broad = (
        sin(direction.x * 2.7 + direction.y * 1.3 + u.time * 0.23 + fi)
        + sin(direction.y * 3.1 - direction.z * 1.7 - u.time * 0.19 + fi * 1.9)
        + sin(direction.z * 2.4 + direction.x * 1.5 + u.time * 0.17 + fi * 2.7)
    ) / 3.0;
    let fold_axis = normalize(vec3<f32>(sin(fi * 1.7) + 0.3, cos(fi * 2.1), sin(fi * 0.8) + 0.2));
    let folds = sin(dot(direction, fold_axis) * 6.0 + u.time * 0.31 + fi * 2.2);
    let travelling = sin(
        dot(direction, normalize(vec3<f32>(0.7, 0.25, -0.45))) * 10.0
            - u.music.z * PI * 2.0
            + fi * 1.4,
    );
    let deformation = broad * (0.145 + u.audio.x * 0.08)
        + folds * 0.045
        + travelling * (0.020 + u.audio.y * 0.060);
    return lens_radius(i) * (1.0 + deformation);
}

/// Project a satellite outside the union of all membranes. Several passes are
/// intentional: moving clear of one overlapping lens can enter its neighbour.
fn clear_of_lenses(p: vec3<f32>) -> vec3<f32> {
    var result = p;
    for (var iteration = 0; iteration < 4; iteration = iteration + 1) {
        for (var i = 0u; i < LENS_COUNT; i = i + 1u) {
            let center = lens_center(i);
            let q = result - center;
            let distance = max(length(q), 1.0e-5);
            let direction = q / distance;
            let boundary = lens_shape_radius(direction, i) + 0.11;
            if distance < boundary {
                result = center + direction * boundary;
            }
        }
    }
    return result;
}

// Particles are beads strung along a handful of closed space curves. Each curve
// is a sum of circles at 1x, 3x, 9x, 27x frequency on tilted planes — an
// epicycle series, so the path is spline-smooth but self-similar at every
// scale, and each harmonic drifts at its own rate as time advances.
const CURVE_COUNT: u32 = 5u;
const HARMONICS: i32 = 4;

fn epicycle(s: f32, seed: f32, t: f32) -> vec3<f32> {
    var p = vec3<f32>(0.0);
    var amp = 1.0;
    var freq = 1.0;

    for (var k = 0; k < HARMONICS; k = k + 1) {
        let fk = f32(k);
        // Each harmonic spins in its own plane, defined by an axis pair.
        let axis = normalize(vec3<f32>(
            sin(fk * 2.1 + seed * 3.0),
            cos(fk * 1.7 + seed * 1.1),
            sin(fk * 0.9 + seed * 1.7) + 0.3,
        ));
        let co_axis = normalize(cross(axis, vec3<f32>(0.0, 1.0, 0.13)));

        // Integer frequency ratios keep the curve closed, so beads flow around
        // it forever without a seam.
        let phase = 2.0 * PI * freq * s + t * (0.25 + 0.19 * fk) + seed * 6.2831 + fk * 1.7;
        p = p + amp * (cos(phase) * axis + sin(phase) * co_axis);

        amp = amp * 0.42;
        freq = freq * 3.0;
    }
    return p;
}

// Where particle `i` sits at time `t`.
fn particle_pos(i: u32, t: f32) -> vec3<f32> {
    let curve = i % CURVE_COUNT;
    let along = f32(i / CURVE_COUNT);
    let per_curve = max(u.particle_count / f32(CURVE_COUNT), 1.0);

    // Beads slide along their curve at slightly different speeds per curve.
    let s = fract(along / per_curve + t * 0.014 * (1.0 + f32(curve) * 0.23));
    let seed = f32(curve) * 0.37;

    let base = rot_y(t * 0.06) * (epicycle(s, seed, t) * 1.15);
    return base + blob_gravity(base, t);
}

// --- the fractal --------------------------------------------------------
//
// A mandelbox: fold the space into a box, fold it through a sphere, scale, and
// repeat. Both folds are distance-preserving enough to keep a usable estimate,
// so the whole structure costs a dozen iterations per sample.
//
// Its scale parameter is what makes it worth using here — sweeping it morphs
// the shape continuously from open and cathedral-like to dense and spiky, and
// the structure is self-similar, so diving into it keeps revealing more.

const FRACTAL_ITERATIONS: i32 = 8;
/// Fold strength. Around 1.2 gives the packed-sphere cathedral; lower opens it
/// out, higher tightens it into gravel.
const FRACTAL_FOLD: f32 = 1.22;
/// How quickly distance washes detail out. Without it the structure is a field
/// of high-frequency detail with no depth to it, which the eye reads as noise.
const FRACTAL_FOG: f32 = 0.085;

struct FractalHit {
    distance: f32,
    /// How far the orbit was scaled up: how deep in the packing a point sits.
    trap: f32,
    /// Closest approach to each axis plane, and to the origin. Different parts
    /// of the structure come closest to different ones, so this is what tells
    /// a strut from the face of a sphere.
    orbit: vec4<f32>,
};

/// An Apollonian gasket.
///
/// Fold space into the unit cell, invert it through a sphere, repeat. Because
/// the fold is periodic the structure repeats forever, so a camera can glide
/// through it indefinitely without ever leaving or reaching a middle.
fn fractal(point: vec3<f32>) -> FractalHit {
    var p = point;
    var scale = 1.0;
    var orbit = vec4<f32>(1000.0);

    for (var i = 0; i < FRACTAL_ITERATIONS; i = i + 1) {
        // Fold into the cell: this is what makes it repeat.
        p = -1.0 + 2.0 * fract(0.5 * p + 0.5);

        let squared = max(dot(p, p), 1.0e-6);
        orbit = min(orbit, vec4<f32>(abs(p), squared));

        // Sphere inversion, which packs the spheres inside each other.
        let factor = FRACTAL_FOLD / squared;
        p = p * factor;
        scale = scale * factor;
    }

    var out: FractalHit;
    // Distance to the folded plane, unscaled back to world space.
    out.distance = 0.25 * abs(p.y) / scale;
    out.trap = scale;
    out.orbit = orbit;
    return out;
}

/// Must match STRINGS, TRACK_POINTS and TRACK_STEP in fractal.rs.
const STRINGS: u32 = 12u;
const TRACK_POINTS: u32 = 128u;
const TRACK_STEP: f32 = 0.135;
/// One corridor's full length in world units: (TRACK_POINTS - 1) * TRACK_STEP.
const TRACK_LENGTH: f32 = 17.145;

/// Distance between one bead and the next along the corridor, in world units.
/// Matched to the bead size in particles.wgsl: a little under a diameter, so
/// the string reads as one cord with the individual beads still visible along
/// it rather than as a row of separate dots or a smooth tube.
const FLOW_SPACING: f32 = 0.13;
/// Travel along the corridor, in world units per second. Slow: the strings are
/// meant to drift through the structure, not shoot down it.
const FLOW_SPEED: f32 = 0.22;
/// How much the strings differ in pace. They travel the same way — that is the
/// point of the bundle — but not in lockstep, or the threads read as one rigid
/// object being dragged along.
const FLOW_PACE_SPREAD: f32 = 0.07;

/// The curl. A string does not run down the middle of its corridor — it winds
/// around it, so the line reads as having a body and a direction of travel
/// instead of as a wire.
///
/// Every radius here is a *fraction of the clearance at that point on the
/// corridor*, not a distance. A fixed radius cannot work: the passages vary in
/// width, so one wide enough to see in the open swings beads into the walls
/// wherever the corridor narrows, and those beads are culled — which is what
/// had the strings breaking up into short fragments. Scaled to the room
/// available, the helix opens out in the voids and tightens through the gaps,
/// which is the string reading the architecture rather than ignoring it.
const CURL_RADIUS: f32 = 0.055;
/// What the beat and the bass add, again as fractions of the clearance. This
/// is the loudest part of the audio sync: the helix visibly opens out on a hit
/// and closes between.
const CURL_FROM_BEAT: f32 = 0.38;
const CURL_FROM_BASS: f32 = 0.24;
/// The most of the clearance the curl may ever use. Under 1.0 with room to
/// spare, because the clearance is sampled on the corridor and the bead sits
/// off it — and because the estimator understates the true distance anyway.
const CURL_CEILING: f32 = 0.80;

/// Radians of winding per world unit — about one turn every 1.2 units.
const CURL_RATE: f32 = 5.2;

/// A swell travelling down the string: how tightly it is wound along the
/// corridor, and how many corridor-lengths it travels per beat. This is what
/// makes the string look like it is carrying the music along its length rather
/// than pulsing all at once.
const PULSE_RATE: f32 = 1.25;
const PULSE_SPEED: f32 = 1.0;

/// How far from either end of the corridor beads fade, in world units.
///
/// The string wraps around the corridor, so the tail would otherwise pop out
/// of existence and reappear at the head. Kept to a few beads' worth: a long
/// fade had a third of the string permanently dim, which read as beads
/// dissolving in mid-air rather than as the ends of a cord. The string is also
/// well short of the corridor now (see FRACTAL_BEADS), so the head and tail
/// spend most of their time in the clear rather than sitting in the taper.
const FLOW_FADE: f32 = 0.55;

const UP: vec3<f32> = vec3<f32>(0.0, 1.0, 0.0);
const FORWARD: vec3<f32> = vec3<f32>(0.0, 0.0, 1.0);

/// normalize() that cannot return NaN.
///
/// normalize(vec3(0)) is a division by zero, and the NaN it produces does not
/// stay where it was made: a NaN multiplied by zero is still NaN, and so is
/// anything mixed with one at weight zero. So a degenerate vector here does
/// not merely give this bead a wrong position — it survives every gate meant
/// to switch the fractal's beads off and poisons the position of every
/// particle in the demo, which is exactly how the first scene lost its
/// particles: before the fractal is traced the corridor is all zeros, every
/// tangent is normalize(0), and mix(orbit, NaN, 0.0) is NaN.
fn safe_direction(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let squared = dot(v, v);
    if squared < 1.0e-12 {
        return fallback;
    }
    return v * inverseSqrt(squared);
}

/// How visible a bead is: gone inside the structure, full in the open. Fading
/// on the distance field is smooth where pushing beads out of it was not, and
/// it reads as the string passing behind the architecture.
fn fractal_flow_visibility(p: vec3<f32>) -> f32 {
    // Only culled when genuinely buried. The depth buffer already hides beads
    // behind surfaces, so this only has to catch the ones inside them, and a
    // wide fade was removing most of the string.
    return smoothstep(0.0, 0.025, fractal(p).distance);
}

/// A point on the corridor, with the frame to wind the curl around it and the
/// room there is to do it in.
struct FlowFrame {
    position: vec3<f32>,
    normal: vec3<f32>,
    tangent: vec3<f32>,
    clearance: f32,
};

/// The corridor a string runs on: traced against the structure on the CPU and
/// sampled here by distance along it.
///
/// An analytic curve cannot follow a tunnel — it has no idea where the walls
/// are, so it clips through them. This one was steered by the distance field,
/// and resampled to uniform arc length so `s` really is a distance.
fn flow_track(s: f32, string: u32) -> FlowFrame {
    let last = f32(TRACK_POINTS - 1u);
    let position = clamp(s / TRACK_STEP, 0.0, last);

    // The corridors are laid end to end in one array.
    let base = string * TRACK_POINTS;
    let index = base + u32(floor(position));
    let next = base + min(u32(floor(position)) + 1u, TRACK_POINTS - 1u);
    let f = fract(position);

    let a = u.track[index].xyz;
    let b = u.track[next].xyz;

    var out: FlowFrame;
    // Interpolated between traced points, so the string runs smoothly rather
    // than stepping from one to the next.
    out.position = mix(a, b, f);
    // The transported frame, so the curl cannot flip where the corridor bends.
    out.normal = safe_direction(mix(u.track_frame[index].xyz, u.track_frame[next].xyz, f), UP);
    out.tangent = safe_direction(b - a, FORWARD);
    out.clearance = mix(u.track[index].w, u.track[next].w, f);
    return out;
}

/// A bead in one of the strings.
///
/// Beads are dealt round-robin between the strings, so each gets an even share
/// and they stay the same length as one another. Within a string every bead
/// sits on the corridor at a fixed spacing behind the one ahead and the whole
/// thing advances together, so it moves like a string being drawn through the
/// structure rather than like a swarm that happens to share a direction. Only
/// the head's position is animated; the rest follows from the spacing.
///
/// `w` is how visible the bead is: faded at the ends of the corridor, and gone
/// where it is inside the structure.
fn fractal_flow_bead(i: u32) -> vec4<f32> {
    let string = i % STRINGS;
    let place = f32(i / STRINGS);
    let si = f32(string);

    // All strings travel the same way — the bundle is one current through the
    // structure — but at slightly different paces and starting offsets, so
    // they are not a rigid formation.
    let pace = 1.0 + (si - f32(STRINGS - 1u) * 0.5) * FLOW_PACE_SPREAD;
    let s = u.time * FLOW_SPEED * pace - place * FLOW_SPACING + si * 5.7;
    // The string wraps around the corridor, so it never runs out of track.
    let wrapped = fract(s / TRACK_LENGTH) * TRACK_LENGTH;

    let frame = flow_track(wrapped, string);
    let binormal = safe_direction(cross(frame.tangent, frame.normal), UP);

    // How wide the audio offset is right now. Its near-zero resting radius
    // leaves the traced corridor as a clean backbone; the music opens the
    // corkscrew around it on a hit
    // (u.frame.z, not the raw pulse, which rises in a single frame and made
    // the string jitter rather than swell), and the bass holds it open through
    // a loud passage.
    //
    // The swell travels: its phase runs along the corridor and advances with
    // the beat grid, so what the eye follows is a widening moving down the
    // string in time rather than the whole string breathing at once. Its own
    // phase per string, so the threads do not pulse together.
    let travel = wrapped * PULSE_RATE - u.music.z * 2.0 * PI * PULSE_SPEED + si * 2.1;
    // One narrow energy packet crosses each string per beat. It is keyed to
    // the audio beat clock, so it travels with the tune instead of making the
    // entire bundle bounce in unison.
    let beat_packet = pow(max(sin(travel), 0.0), 3.0) * u.frame.z;
    // Spread the strings across the spectrum. Each keeps a distinct musical
    // voice while the shared packet makes their motion read as one current.
    let voice = band((1u + string * 5u) % 16u);
    let drive = beat_packet * CURL_FROM_BEAT
        + (u.audio.x * 0.45 + voice * 0.55) * CURL_FROM_BASS;
    // Everything above is a fraction of what the corridor has room for here.
    let radius = clamp(CURL_RADIUS + drive, 0.0, CURL_CEILING) * frame.clearance;

    // The corkscrew is fixed to distance along the backbone, not continuously
    // spun as a whole. Audio changes its radius and gives the live packet a
    // small twist, so the string stays directional instead of reading as a
    // rotating spring.
    let phase = wrapped * CURL_RATE + si * 2.4 + beat_packet * 0.45;
    let position = frame.position
        + (frame.normal * cos(phase) + binormal * sin(phase)) * radius;

    // Faded in at the head of the corridor and out at the tail, so the wrap is
    // a bead dimming away and another brightening rather than a jump.
    let ends = smoothstep(0.0, FLOW_FADE, wrapped)
        * smoothstep(0.0, FLOW_FADE, TRACK_LENGTH - wrapped);

    return vec4<f32>(position, ends * fractal_flow_visibility(position));
}

/// Where a bead sits on the fractal.
///
/// Rather than orbiting in free space, each one is dropped onto the structure:
/// march inward along its own slowly turning direction and stop at the first
/// surface. As the direction sweeps, the bead crawls over whatever architecture
/// is beneath it — so the swarm reads the shape instead of ignoring it.
fn fractal_surface_point(i: u32, t: f32) -> vec3<f32> {
    // A slow drift, so the beads creep over the structure rather than skating.
    let dir = normalize(sphere_direction(i, 512u) + vec3<f32>(
        sin(t * 0.021 + f32(i)) * 0.22,
        cos(t * 0.016 + f32(i) * 1.7) * 0.22,
        sin(t * 0.019 + f32(i) * 0.9) * 0.22,
    ));

    // Inward from outside the structure.
    let ro = dir * 9.0;
    let rd = -dir;

    var travel = 0.0;
    for (var step = 0; step < 48; step = step + 1) {
        let d = fractal(ro + rd * travel).distance;
        if d < 0.004 {
            break;
        }
        travel = travel + d * 0.9;
        if travel > 12.0 {
            break;
        }
    }

    // Just clear of the surface, so the beads sit on it rather than in it.
    return ro + rd * (travel - 0.05);
}

// --- environment --------------------------------------------------------

const SUN_DIR: vec3<f32> = vec3<f32>(0.5, 0.72, -0.48);

// --- procedural noise ---------------------------------------------------

fn hash13(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q = q + dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = mix(hash13(i + vec3<f32>(0.0, 0.0, 0.0)), hash13(i + vec3<f32>(1.0, 0.0, 0.0)), w.x);
    let b = mix(hash13(i + vec3<f32>(0.0, 1.0, 0.0)), hash13(i + vec3<f32>(1.0, 1.0, 0.0)), w.x);
    let c = mix(hash13(i + vec3<f32>(0.0, 0.0, 1.0)), hash13(i + vec3<f32>(1.0, 0.0, 1.0)), w.x);
    let d = mix(hash13(i + vec3<f32>(0.0, 1.0, 1.0)), hash13(i + vec3<f32>(1.0, 1.0, 1.0)), w.x);
    return mix(mix(a, b, w.y), mix(c, d, w.y), w.z);
}

fn fbm(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i = i + 1) {
        sum = sum + amp * vnoise(q);
        q = q * 2.03;
        amp = amp * 0.5;
    }
    return sum;
}

// --- the room -----------------------------------------------------------
//
// The whole scene sits inside a large sphere. The camera is inside it, so this
// is both the backdrop and what the glossy sphere reflects.

const ROOM_RADIUS: f32 = 9.0;
/// What is behind the room once it has gone.
const VOID_COLOUR: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);

/// The one warm accent in an otherwise cold scene.
const VEIN_COLOR: vec3<f32> = vec3<f32>(1.0, 0.26, 0.03);
/// What the veins cool to once the ferrofluid has taken the warm role. The
/// scene only supports one warm element; when the body becomes it, the walls
/// have to give it up or the two compete.
const VEIN_COLD: vec3<f32> = vec3<f32>(0.16, 0.48, 0.95);
const VEIN_COLD_CORE: vec3<f32> = vec3<f32>(0.62, 0.86, 1.0);
/// Hotter core, so the thinnest filaments read white-hot rather than flat.
const VEIN_CORE: vec3<f32> = vec3<f32>(1.0, 0.72, 0.35);
/// Raise to widen the veins, lower to make them rarer and finer.
const VEIN_THRESHOLD: f32 = 0.87;
const VEIN_INTENSITY: f32 = 0.9;
/// Direction the pulses of energy travel through the vein network.
const VEIN_FLOW_DIR: vec3<f32> = vec3<f32>(0.35, 1.0, 0.2);
/// Surges per beat. Fractions give a surge every N beats.
const VEIN_SURGES_PER_BEAT: f32 = 0.5;

// Height field on the shell's surface. Domain-warped by time, which is what
// makes the walls look like they're slowly morphing.
fn room_height(dir: vec3<f32>) -> f32 {
    let t = u.time * 0.08;
    let warp = vec3<f32>(
        fbm(dir * 1.3 + vec3<f32>(t, 0.0, 0.0)),
        fbm(dir * 1.3 + vec3<f32>(0.0, t, 5.2)),
        fbm(dir * 1.3 + vec3<f32>(3.7, 0.0, t)),
    );
    return fbm(dir * 3.4 + warp * 1.6 + vec3<f32>(0.0, t * 0.5, 0.0));
}

/// The shell's radius now, which shrinks as the room collapses.
fn room_radius() -> f32 {
    return mix(ROOM_RADIUS, 0.0, u.collapse.x);
}

/// Where a ray meets the room shell. Negative once the shell has shrunk past
/// the camera and the ray no longer reaches it.
fn room_hit(ro: vec3<f32>, rd: vec3<f32>) -> f32 {
    let radius = room_radius();
    let b = dot(ro, rd);
    let c = dot(ro, ro) - radius * radius;
    let h = b * b - c;
    if h < 0.0 {
        return -1.0;
    }
    let root = sqrt(h);
    let far = -b + root;
    let near = -b - root;
    // Inside the shell we want the far wall; outside, the near face of what is
    // now a receding ball.
    return select(far, near, near > 0.0);
}

// Shade the inside of the shell along `rd`.
fn environment(rd: vec3<f32>) -> vec3<f32> {
    let ro = u.camera_pos.xyz;

    let distance = room_hit(ro, rd);
    if distance < 0.0 {
        // Not flat white: a surface with nothing to reflect reads as noise,
        // and a silhouette against an even field has no edge. A soft gradient
        // gives the metal something to pick up and the shape somewhere to sit.
        let height = rd.y * 0.5 + 0.5;
        let sky = mix(vec3<f32>(0.62, 0.66, 0.74), VOID_COLOUR, smoothstep(0.35, 1.0, height));
        let glow = pow(max(dot(rd, normalize(SUN_DIR)), 0.0), 12.0) * 0.35;
        return sky + vec3<f32>(1.0, 0.97, 0.92) * glow;
    }

    let p = ro + rd * distance;
    let dir = p / max(room_radius(), 1.0e-4);

    // The height field is sampled at the hit *point*, scaled by the shell's
    // full size rather than its current one. At rest the two are identical,
    // but as the shell contracts the pattern contracts with it, so the walls
    // are visibly rushing inward.
    //
    // Sampling by direction instead — which is what this did — makes a
    // shrinking sphere look completely static from inside, because the
    // directions never change. The collapse was invisible until the wall
    // crossed the camera, and then the whole frame flipped to white at once.
    let field = p / ROOM_RADIUS;
    let n = -dir; // inward-facing

    // Bump mapping: perturb the normal by the height field's tangent gradient.
    // This is the modern stand-in for an emboss map — no texture, just noise.
    let t1 = normalize(cross(select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9), n));
    let t2 = cross(n, t1);
    let e = 0.035;
    let h = room_height(field);
    let dh1 = (room_height(field + t1 * e) - h) / e;
    let dh2 = (room_height(field + t2 * e) - h) / e;
    let nb = normalize(n - (t1 * dh1 + t2 * dh2) * 0.14);

    // Albedo and roughness both driven by the same field, so the raised areas
    // read as polished and the recesses as matte.
    // The room dims as the second scene takes hold, so the body carries the
    // frame instead of sharing it with the walls.
    let recede = 1.0 - u.motion.w * 0.45;
    let albedo = mix(vec3<f32>(0.14, 0.17, 0.25), vec3<f32>(0.48, 0.55, 0.72), smoothstep(0.30, 0.78, h))
        * recede;
    let rough = clamp(0.85 - h * 0.7, 0.08, 0.95);

    let l = normalize(SUN_DIR);
    let v = -rd;
    let hv = normalize(l + v);

    let diff = max(dot(nb, l), 0.0);
    let gloss = exp2(mix(3.0, 11.0, 1.0 - rough));
    let spec = pow(max(dot(nb, hv), 0.0), gloss) * (1.0 - rough) * 3.0;
    let fres = pow(1.0 - max(dot(nb, v), 0.0), 5.0) * (1.0 - rough);

    // Cool ambient from "above" plus a warm bounce, so the walls have depth
    // even where the key light doesn't reach.
    let ambient = mix(vec3<f32>(0.10, 0.13, 0.20), vec3<f32>(0.30, 0.35, 0.48), nb.y * 0.5 + 0.5);

    var color = albedo * (ambient * recede + diff * vec3<f32>(0.75, 0.85, 1.05));
    color = color + vec3<f32>(0.7, 0.8, 1.0) * spec;
    color = color + vec3<f32>(0.25, 0.4, 0.8) * fres * 0.5;

    // Molten veins running through the walls. The whole scene is cool blue, so
    // these sit opposite it on the wheel and are the only warm thing in frame.
    //
    // Ridged noise: folding the height field at its midline turns smooth blobs
    // into sharp creases, which is what gives filaments rather than patches.
    let ridge = 1.0 - abs(h * 2.0 - 1.0);
    let vein = pow(smoothstep(VEIN_THRESHOLD, 1.0, ridge), 2.5);

    // Energy travelling through the network. The phase runs on beats rather
    // than seconds, so surges arrive with the music instead of drifting past it.
    let beats = u.music.z;
    let sweep = dot(field, normalize(VEIN_FLOW_DIR)) * 5.0 - beats * PI * 2.0 * VEIN_SURGES_PER_BEAT;
    let flow = 0.35 + 0.65 * pow(0.5 + 0.5 * sin(sweep), 3.0);

    // The network is also a spectrum: veins low in the room answer to the bass,
    // veins overhead to the hats, so different parts light with different parts
    // of the mix.
    let band_index = u32(clamp((dir.y * 0.5 + 0.5) * 16.0, 0.0, 15.0));
    let energy = 0.3 + 1.3 * band(band_index);

    // Fine flicker keyed off the height field, so neighbouring filaments don't
    // breathe in lockstep.
    let flicker = 0.85 + 0.15 * sin(h * 28.0 + beats * 3.0);

    // Deep orange body with a hotter core, kept above 1.0 in the brightest
    // filaments so they still read as emissive through the tonemap.
    let body_colour = mix(VEIN_COLOR, VEIN_COLD, u.motion.w);
    let core_colour = mix(VEIN_CORE, VEIN_COLD_CORE, u.motion.w);
    let heat = body_colour * vein + core_colour * pow(vein, 4.0);

    // Dimmer too: cold veins at the old brightness would just be a different
    // colour competing, rather than a background.
    let vein_level = VEIN_INTENSITY * (1.0 - u.motion.w * 0.35);
    color = color + heat * vein_level * flow * flicker * energy * (0.8 + u.audio.w * 0.7);

    // Whatever is left of the room washes out into the white behind it, so the
    // last of it does not linger as a dark ball.
    return mix(color, VOID_COLOUR, smoothstep(0.75, 1.0, u.collapse.x));
}
