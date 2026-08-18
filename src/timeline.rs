//! When things happen.
//!
//! The demo's structure lives here rather than being scattered through the
//! shaders: a section list keyed on the audio clock, producing the handful of
//! values the renderer needs this frame. Sections and camera direction will
//! extend this same table.

use glam::Vec3;

use crate::audio::Sync;

mod cameras;
use cameras::{
    choose_lens_focus, cube_camera, lens_camera, lens_focus_distance, lens_is_visible,
    stabilize_lens_view, tunnel_camera,
};

#[cfg(test)]
use cameras::{
    animated_lens_center, animated_lens_radius, lens_flight_point, lens_visibility_score,
};

/// Cards shown before the scene appears, as (start, end) in seconds.
const CARDS: [(f32, f32); 3] = [(0.4, 2.9), (3.2, 5.5), (5.8, 8.1)];
/// A single attribution under the ferrofluid, indexed after the intro cards.
const CREDITS: [(f32, f32); 1] = [(MERGE_BEATS + 6.0, MERGE_BEATS + 18.0)];
/// The isolated slam after the DnB build's quiet pocket, measured directly in
/// the final Ogg. This—not the earlier downbeat—is the fractal drop.
const FRACTAL_DROP_BEATS: f32 = 105.6;
/// Greetings occupy the now-short, settled fractal section between that drop
/// and the fixed synth-lead signal. They are compact enough that every name
/// clears before the signal cube arrives.
const GREETINGS: [(f32, f32); 7] = [
    (FRACTAL_DROP_BEATS + 1.5, FRACTAL_DROP_BEATS + 4.3),
    (FRACTAL_DROP_BEATS + 4.6, FRACTAL_DROP_BEATS + 7.4),
    (FRACTAL_DROP_BEATS + 7.7, FRACTAL_DROP_BEATS + 10.5),
    (FRACTAL_DROP_BEATS + 10.8, FRACTAL_DROP_BEATS + 13.6),
    (FRACTAL_DROP_BEATS + 13.9, FRACTAL_DROP_BEATS + 16.7),
    (FRACTAL_DROP_BEATS + 17.0, FRACTAL_DROP_BEATS + 19.8),
    (FRACTAL_DROP_BEATS + 20.1, FRACTAL_DROP_BEATS + 22.9),
];
const SIGNAL_WORDS: usize = 14;
const SIGNAL_TEXT_START: f32 = 43.776;
const SIGNAL_TEXT_CADENCE: f32 = 0.514;
const SIGNAL_TEXT_HOLD: f32 = 0.42;
/// Screen-space continuation of the cube's successive X/Y/Z quarter-turns.
/// Every second triplet reverses the apparent sweep as the cube's accumulated
/// orientation presents the opposite faces to the camera.
const SIGNAL_TEXT_THROW: [[f32; 2]; SIGNAL_WORDS] = [
    [0.0, -1.0],
    [1.0, 0.0],
    [-0.72, 0.72],
    [0.0, 1.0],
    [-1.0, 0.0],
    [0.72, -0.72],
    [0.0, -1.0],
    [1.0, 0.0],
    [-0.72, 0.72],
    [0.0, 1.0],
    [-1.0, 0.0],
    [0.72, -0.72],
    [0.0, -1.0],
    [1.0, 0.0],
];
/// Credits sit small, low, and in black — they are a footnote to the scene,
/// not a title over it.
const CREDIT_SCALE: f32 = 0.52;
/// Clip-space corner the credits sit in.
const CREDIT_X: f32 = -0.52;
const CREDIT_Y: f32 = -0.70;

/// Apparent size of each card. The name sits closest to the camera; the rest
/// fall back behind it.
const CARD_SCALES: [f32; 3] = [1.9, 1.0, 1.25];
/// How long a card takes to fade in and out.
const CARD_FADE: f32 = 0.35;
/// When the scene begins to appear, and how long it takes.
const SCENE_START: f32 = 7.7;
const SCENE_FADE: f32 = 2.4;
/// When the ferrofluid takes over, in beats from the start. The titles clear
/// around beat 24; holding until beat 48 gives the liquid blobs a full phrase
/// of their own before the bristles arrive.
const SPIKE_BEATS: f32 = 48.0;
/// How long the change takes. Short: a transition that eases over sixteen
/// beats reads as a fade, not as an event.
const SPIKE_RAMP: f32 = 6.0;

/// The path the camera glides once it is inside, as directions from the
/// centre. Scaled by FRACTAL_INSIDE and then nudged clear of any wall.
const FRACTAL_PATH: [[f32; 3]; 6] = [
    [0.95, 0.25, 0.18],
    [0.35, 0.62, 0.70],
    [-0.45, 0.20, 0.87],
    [-0.90, -0.25, 0.35],
    [-0.30, -0.70, -0.65],
    [0.55, -0.35, -0.76],
];
/// Where along the path the first stand is. Later shots step on from here.
const FRACTAL_STAND: f32 = 0.17;
/// How many places on the path each shot tries before picking one, how far a
/// probe looks, and how much open space is enough to score full marks.
const STAND_CANDIDATES: usize = 12;
const STAND_REACH: f32 = 9.0;
const STAND_ENOUGH: f32 = 5.0;

/// How far along the corridor the camera looks, as a fraction of its length,
/// and how far that aim drifts either side of it. The drift is the whole of
/// the camera's motion in this scene: the view tracks along the string rather
/// than turning on a clock of its own.
///
/// Near the start, because that is the part of the corridor within a few units
/// of the camera — where beads still read as beads. Aimed halfway along, the
/// view points at something twelve units off and the near string, which is the
/// part worth looking at, sits out at the edge of frame.
const FRACTAL_AIM: f32 = 0.08;
/// A short lateral camera slide across the local clear pocket. The strings
/// remain the subject in front of the camera instead of becoming a rail the
/// camera travels down.
const FRACTAL_SIDE_GLIDE: f32 = 0.32;

/// Where the string corridor starts relative to the camera. This is separate
/// from `FRACTAL_FOCUS`, which scores a safe viewing direction farther out:
/// the particle field itself needs to begin in the foreground.
const FRACTAL_STRING_LEAD: f32 = 0.45;
const FRACTAL_FOCUS: f32 = 1.8;
const FRACTAL_OFFSET: f32 = 0.05;
/// How wide the fractal scene shoots. Much wider than the rest of the demo:
/// the camera is standing inside the structure with three strings spread
/// across it, and at the 52 degrees the shot list uses, the outer two are
/// outside the frame before they have gone anywhere.
const FRACTAL_FOV: f32 = 96.0;

/// Clearance the glide keeps from the structure, and how hard the resulting
/// path is smoothed, in seconds. The clearance has to be something the
/// distance field can actually offer — see fractal::MAX_CLEARANCE.
const FRACTAL_CLEARANCE: f32 = 0.14;
const GLIDE_SMOOTHING: f32 = 0.55;

/// How close in the shot settles once it is inside.
const FRACTAL_INSIDE: f32 = 3.8;

/// How much longer shots hold in the fractal scene.
const FRACTAL_SHOT_HOLD: f32 = 2.2;
/// The fractal uses a simple visual rhythm: hold a place for four beats, pan
/// once across it, then cut to the next safe vantage on the following beat.
const FRACTAL_HOLD_BEATS: f32 = 4.0;

/// How much closer the shot drifts over the whole scene, as a fraction.
const FRACTAL_DIVE: f32 = 0.22;

/// Start contracting late enough that the white cover occupies the quiet
/// pocket before the measured drop, rather than swallowing the DnB build.
const COLLAPSE_BEATS: f32 = MERGE_BEATS + 38.0;
const COLLAPSE_RAMP: f32 = 12.0;

/// How far into the collapse the scene changes over, as fractions of it.
///
/// The frame washes to white from WASH_IN, holds solid white from WASH_HOLD to
/// WASH_BACK, then comes back by WASH_OUT. The geometry is swapped inside that
/// hold — at 0.9, which is the threshold scene.wgsl switches on and must stay
/// in step with these — so the one frame where the old scene becomes the
/// fractal is a frame with nothing in it. A plateau rather than a single peak
/// because the swap has to be covered at whatever moment the frame lands on,
/// not only at the instant the curve happens to touch 1.
const WASH_IN: f32 = 0.52;
const WASH_HOLD: f32 = 0.86;
/// A half-beat snap out of white: quick enough to belong to the slam, softened
/// enough not to alias as a single-frame cut.
const FRACTAL_REVEAL_BEATS: f32 = 0.5;
/// Let the new room settle after the white-out before its bead field arrives.
/// Kept in beats so the pause remains musical at any tracker tempo.
const BEAD_REVEAL_DELAY: f32 = 2.0;
const BEAD_REVEAL_BEATS: f32 = 1.5;

/// The fractal gets a full section before one of its holes becomes the first
/// living lens. The handoff itself is eight beats: seal, refract, then cross.
/// Minimal signal-cube interlude after the last greeting.
/// Fixed to the isolated synth lead: its repeated signal enters at 43.776s.
const SIGNAL_BEATS: f32 = 130.0;
const SIGNAL_DURATION_BEATS: f32 = 22.0;
const SIGNAL_TRANSITION_BEATS: f32 = 2.0;
const LENS_BEATS: f32 = SIGNAL_BEATS + SIGNAL_DURATION_BEATS;
const LENS_SEAL_BEATS: f32 = 3.0;
const LENS_TRANSITION_BEATS: f32 = 8.0;
/// One long flight through the lens field before its closed spline repeats.
/// At 125 BPM this is about 46 seconds without a cut or visible reset.
const LENS_FLIGHT_BEATS: f32 = 96.0;
/// Hold one membrane in focus for two bars before pulling to another depth.
const LENS_FOCUS_BEATS: f32 = 8.0;
/// Twelve bars from lens entrance to tunnel. The synth stem empties across
/// beats 180-196, then returns at beat 200 exactly where the tunnel takes over.
const TUNNEL_BEATS: f32 = LENS_BEATS + LENS_TRANSITION_BEATS + 40.0;
const TUNNEL_TRANSITION_BEATS: f32 = 8.0;
const TUNNEL_TENTACLE_BEATS: f32 = 16.0;
/// Three measured drum accents drive the final handoff: the cover launches on
/// the first, fills the frame on the second, and uncovers the sea on the third.
const CUBE_TRANSITION_START_BEATS: f32 = 239.94;
const CUBE_TRANSITION_COVER_BEATS: f32 = 241.44;
const CUBE_TRANSITION_END_BEATS: f32 = 244.02;
/// Motion is already established behind the transition when the sea appears.
const CUBE_GRAVITY_LEAD_BEATS: f32 = 6.0;
const CUBE_GRAVITY_BEATS: f32 = 8.0;
/// Four bars of the complete cube field before it fades into the end cards.
const OUTRO_BEATS: f32 = 275.0;
const OUTRO_FADE_BEATS: f32 = 8.0;
/// Kick and low-tom attacks measured from the isolated drum stem, in seconds
/// into the final cut. Strength keeps secondary hits from launching the field
/// as high as the main kicks; the long 84.1-87.3 gap remains deliberately calm.
const CUBE_DRUM_HITS: [(f32, f32); 21] = [
    (79.98, 1.00),
    (80.48, 1.00),
    (80.80, 0.55),
    (81.34, 1.00),
    (81.86, 0.95),
    (82.12, 1.00),
    (82.72, 0.95),
    (83.24, 1.00),
    (83.56, 0.55),
    (84.10, 1.00),
    (87.32, 0.78),
    (87.62, 0.90),
    (87.94, 0.85),
    (88.46, 0.88),
    (88.96, 0.86),
    (89.50, 0.88),
    (90.02, 0.90),
    (90.28, 0.80),
    (90.78, 0.86),
    (91.14, 0.75),
    (91.62, 0.62),
];
const OUTRO_CARDS: [(f32, f32); 2] = [
    (
        OUTRO_BEATS + OUTRO_FADE_BEATS + 2.0,
        OUTRO_BEATS + OUTRO_FADE_BEATS + 10.0,
    ),
    (
        OUTRO_BEATS + OUTRO_FADE_BEATS + 11.0,
        OUTRO_BEATS + OUTRO_FADE_BEATS + 19.0,
    ),
];
const DEMO_END_BEATS: f32 = OUTRO_BEATS + OUTRO_FADE_BEATS + 21.0;
/// Space between the camera and the most deformed membrane. This is deliberately
/// generous: the safety projection should be a last resort, not something that
/// visibly reshapes the authored spline as the shot begins.
const LENS_CAMERA_CLEARANCE: f32 = 0.32;
const LENS_FOV_DEGREES: f32 = 64.0;
const LENS_CENTERS: [Vec3; 7] = [
    Vec3::new(-2.70, 0.50, 0.40),
    Vec3::new(2.50, -0.80, -0.40),
    Vec3::new(0.00, 2.70, -2.00),
    Vec3::new(-3.80, -1.80, -3.70),
    Vec3::new(4.00, 1.70, -4.20),
    Vec3::new(0.60, -2.90, -5.50),
    Vec3::new(-0.80, 0.50, -8.00),
];
const LENS_RADII: [f32; 7] = [1.72, 1.48, 1.08, 1.68, 1.88, 1.28, 2.45];
const LENS_FLIGHT: [[f32; 3]; 10] = [
    [0.0, 0.1, 5.0],
    [-0.8, 0.2, 3.4],
    [0.2, -0.1, 1.8],
    [0.8, 0.5, 0.5],
    [-0.9, -0.2, -1.5],
    [-0.3, -0.5, -3.0],
    [1.0, -0.4, -4.5],
    [1.8, 1.2, -5.4],
    [0.8, 2.8, -5.0],
    [0.0, 0.0, -2.2],
];

/// How long the room takes to give up the warm accent — much slower than the
/// body takes to claim it.
const PALETTE_RAMP: f32 = 24.0;

/// When the blobs start gathering into one, and how long that takes.
const MERGE_BEATS: f32 = SPIKE_BEATS + 8.0;
const MERGE_RAMP: f32 = 16.0;
/// When it starts turning.
const SPIN_BEATS: f32 = SPIKE_BEATS + 20.0;
/// Turn rate at rest, and how much the music adds on top.
const SPIN_BASE: f32 = 0.22;
const SPIN_FROM_BASS: f32 = 2.5;
const SPIN_FROM_BEAT: f32 = 1.1;
/// The second axis runs slower, so the two never resolve into one tumble.
const TILT_RATIO: f32 = 0.37;

/// How far a card travels while it is on screen, in cap heights.
const SCROLL_RANGE: f32 = 1.5;

/// What the timeline says about this frame.
#[derive(Clone, Copy, Default)]
pub struct Stage {
    /// Which card is showing, or -1 for none.
    pub card: i32,
    /// Card opacity, including its beat pulse.
    pub card_alpha: f32,
    /// How far the scene has faded up, 0 to 1.
    pub scene: f32,
    /// Horizontal drift of the card, in cap heights. Positive is right.
    pub scroll: f32,
    /// How large the card reads, which is how near it feels.
    pub scale: f32,
    /// How far through its life the card is, 0 to 1.
    pub card_progress: f32,
    /// Position of the card in clip space. Applied there rather than in text
    /// space so a card's placement does not move when its size changes.
    pub card_offset: [f32; 2],
    /// How far the blob has turned into a ferrofluid, 0 to 1.
    pub spike: f32,
    /// Transition mask threshold: 0 is fully the old scene, 1 the new one.
    pub dissolve: f32,
    /// How far the blobs have gathered into a single body, 0 to 1.
    pub merge: f32,
    /// How far the body has bled out into the water, 0 to 1. It leads the
    /// collapse slightly, so the ink is already in the water when the room
    /// goes — what scene three is made of came out of the blob.
    pub bleed: f32,
    /// How far into the fractal the shot has drifted, 0 to 1.
    pub dive: f32,
    /// How far the room has collapsed, 0 to 1. Past 1 the shell has shrunk
    /// through the camera and there is nothing but white behind it.
    pub collapse: f32,
    /// How white the frame is washed over the scene change: 0 either side, 1
    /// at the moment the geometry is swapped.
    pub wash: f32,
    /// How far the fractal's bead strings have arrived, 0 to 1. Behind the
    /// wash, so they appear with the scene they belong to.
    pub beads: f32,
    /// Fractal-to-signal-cube transition and beats elapsed within that scene.
    pub signal_field: f32,
    pub signal_travel: f32,
    /// One fractal aperture sealing into a membrane.
    pub lens_seal: f32,
    /// Passage through that membrane, including the hidden camera handoff.
    pub lens_cross: f32,
    /// How completely the living-lens field has replaced the fractal.
    pub lens_field: f32,
    /// Bead strings releasing into free satellites around the lenses.
    pub lens_particles: f32,
    /// Lens membrane flooding the frame and stretching into a tunnel mouth.
    pub tunnel_cross: f32,
    /// How completely the tunnel has replaced the lens field.
    pub tunnel_field: f32,
    /// Staggered growth of the wall-to-wall liquid-metal tentacles.
    pub tunnel_tentacles: f32,
    /// Beats since the camera began its forward tunnel march.
    pub tunnel_travel: f32,
    /// Tunnel closing into, then opening out of, a reflective cube.
    pub cube_cross: f32,
    /// How completely the gravitational cube sea has replaced the tunnel.
    pub cube_field: f32,
    /// Strength of the cubes' ballistic motion.
    pub cube_gravity: f32,
    /// Beats elapsed since arriving in the cube sea.
    pub cube_travel: f32,
    /// Strength of and seconds since the latest measured drum hit in the cube
    /// section. The shader turns this pair into a travelling crest and flash.
    pub cube_drum_strength: f32,
    pub cube_drum_phase: f32,
    /// Cube field disappearing into the outro black.
    pub outro_fade: f32,
    /// Dim peach beam retained behind the outro cards.
    pub outro_beam: f32,
    /// Final fade including grain and every post effect; held before exit.
    pub final_black: f32,
    /// The room's palette shift, on its own slower ramp than the body's. A
    /// background caught changing draws attention to itself; the point is that
    /// it has receded, not that it receded just now.
    pub palette: f32,
    /// How far the body has wound up, 0 to 1. The angles themselves live in
    /// `Spin`, since a rate that follows the music has to be integrated.
    pub winding: f32,
}

impl Stage {
    pub fn at(music: &Sync) -> Self {
        let t = music.time;
        let beats = music.beat_phase;

        let mut card = -1;
        let mut card_alpha = 0.0;
        let mut scroll = 0.0;
        let mut scale = 1.0;
        let mut card_progress = 0.0;
        let mut card_offset = [0.0, 0.0];
        for (index, (start, end)) in CARDS.iter().enumerate() {
            if t >= *start && t < *end {
                card = index as i32;
                // Fade in, hold, fade out.
                let in_edge = smoothstep(*start, start + CARD_FADE, t);
                let out_edge = 1.0 - smoothstep(end - CARD_FADE, *end, t);
                card_alpha = in_edge * out_edge;

                // Drifts right to left across its life, slowly.
                let progress = (t - start) / (end - start);
                scroll = SCROLL_RANGE * (0.5 - progress);

                // Drifts very slightly toward the camera while it is up.
                scale = CARD_SCALES[index] * (1.0 + progress * 0.06);
                card_progress = progress;
            }
        }

        // Credits run on the same machinery, just lower, smaller and dark.
        for (index, (start, end)) in CREDITS.iter().enumerate() {
            if beats >= *start && beats < *end {
                card = (CARDS.len() + index) as i32;

                let fade = 1.5;
                let in_edge = smoothstep(*start, start + fade, beats);
                let out_edge = 1.0 - smoothstep(end - fade, *end, beats);
                card_alpha = in_edge * out_edge;

                let progress = (beats - start) / (end - start);
                card_progress = progress;
                scroll = SCROLL_RANGE * (0.5 - progress);
                scale = CREDIT_SCALE;
                card_offset = [CREDIT_X, CREDIT_Y];
            }
        }

        // Names appear one at a time as quiet inscriptions over the fractal.
        for (index, (start, end)) in GREETINGS.iter().enumerate() {
            if beats >= *start && beats < *end {
                card = (CARDS.len() + CREDITS.len() + index) as i32;
                let fade = 0.75;
                card_alpha = smoothstep(*start, start + fade, beats)
                    * (1.0 - smoothstep(end - fade, *end, beats));
                let progress = (beats - start) / (end - start);
                card_progress = progress;
                scroll = SCROLL_RANGE * 0.55 * (0.5 - progress);
                scale = CREDIT_SCALE * 1.18;
                card_offset = [CREDIT_X, CREDIT_Y];
            }
        }

        // One word on every measured synth-lead beep. These are intentionally
        // timed in seconds: the motif is ~0.514s, not a division of 180 BPM.
        for (index, direction) in SIGNAL_TEXT_THROW.iter().enumerate() {
            let start = SIGNAL_TEXT_START + index as f32 * SIGNAL_TEXT_CADENCE;
            let end = start + SIGNAL_TEXT_HOLD;
            if t >= start && t < end {
                card = (CARDS.len() + CREDITS.len() + GREETINGS.len() + index) as i32;
                let progress = (t - start) / SIGNAL_TEXT_HOLD;
                card_alpha =
                    smoothstep(0.0, 0.08, progress) * (1.0 - smoothstep(0.58, 1.0, progress));
                card_progress = progress;
                scroll = 0.0;
                // Sit precisely over the hero during its fast quarter-turn,
                // then inherit that turn's screen-space momentum and leave
                // the frame. Ease-in keeps the word/cube lock crisp at onset.
                let departure = smoothstep(0.20, 0.92, progress).powi(2);
                card_offset = [
                    direction[0] * departure * 1.32,
                    direction[1] * departure * 1.18,
                ];
                scale = (2.25 + index.saturating_sub(11) as f32 * 0.28) * (1.0 + departure * 0.10);
            }
        }

        // Return to the opening's centered, bright card treatment at the end.
        for (index, (start, end)) in OUTRO_CARDS.iter().enumerate() {
            if beats >= *start && beats < *end {
                card =
                    (CARDS.len() + CREDITS.len() + GREETINGS.len() + SIGNAL_WORDS + index) as i32;
                let fade = 1.0;
                card_alpha = smoothstep(*start, start + fade, beats)
                    * (1.0 - smoothstep(end - fade, *end, beats));
                let progress = (beats - start) / (end - start);
                card_progress = progress;
                scroll = SCROLL_RANGE * (0.5 - progress);
                scale = [1.9, 1.25][index] * (1.0 + progress * 0.06);
                card_offset = [0.0, 0.0];
            }
        }

        // Cards breathe on the beat rather than sitting flat.
        card_alpha *= 0.82 + music.beat * 0.35;

        // Scene changes, in beats from the start of the tune. The blob spends
        // the opening as a fluid, then bristles.
        let spike = smoothstep(SPIKE_BEATS, SPIKE_BEATS + SPIKE_RAMP, beats);

        // The wipe leads the change slightly, so the new scene is revealed
        // rather than appearing and then being wiped to.
        let dissolve = smoothstep(SPIKE_BEATS - 2.0, SPIKE_BEATS + SPIKE_RAMP * 0.7, beats);

        let merge = smoothstep(MERGE_BEATS, MERGE_BEATS + MERGE_RAMP, beats);
        let palette = smoothstep(MERGE_BEATS, MERGE_BEATS + PALETTE_RAMP, beats);
        let collapse = smoothstep(COLLAPSE_BEATS, COLLAPSE_BEATS + COLLAPSE_RAMP, beats);
        let bleed = smoothstep(COLLAPSE_BEATS - 6.0, COLLAPSE_BEATS + 2.0, beats);

        // The room going white over the change, and coming back on the far
        // side. Full white exactly at SWAP, which is where scene.wgsl trades
        // the old geometry for the fractal — so the swap itself lands in the
        // one frame where nothing can be made out.
        let wash = smoothstep(WASH_IN, WASH_HOLD, collapse).min(
            1.0 - smoothstep(
                FRACTAL_DROP_BEATS,
                FRACTAL_DROP_BEATS + FRACTAL_REVEAL_BEATS,
                beats,
            ),
        );

        // Do not let the beads leak into the outgoing scene. The new room
        // first gets a clean beat to establish itself, then the shader reveals
        // the individual strings progressively from their leading ends.
        let bead_start = COLLAPSE_BEATS + COLLAPSE_RAMP + BEAD_REVEAL_DELAY;
        let beads = smoothstep(bead_start, bead_start + BEAD_REVEAL_BEATS, beats);
        let signal_field = smoothstep(SIGNAL_BEATS, SIGNAL_BEATS + SIGNAL_TRANSITION_BEATS, beats);
        let signal_travel = (beats - SIGNAL_BEATS).max(0.0);

        let lens_seal = smoothstep(LENS_BEATS, LENS_BEATS + LENS_SEAL_BEATS, beats);
        let lens_cross = smoothstep(
            LENS_BEATS + LENS_SEAL_BEATS,
            LENS_BEATS + LENS_TRANSITION_BEATS,
            beats,
        );
        let lens_field = smoothstep(
            LENS_BEATS + LENS_SEAL_BEATS + 1.0,
            LENS_BEATS + LENS_TRANSITION_BEATS,
            beats,
        );
        let lens_particles = smoothstep(
            LENS_BEATS + LENS_SEAL_BEATS + (LENS_TRANSITION_BEATS - LENS_SEAL_BEATS) * 0.62,
            LENS_BEATS + LENS_TRANSITION_BEATS + 2.0,
            beats,
        );
        let tunnel_cross = smoothstep(TUNNEL_BEATS, TUNNEL_BEATS + TUNNEL_TRANSITION_BEATS, beats);
        let tunnel_field = smoothstep(
            TUNNEL_BEATS + TUNNEL_TRANSITION_BEATS * 0.45,
            TUNNEL_BEATS + TUNNEL_TRANSITION_BEATS,
            beats,
        );
        let tunnel_tentacles = smoothstep(
            TUNNEL_BEATS + TUNNEL_TRANSITION_BEATS,
            TUNNEL_BEATS + TUNNEL_TRANSITION_BEATS + TUNNEL_TENTACLE_BEATS,
            beats,
        );
        let tunnel_travel = (beats - TUNNEL_BEATS - TUNNEL_TRANSITION_BEATS).max(0.0);
        let cube_cross = if beats < CUBE_TRANSITION_COVER_BEATS {
            smoothstep(
                CUBE_TRANSITION_START_BEATS,
                CUBE_TRANSITION_COVER_BEATS,
                beats,
            ) * 0.5
        } else {
            0.5 + smoothstep(
                CUBE_TRANSITION_COVER_BEATS,
                CUBE_TRANSITION_END_BEATS,
                beats,
            ) * 0.5
        };
        let cube_field = smoothstep(
            CUBE_TRANSITION_COVER_BEATS,
            CUBE_TRANSITION_END_BEATS,
            beats,
        );
        let cube_gravity = smoothstep(
            CUBE_TRANSITION_START_BEATS - CUBE_GRAVITY_LEAD_BEATS,
            CUBE_TRANSITION_START_BEATS - CUBE_GRAVITY_LEAD_BEATS + CUBE_GRAVITY_BEATS,
            beats,
        );
        // Run the phase clock alongside the gravity ramp, not from arrival.
        // The sea is therefore already moving when the cover opens onto it.
        let cube_travel =
            (beats - (CUBE_TRANSITION_START_BEATS - CUBE_GRAVITY_LEAD_BEATS)).max(0.0);
        let (cube_drum_strength, cube_drum_phase) = cube_drum_sync(t);
        let outro_fade = smoothstep(OUTRO_BEATS, OUTRO_BEATS + OUTRO_FADE_BEATS, beats);
        let outro_beam = smoothstep(OUTRO_BEATS + 1.0, OUTRO_BEATS + OUTRO_FADE_BEATS, beats)
            * (1.0 - smoothstep(DEMO_END_BEATS - 2.0, DEMO_END_BEATS, beats));
        let final_black = smoothstep(DEMO_END_BEATS - 2.0, DEMO_END_BEATS - 1.0, beats);

        Self {
            card,
            card_alpha,
            merge,
            palette,
            collapse,
            wash,
            beads,
            signal_field,
            signal_travel,
            lens_seal,
            lens_cross,
            lens_field,
            lens_particles,
            tunnel_cross,
            tunnel_field,
            tunnel_tentacles,
            tunnel_travel,
            cube_cross,
            cube_field,
            cube_gravity,
            cube_travel,
            cube_drum_strength,
            cube_drum_phase,
            outro_fade,
            outro_beam,
            final_black,
            bleed,
            dive: smoothstep(COLLAPSE_BEATS, COLLAPSE_BEATS + 160.0, beats),
            winding: smoothstep(SPIN_BEATS, SPIN_BEATS + 12.0, beats),
            spike,
            dissolve,
            scene: smoothstep(SCENE_START, SCENE_START + SCENE_FADE, t),
            scroll,
            scale,
            card_progress,
            card_offset,
        }
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn cube_drum_sync(time: f32) -> (f32, f32) {
    let mut strength = 0.0;
    let mut phase = 1.0;
    for (hit, hit_strength) in CUBE_DRUM_HITS {
        if time < hit {
            break;
        }
        strength = hit_strength;
        phase = time - hit;
    }
    // A drum crest is finished before the next normal kick interval. In the
    // long stem break this deliberately returns to the unaccented sea wave.
    if phase > 0.62 {
        strength = 0.0;
    }
    (strength, phase)
}

/// True after the final card and its remaining beam have faded completely.
pub fn is_finished(music: &Sync) -> bool {
    music.beat_phase >= DEMO_END_BEATS
}

/// Master output gain. The tune recedes across the whole visual outro, reaches
/// silence before the last black frame, and stays silent through the hold.
pub fn audio_gain(music: &Sync) -> f32 {
    1.0 - smoothstep(OUTRO_BEATS, DEMO_END_BEATS - 4.0, music.beat_phase)
}

// --- camera --------------------------------------------------------------
//
// One continuous orbit reads as a screensaver however good the shading is.
// Demos cut. Shots are measured in beats rather than seconds so every cut
// lands on the music, and the phase they are counted from is the one the
// onset detector locked to the bass.

/// Where the camera is and what it is looking at.
#[derive(Clone, Copy)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    /// Camera up, allowing the lens flight to bank around its viewing axis.
    pub up: Vec3,
    pub fov_degrees: f32,
    /// Distance to the visible subject surface, not merely its look-at point.
    pub focus_distance: f32,
}

/// A single camera setup. Its motion can take longer than the edit holds it:
/// that lets the cut rhythm tighten without making every pan and dolly race.
struct Shot {
    /// Full duration of this setup's camera move.
    beats: f32,
    /// How long the edit holds it before becoming willing to cut.
    cut_beats: f32,
    kind: Kind,
}

enum Kind {
    /// A dolly. Radius, height and lens all ease together, and the camera
    /// arcs while it moves — a move straight along the view axis reads as a
    /// zoom rather than as a camera going somewhere.
    Dolly {
        azimuth: f32,
        sweep: f32,
        from: f32,
        to: f32,
        from_height: f32,
        to_height: f32,
        from_fov: f32,
        to_fov: f32,
        /// 0 eases symmetrically. Above 1 front-loads the move: it leaves fast
        /// and settles slowly, which is what makes a zoom feel like it has
        /// weight rather than being animated at a constant rate.
        snap: f32,
    },
    /// Tight and low, drifting around the surface.
    Close {
        azimuth: f32,
        radius: f32,
        height: f32,
        sweep: f32,
    },
    /// Wide, circling. The one shot that shows the room.
    Orbit {
        azimuth: f32,
        radius: f32,
        height: f32,
        sweep: f32,
    },
    /// Follows a closed Catmull-Rom spline through control points around the
    /// blob. A circle is predictable; a spline swoops — the radius and height
    /// vary continuously, so the move reads as flown rather than orbited.
    Spline {
        points: &'static [[f32; 3]],
        fov: f32,
    },
    /// Climbs while circling, looking down as it goes.
    Rise {
        azimuth: f32,
        radius: f32,
        from_height: f32,
        to_height: f32,
    },
}

/// Control points for the spline shot: a loop that dives in close, swings
/// wide and high, then drops under the blob.
const FLIGHT: [[f32; 3]; 6] = [
    [3.4, 0.9, 0.6],
    [1.6, -0.5, 2.2],
    [-1.4, 0.3, 3.2],
    [-3.6, 1.7, -0.4],
    [-1.2, 0.6, -2.8],
    [1.9, -0.9, -2.1],
];

/// The cut list. Total length is one loop; it repeats.
const SHOTS: [Shot; 7] = [
    Shot {
        beats: 12.0,
        cut_beats: 6.0,
        kind: Kind::Dolly {
            azimuth: 0.6,
            sweep: 0.35,
            from: 5.2,
            to: 3.0,
            from_height: 0.5,
            to_height: 0.75,
            from_fov: 55.0,
            to_fov: 52.0,
            snap: 2.6,
        },
    },
    Shot {
        beats: 8.0,
        cut_beats: 4.0,
        kind: Kind::Close {
            azimuth: 2.3,
            radius: 1.95,
            height: -0.35,
            sweep: 0.5,
        },
    },
    Shot {
        beats: 16.0,
        cut_beats: 8.0,
        kind: Kind::Spline {
            points: &FLIGHT,
            fov: 56.0,
        },
    },
    Shot {
        beats: 8.0,
        cut_beats: 4.0,
        kind: Kind::Rise {
            azimuth: 1.2,
            radius: 2.6,
            from_height: -1.2,
            to_height: 1.6,
        },
    },
    Shot {
        beats: 8.0,
        cut_beats: 4.0,
        kind: Kind::Close {
            azimuth: 5.1,
            radius: 1.8,
            height: 0.6,
            sweep: -0.4,
        },
    },
    // A fast whip round: short enough that it reads as a single gesture.
    Shot {
        beats: 6.0,
        cut_beats: 3.0,
        kind: Kind::Orbit {
            azimuth: 2.9,
            radius: 2.7,
            height: -0.15,
            sweep: 1.7,
        },
    },
    // Arcs up and out, widening as it goes, and lands on the first shot's
    // starting framing so the loop closes without a jolt.
    Shot {
        beats: 12.0,
        cut_beats: 6.0,
        kind: Kind::Dolly {
            azimuth: 3.4,
            sweep: 1.5,
            from: 2.4,
            to: 5.2,
            from_height: -0.4,
            to_height: 0.5,
            from_fov: 46.0,
            to_fov: 58.0,
            snap: 1.9,
        },
    },
];

/// Spherical coordinates to a position, with the blob at the origin.
fn orbit_position(azimuth: f32, radius: f32, height: f32) -> Vec3 {
    Vec3::new(azimuth.cos() * radius, height, azimuth.sin() * radius)
}

/// Runs the cut list.
///
/// A shot's beat count is when it becomes *willing* to cut, not when it does.
/// The cut then waits for the next accent, so it lands on something the
/// arrangement plays rather than on a count that happens to be running.
pub struct Director {
    shot: usize,
    /// Beat position this shot began at.
    started: f32,
    /// Once the room has taken over, use its own beat-grid cut cadence instead
    /// of inheriting the elaborate opening-shot timing.
    fractal_mode: bool,
    /// Smoothed camera position for the fractal glide. The raw position is
    /// corrected against the structure every frame, and those corrections are
    /// not continuous — filtering them is what keeps the glide smooth.
    glide: Vec3,
    glide_started: bool,
    /// Low-pass filtered lens framing. The lenses can surround the eye, where
    /// a spatial average alone becomes ill-conditioned and flips direction.
    lens_forward: Vec3,
    lens_view_started: bool,
    /// Identity is held for the whole phrase so deformation and changing
    /// visibility cannot trigger unscheduled autofocus changes.
    lens_focus_phrase: usize,
    lens_focus_index: usize,
    /// Where the camera is along the path, and at what radius, so the beads
    /// can be strung along the same corridor rather than their own.
    pub along: f32,
    pub radius: f32,
    /// Which shot the glide was last placed for, so a cut can move it.
    placed_for: usize,
    /// And which the corridors were last traced for. Separate from the glide's,
    /// because the glide updates first and would otherwise clear the flag
    /// before the trace ever saw it.
    traced_for: usize,
    /// The corridors the bead strings run along, retraced on each cut.
    pub bundle: [crate::fractal::Corridor; crate::fractal::STRINGS],
}

/// How much closer the second scene frames, and how far each of its shots
/// creeps in over its life.
const OCTOPUS_FRAMING: f32 = 0.78;
const OCTOPUS_CREEP: f32 = 0.12;

/// How long past its length a shot waits for an accent before cutting anyway.
const CUT_GRACE: f32 = 1.5;

impl Default for Director {
    fn default() -> Self {
        Self {
            shot: 0,
            started: 0.0,
            fractal_mode: false,
            glide: Vec3::ZERO,
            glide_started: false,
            lens_forward: Vec3::ZERO,
            lens_view_started: false,
            lens_focus_phrase: usize::MAX,
            lens_focus_index: 0,
            along: 0.0,
            radius: FRACTAL_INSIDE,
            placed_for: usize::MAX,
            traced_for: usize::MAX,
            bundle: std::array::from_fn(|_| crate::fractal::Corridor::default()),
        }
    }
}

impl Director {
    pub fn update(&mut self, music: &Sync, stage: &Stage) -> Camera {
        let fractal_active = stage.collapse > 0.85 && stage.lens_field < 1.0;
        if fractal_active && !self.fractal_mode {
            self.fractal_mode = true;
            // Anchor the first held shot to the musical grid. Subsequent cuts
            // use the same grid rather than a wall-clock duration.
            self.started = music.beat_phase.floor();
        }

        let length = if fractal_active {
            FRACTAL_HOLD_BEATS
        } else {
            SHOTS[self.shot].cut_beats * (1.0 + stage.collapse * FRACTAL_SHOT_HOLD)
        };
        let elapsed = music.beat_phase - self.started;

        // The opening follows accents; the fractal is intentionally simpler:
        // every move is a hard cut on the next beat after its four-beat hold.
        let cut = if fractal_active {
            elapsed >= length && music.beat_phase.floor() > self.started.floor()
        } else {
            elapsed >= length && (music.hard_hit || elapsed >= length + CUT_GRACE)
        };
        if cut {
            self.shot = (self.shot + 1) % SHOTS.len();
            self.started = music.beat_phase;
        }

        let shot = &SHOTS[self.shot];
        // Clamped, so a shot held past its length holds its final framing
        // rather than running off the end of the move.
        let t = ((music.beat_phase - self.started) / shot.beats).clamp(0.0, 1.0);
        let mut camera = Camera::compose(shot, t, music, stage.merge, stage.collapse);

        // The fractal is metres across, so the shot list's radii would put the
        // camera in a wall. Frame the whole structure first, then work inward
        // to a vantage point found by asking the distance estimator where the
        // open pockets are.
        if stage.collapse > 0.0 && stage.lens_field < 1.0 {
            // The camera stands still and pans. Flying it through the
            // structure fought the structure: the eye cannot follow a shape
            // this dense while the viewpoint is also moving, and every
            // correction against a wall showed up as a lurch.
            //
            // It does move, but by cutting: each shot stands somewhere else on
            // the path, so a long scene is a series of held views rather than
            // one.
            //
            // Which somewhere is chosen rather than stepped to. Walking the
            // path by a fixed offset and clearing whatever it landed on is
            // blind — nothing asks whether the resulting view has anywhere for
            // the strings to run, so some shots opened on a wall a few
            // centimetres away and some in the middle of a void with the
            // architecture out of reach. That is what made the scene hit and
            // miss from cut to cut.
            //
            // Safe to make a discrete pick here, unlike the per-frame
            // correction: this is evaluated once per shot, and a shot begins
            // with a cut, so the camera moving to a different candidate is the
            // cut rather than a jump during one.
            let radius = FRACTAL_INSIDE
                * (1.0 - stage.dive * FRACTAL_DIVE)
                * (0.9 + (self.shot % 3) as f32 * 0.1);

            let mut best = (f32::MIN, 0.0, Vec3::ZERO);
            for candidate in 0..STAND_CANDIDATES {
                let along = (FRACTAL_STAND
                    + (self.shot * STAND_CANDIDATES + candidate) as f32 * 0.137)
                    .rem_euclid(1.0);
                let stand = crate::fractal::push_clear(
                    spline(&FRACTAL_PATH, along) * radius,
                    FRACTAL_CLEARANCE,
                );
                let score = stand_score(stand, self.shot);
                if score > best.0 {
                    best = (score, along, stand);
                }
            }
            let (_, along, corrected) = best;

            self.along = along;
            self.radius = radius;

            // Even that moves in steps when the surface underneath changes, so
            // it is low-passed. The filter is on position, not velocity, so it
            // cannot introduce overshoot.
            // A cut moves the camera; it does not slide there, or the move
            // becomes exactly the flight this scene is trying not to have.
            if !self.glide_started || self.placed_for != self.shot {
                self.glide = corrected;
                self.glide_started = true;
                self.placed_for = self.shot;
            }
            let alpha = 1.0 - (-music.dt / GLIDE_SMOOTHING).exp();
            self.glide += (corrected - self.glide) * alpha;
            let inside = self.glide;

            // The corridor is laid out first and the camera is aimed at it
            // afterwards. The other way round — pan the camera, then trace
            // through wherever it happens to point — is what had the string
            // wandering out of frame: the pan ran on a clock of its own while
            // the corridor was fixed for the whole shot, so the two drifted
            // apart within seconds of every cut.
            //
            // A fixed facing per shot, so the trace does not move under the
            // beads. The same one the stand was scored on, or the camera would
            // be judged on a view it does not end up taking.
            let forward = stand_forward(self.glide, self.shot);
            let across = forward.cross(Vec3::Y).normalize_or_zero();
            let above = across.cross(forward).normalize_or_zero();
            let heading = string_heading(forward, across, above);

            // Started off to one side and in front, then cleared — a start
            // buried in the structure puts the head of the string inside a
            // wall, where every bead on it is culled.
            let origin = crate::fractal::push_clear(
                self.glide + forward * FRACTAL_STRING_LEAD - across * FRACTAL_OFFSET,
                FRACTAL_CLEARANCE,
            );

            // Traced once per shot, not per frame. The trace steers on the
            // distance field, so a slightly different start finds a slightly
            // different route — retracing every frame redrew the corridors
            // underneath the beads, which is what had them blinking and
            // jumping about.
            if self.traced_for != self.shot {
                self.traced_for = self.shot;
                crate::fractal::trace_bundle(origin, heading, across, above, &mut self.bundle);

                if std::env::var("KR_DEBUG").is_ok() {
                    for (i, corridor) in self.bundle.iter().enumerate() {
                        let clear = corridor
                            .points
                            .iter()
                            .filter(|point| {
                                crate::fractal::distance(**point)
                                    > crate::fractal::TRACK_CLEARANCE * 0.5
                            })
                            .count();
                        log::info!(
                            "string {i}: {clear}/{} points clear, start {:.2?}",
                            crate::fractal::TRACK_POINTS,
                            corridor.points[0],
                        );
                    }
                }
            }

            // Not find_vantage: choosing the best of a set of candidates is a
            // discrete pick, and as the path advances the winner flips from one
            // to another and the camera jumps. Only the continuous correction
            // is used here.
            let arrival = smoothstep(0.85, 1.0, stage.collapse);
            camera.eye = camera.eye.lerp(inside, arrival);
            camera.fov_degrees += (FRACTAL_FOV - camera.fov_degrees) * arrival;

            // Track sideways across the strings, maintaining a fixed look at
            // the field. The clearance correction keeps this short slide in
            // the local hole rather than allowing the camera into a wall.
            let side = if fractal_active {
                let direction = if self.shot.is_multiple_of(2) {
                    1.0
                } else {
                    -1.0
                };
                ((elapsed / FRACTAL_HOLD_BEATS).clamp(0.0, 1.0) - 0.5)
                    * 2.0
                    * FRACTAL_SIDE_GLIDE
                    * direction
            } else {
                0.0
            };
            let side_eye = crate::fractal::push_clear(inside + across * side, FRACTAL_CLEARANCE);
            let aim = self.corridor_point(FRACTAL_AIM);
            camera.eye = camera.eye.lerp(side_eye, arrival);
            camera.target = camera.target.lerp(aim, arrival);
            camera.focus_distance = camera.eye.distance(camera.target);
        }

        // The fractal dissolves into a brutally static signal shot. Keeping
        // this camera fixed lets the quantized cube rotation own the motion.
        if stage.signal_field > 0.0 {
            let rack = (smoothstep(6.5, 7.5, stage.signal_travel)
                * (1.0 - smoothstep(9.0, 10.0, stage.signal_travel)))
            .max(
                smoothstep(15.0, 16.0, stage.signal_travel)
                    * (1.0 - smoothstep(17.5, 18.5, stage.signal_travel)),
            ) * (1.0 - music.beat * 0.85);
            let signal = Camera {
                eye: Vec3::new(3.25, 2.15, 4.35),
                target: Vec3::ZERO,
                up: Vec3::Y,
                fov_degrees: 48.0,
                // Rack toward the rotating room, then pull straight back to
                // the hero whenever a synth/drum snap lands.
                focus_distance: 4.82 + rack * 2.2,
            };
            if stage.signal_field >= 0.5 {
                camera = signal;
            }
        }

        // The expanding membrane covers this handoff. By the time it clears,
        // the camera is already making the lens field's restrained side glide.
        if stage.lens_cross > 0.0 {
            let mut lens = lens_camera(music);
            let desired = (lens.target - lens.eye).normalize_or_zero();
            if !self.lens_view_started {
                self.lens_forward = desired;
                self.lens_view_started = true;
            } else {
                self.lens_forward =
                    stabilize_lens_view(lens.eye, self.lens_forward, desired, music);
            }
            lens.target = lens.eye + self.lens_forward * 4.0;
            let since = (music.beat_phase - LENS_BEATS - LENS_TRANSITION_BEATS).max(0.0);
            let focus_phrase = (since / LENS_FOCUS_BEATS).floor() as usize;
            if focus_phrase != self.lens_focus_phrase
                || !lens_is_visible(lens.eye, self.lens_forward, self.lens_focus_index, music)
            {
                self.lens_focus_index =
                    choose_lens_focus(lens.eye, self.lens_forward, music, focus_phrase);
                self.lens_focus_phrase = focus_phrase;
            }
            lens.focus_distance = lens_focus_distance(lens.eye, self.lens_focus_index);
            // The membrane is opaque around the midpoint, so make the camera
            // handoff there. Blending it throughout the visible wipe made the
            // outgoing fractal slide and warp before it was covered.
            let handoff = smoothstep(0.46, 0.54, stage.lens_cross);
            camera.eye = camera.eye.lerp(lens.eye, handoff);
            camera.target = camera.target.lerp(lens.target, handoff);
            camera.up = camera.up.lerp(lens.up, handoff).normalize_or_zero();
            camera.fov_degrees += (lens.fov_degrees - camera.fov_degrees) * handoff;
            camera.focus_distance += (lens.focus_distance - camera.focus_distance) * handoff;
        }
        if stage.tunnel_cross > 0.0 {
            let tunnel = tunnel_camera(music);
            // The liquid membrane is fully opaque across this narrow handoff,
            // so the lens flight never visibly interpolates into the tunnel.
            let handoff = smoothstep(0.46, 0.54, stage.tunnel_cross);
            camera.eye = camera.eye.lerp(tunnel.eye, handoff);
            camera.target = camera.target.lerp(tunnel.target, handoff);
            camera.up = camera.up.lerp(tunnel.up, handoff).normalize_or_zero();
            camera.fov_degrees += (tunnel.fov_degrees - camera.fov_degrees) * handoff;
            camera.focus_distance += (tunnel.focus_distance - camera.focus_distance) * handoff;
        }
        if stage.cube_cross > 0.0 {
            let cubes = cube_camera(music);
            let handoff = smoothstep(0.46, 0.54, stage.cube_cross);
            camera.eye = camera.eye.lerp(cubes.eye, handoff);
            camera.target = camera.target.lerp(cubes.target, handoff);
            camera.up = camera.up.lerp(cubes.up, handoff).normalize_or_zero();
            camera.fov_degrees += (cubes.fov_degrees - camera.fov_degrees) * handoff;
            camera.focus_distance += (cubes.focus_distance - camera.focus_distance) * handoff;
        }
        camera
    }

    /// Where the whole bundle is, at a fraction along its length: the mean of
    /// the strings' corridors there.
    ///
    /// The middle string alone is not enough. The traces are steered by the
    /// structure and diverge by however much the architecture demands, so the
    /// middle one can end up at the edge of the group rather than in it —
    /// holding it in frame then pushes the other two out. The centroid moves
    /// with wherever the strings actually went.
    fn corridor_point(&self, t: f32) -> Vec3 {
        let last = crate::fractal::TRACK_POINTS - 1;
        let scaled = t.clamp(0.0, 1.0) * last as f32;
        let index = (scaled.floor() as usize).min(last - 1);
        let f = scaled.fract();

        let sum: Vec3 = self
            .bundle
            .iter()
            .map(|c| c.points[index].lerp(c.points[index + 1], f))
            .sum();
        sum / crate::fractal::STRINGS as f32
    }
}

/// Which way the camera looks from a given stand: roughly towards the middle
/// of the structure, turned a little per shot so successive stands do not all
/// look the same way.
fn stand_forward(stand: Vec3, shot: usize) -> Vec3 {
    let turn = shot as f32 * 1.31;
    let facing = (Vec3::ZERO - stand).normalize_or_zero();
    Vec3::new(
        facing.x * turn.cos() - facing.z * turn.sin(),
        facing.y,
        facing.x * turn.sin() + facing.z * turn.cos(),
    )
    .normalize_or_zero()
}

/// The strings' heading: mostly away from the camera, angled across the frame
/// so a string reads as travelling through the structure rather than as a dot
/// coming towards the lens.
fn string_heading(forward: Vec3, across: Vec3, above: Vec3) -> Vec3 {
    (forward + across * 0.55 - above * 0.18).normalize_or_zero()
}

/// How good a vantage point is: how far the view runs before it meets a
/// surface, how far the strings will get along their heading, and how much
/// room there is at the stand itself.
///
/// The two runs are capped. Past a few units more open space adds nothing —
/// the structure has already receded past where detail reads — and without the
/// cap the score just picks the largest void on the path every time, which is
/// a view of nothing in particular.
fn stand_score(stand: Vec3, shot: usize) -> f32 {
    let forward = stand_forward(stand, shot);
    let across = forward.cross(Vec3::Y).normalize_or_zero();
    let above = across.cross(forward).normalize_or_zero();
    let heading = string_heading(forward, across, above);

    let view = crate::fractal::free_run(stand, forward, STAND_REACH);
    let run = crate::fractal::free_run(stand + forward * FRACTAL_FOCUS, heading, STAND_REACH);

    view.min(STAND_ENOUGH) + run.min(STAND_ENOUGH) + crate::fractal::distance(stand) * 8.0
}

impl Camera {
    fn compose(shot: &Shot, t: f32, music: &Sync, octopus: f32, collapse: f32) -> Self {
        let mut fov = 52.0;
        let mut target = Vec3::ZERO;

        let eye = match shot.kind {
            Kind::Dolly {
                azimuth,
                sweep,
                from,
                to,
                from_height,
                to_height,
                from_fov,
                to_fov,
                snap,
            } => {
                let e = if snap > 0.0 {
                    ease_out(t, snap)
                } else {
                    ease(t)
                };
                fov = from_fov + (to_fov - from_fov) * e;
                let radius = from + (to - from) * e;
                let height = from_height + (to_height - from_height) * e;
                // The arc is linear in t while the dolly eases, so the camera
                // keeps moving laterally even where the push has settled.
                orbit_position(azimuth + t * sweep, radius, height)
            }
            Kind::Close {
                azimuth,
                radius,
                height,
                sweep,
            } => {
                fov = 62.0;
                // Look slightly off centre, so the blob sits off-axis.
                target = Vec3::new(0.0, height * 0.35, 0.0);
                orbit_position(azimuth + t * sweep, radius, height)
            }
            Kind::Spline {
                points,
                fov: shot_fov,
            } => {
                fov = shot_fov;
                // Linear in t: the spline's own shape supplies the variation,
                // and easing it as well would make it lurch between points.
                spline(points, t)
            }
            Kind::Orbit {
                azimuth,
                radius,
                height,
                sweep,
            } => orbit_position(azimuth + t * sweep, radius, height),
            Kind::Rise {
                azimuth,
                radius,
                from_height,
                to_height,
            } => {
                fov = 58.0;
                let height = from_height + (to_height - from_height) * ease(t);
                orbit_position(azimuth + t * 0.6, radius, height)
            }
        };

        // A small kick back on each beat, so hits register even in a wide.
        let recoil = 1.0 + music.beat * 0.035 * (1.0 - collapse);

        // The second scene sits closer, and every shot pushes in a little over
        // its life — eased, so the drift is never a visible start or stop.
        let closer = 1.0 + (OCTOPUS_FRAMING - 1.0) * octopus;
        let creep = 1.0 - ease(t) * OCTOPUS_CREEP * octopus;

        let eye = eye * recoil * closer * creep;
        Self {
            eye,
            target,
            up: Vec3::Y,
            fov_degrees: fov,
            // The opening and ferrofluid subjects are centred implicit
            // surfaces. Focus on their front skin instead of their centre.
            // Aim at the middle of the visible body. As the blobs gather into
            // the larger octopus form its central silhouette grows, so its
            // front surface sits farther ahead of the look-at point.
            focus_distance: (eye.distance(target) - (0.62 + octopus * 0.26)).max(0.35),
        }
    }
}

/// Smootherstep: zero first and second derivatives at both ends, so a move
/// has no visible corner where it starts or stops.
fn ease(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Front-loaded ease: quick away, long settle. Higher powers are punchier.
fn ease_out(t: f32, power: f32) -> f32 {
    1.0 - (1.0 - t).powf(power)
}

/// Closed Catmull-Rom through the control points. The curve passes through
/// every point, which makes a path easy to author by hand — unlike a Bezier,
/// where the controls sit off the curve.
fn spline(points: &[[f32; 3]], t: f32) -> Vec3 {
    let count = points.len();
    let scaled = t.clamp(0.0, 0.999_9) * count as f32;
    let index = scaled.floor() as usize % count;
    let f = scaled.fract();

    // Wrapping the indices closes the loop, so the shot has no seam.
    let at = |k: usize| Vec3::from(points[(index + count + k - 1) % count]);
    let (p0, p1, p2, p3) = (at(0), at(1), at(2), at(3));

    0.5 * ((p1 * 2.0)
        + (p2 - p0) * f
        + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * f * f
        + (p1 * 3.0 - p0 - p2 * 3.0 + p3) * f * f * f)
}

/// The string's swing.
///
/// Travel along the track is constant; the music goes into how hard the string
/// bounces instead. Modulating the speed moved every bead and their spacing
/// with it, which read as the scene lurching rather than the string reacting.
#[derive(Default)]
pub struct Flow {
    /// The softened beat, held between frames.
    level: f32,
}

impl Flow {
    /// A softened beat, for anything that moves rather than scales. The pulse
    /// itself rises in one frame; applied to a position that is a teleport,
    /// which is what made the string jitter rather than swing.
    pub fn swell(&mut self, music: &Sync) -> f32 {
        let rising = music.beat > self.level;
        let tau = if rising { SWELL_ATTACK } else { SWELL_RELEASE };
        let alpha = 1.0 - (-music.dt / tau).exp();
        self.level += (music.beat - self.level) * alpha;
        self.level
    }
}

/// How quickly the string's swing builds and falls away.
const SWELL_ATTACK: f32 = 0.09;
const SWELL_RELEASE: f32 = 0.30;

/// The body's accumulated rotation.
///
/// Two axes, and the rate follows the music — which is why this is integrated
/// rather than written as a function of time. Bass drives the yaw, so the thing
/// visibly winds up when the track does.
#[derive(Default)]
pub struct Spin {
    pub yaw: f32,
    pub tilt: f32,
}

impl Spin {
    pub fn update(&mut self, music: &Sync, stage: &Stage) -> &Self {
        let drive = SPIN_BASE + music.low * SPIN_FROM_BASS + music.beat * SPIN_FROM_BEAT;
        let rate = drive * stage.winding;

        self.yaw += rate * music.dt;
        // Not a whole-number ratio of the yaw, or the two would keep syncing
        // up and the tumble would look periodic.
        self.tilt += rate * TILT_RATIO * music.dt;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Sync;

    #[test]
    fn lens_transition_is_ordered_after_the_fractal() {
        let stage_at = |beat_phase| {
            Stage::at(&Sync {
                beat_phase,
                ..Default::default()
            })
        };

        let before = stage_at(LENS_BEATS - 0.01);
        assert_eq!(before.lens_seal, 0.0);
        assert_eq!(before.lens_field, 0.0);

        let sealed = stage_at(LENS_BEATS + LENS_SEAL_BEATS);
        assert_eq!(sealed.lens_seal, 1.0);
        assert_eq!(sealed.lens_cross, 0.0);

        let crossing = stage_at(LENS_BEATS + 5.0);
        assert!(crossing.lens_cross > 0.0 && crossing.lens_cross < 1.0);
        assert!(crossing.lens_field > 0.0 && crossing.lens_field < 1.0);

        let arrived = stage_at(LENS_BEATS + LENS_TRANSITION_BEATS + 2.0);
        assert_eq!(arrived.lens_cross, 1.0);
        assert_eq!(arrived.lens_field, 1.0);
        assert_eq!(arrived.lens_particles, 1.0);
    }

    #[test]
    fn fractal_reveal_lands_on_the_drum_drop() {
        let stage_at = |beat_phase| {
            Stage::at(&Sync {
                beat_phase,
                ..Default::default()
            })
        };

        // The covered geometry swap happens earlier, but no fractal pixels
        // escape the white plateau before the measured isolated slam.
        assert_eq!(stage_at(FRACTAL_DROP_BEATS - 0.01).wash, 1.0);
        assert!(stage_at(FRACTAL_DROP_BEATS + 0.30).wash < 0.5);
        assert_eq!(
            stage_at(FRACTAL_DROP_BEATS + FRACTAL_REVEAL_BEATS).wash,
            0.0
        );
    }

    #[test]
    fn tunnel_transition_follows_a_full_lens_section() {
        let stage_at = |beat_phase| {
            Stage::at(&Sync {
                beat_phase,
                ..Default::default()
            })
        };
        let before = stage_at(TUNNEL_BEATS - 0.01);
        assert_eq!(before.tunnel_cross, 0.0);
        assert_eq!(before.tunnel_field, 0.0);

        let covered = stage_at(TUNNEL_BEATS + TUNNEL_TRANSITION_BEATS * 0.5);
        assert!(covered.tunnel_cross > 0.45 && covered.tunnel_cross < 0.55);
        assert!(covered.tunnel_field < 0.1);

        let arrived = stage_at(TUNNEL_BEATS + TUNNEL_TRANSITION_BEATS + 0.01);
        assert_eq!(arrived.tunnel_cross, 1.0);
        assert_eq!(arrived.tunnel_field, 1.0);
        assert!(arrived.tunnel_tentacles > 0.0);
    }

    #[test]
    fn cube_sea_follows_a_full_tunnel_section() {
        let stage_at = |beat_phase| {
            Stage::at(&Sync {
                beat_phase,
                ..Default::default()
            })
        };
        let before = stage_at(CUBE_TRANSITION_START_BEATS - 0.01);
        assert_eq!(before.cube_cross, 0.0);
        assert_eq!(before.cube_field, 0.0);

        let covered = stage_at(CUBE_TRANSITION_COVER_BEATS);
        assert_eq!(covered.cube_cross, 0.5);
        assert!(covered.cube_field < 0.1);

        let arrived = stage_at(CUBE_TRANSITION_END_BEATS + 0.01);
        assert_eq!(arrived.cube_cross, 1.0);
        assert_eq!(arrived.cube_field, 1.0);
        assert!(arrived.cube_gravity > 0.0);
    }

    #[test]
    fn cube_crests_follow_the_measured_drum_hits_and_gap() {
        let hit = Stage::at(&Sync {
            time: 84.11,
            ..Default::default()
        });
        assert_eq!(hit.cube_drum_strength, 1.0);
        assert!(hit.cube_drum_phase < 0.02);

        let gap = Stage::at(&Sync {
            time: 86.0,
            ..Default::default()
        });
        assert_eq!(gap.cube_drum_strength, 0.0);

        let return_hit = Stage::at(&Sync {
            time: 87.33,
            ..Default::default()
        });
        assert_eq!(return_hit.cube_drum_strength, 0.78);
        assert!(return_hit.cube_drum_phase < 0.02);
    }

    #[test]
    fn outro_reaches_silence_and_holds_black_before_exit() {
        let sync = |beat_phase| Sync {
            beat_phase,
            ..Default::default()
        };
        assert_eq!(audio_gain(&sync(OUTRO_BEATS - 0.01)), 1.0);
        assert_eq!(audio_gain(&sync(DEMO_END_BEATS - 4.0)), 0.0);

        let black = Stage::at(&sync(DEMO_END_BEATS - 0.5));
        assert_eq!(black.final_black, 1.0);
        assert!(!is_finished(&sync(DEMO_END_BEATS - 0.5)));
        assert!(is_finished(&sync(DEMO_END_BEATS)));
    }

    #[test]
    fn extended_edit_lands_on_stem_boundaries() {
        assert_eq!(SIGNAL_BEATS, 130.0);
        assert_eq!(LENS_BEATS, 152.0);
        assert_eq!(TUNNEL_BEATS, 200.0);
        assert_eq!(DEMO_END_BEATS, 304.0);
    }

    #[test]
    fn lens_camera_never_enters_a_membrane() {
        let mut largest_correction = 0.0f32;
        let mut largest_step = 0usize;
        let mut opening_correction = 0.0f32;
        for step in 0..2400 {
            let phase = step as f32 / 2400.0;
            let beat_phase = LENS_BEATS + LENS_TRANSITION_BEATS + phase * LENS_FLIGHT_BEATS;
            let music = Sync {
                time: beat_phase * 0.48,
                beat_phase,
                low: 0.75 + (phase * 31.0).sin() * 0.65,
                mid: 0.70 + (phase * 47.0).cos() * 0.60,
                ..Default::default()
            };
            let camera = lens_camera(&music);
            let raw = lens_flight_point(phase);
            let correction = (camera.eye - raw).length();
            if correction > largest_correction {
                largest_correction = correction;
                largest_step = step;
            }
            if step < 120 {
                opening_correction = opening_correction.max(correction);
            }

            for i in 0..LENS_CENTERS.len() {
                let center = animated_lens_center(i, &music);
                let delta = camera.eye - center;
                let distance = delta.length();
                let boundary = animated_lens_radius(delta.normalize_or_zero(), i, &music);
                assert!(
                    distance + 1.0e-3 >= boundary + LENS_CAMERA_CLEARANCE,
                    "step {step}: camera entered lens {i} by {:.3}",
                    boundary + LENS_CAMERA_CLEARANCE - distance,
                );
            }
        }
        assert!(
            largest_correction < 0.001,
            "collision guard moved the authored flight by {largest_correction:.3} at step {largest_step}; the spline is shaping the camera poorly",
        );
        assert!(
            opening_correction < 0.02,
            "the opening still relies on a visible {opening_correction:.3}-unit collision correction",
        );
    }

    #[test]
    fn lens_camera_always_frames_a_membrane() {
        let mut total_visible = 0usize;
        let mut minimum_visible = usize::MAX;
        let mut previous_forward = None;
        let mut stabilized_forward = None;
        let mut largest_turn = 0.0f32;
        let mut largest_turn_step = 0usize;
        for step in 0..2400 {
            let phase = step as f32 / 2400.0;
            let beat_phase = LENS_BEATS + LENS_TRANSITION_BEATS + phase * LENS_FLIGHT_BEATS;
            let music = Sync {
                time: beat_phase * 0.48,
                beat_phase,
                low: 0.75 + (phase * 31.0).sin() * 0.65,
                mid: 0.70 + (phase * 47.0).cos() * 0.60,
                dt: 1.0 / 60.0,
                ..Default::default()
            };
            let camera = lens_camera(&music);
            let desired = (camera.target - camera.eye).normalize_or_zero();
            let forward = stabilized_forward.map_or(desired, |previous| {
                stabilize_lens_view(camera.eye, previous, desired, &music)
            });
            stabilized_forward = Some(forward);
            if let Some(previous) = previous_forward {
                let turn = Vec3::dot(previous, forward).clamp(-1.0, 1.0).acos();
                if turn > largest_turn {
                    largest_turn = turn;
                    largest_turn_step = step;
                }
            }
            previous_forward = Some(forward);
            let (visible, _) = lens_visibility_score(camera.eye, forward, &music);
            minimum_visible = minimum_visible.min(visible);
            total_visible += visible;
            assert!(visible >= 1, "step {step}: no membrane was visible");
        }

        let average_visible = total_visible as f32 / 2400.0;
        assert!(minimum_visible >= 1);
        assert!(
            average_visible >= 2.0,
            "camera only framed {average_visible:.2} membranes on average",
        );
        assert!(
            largest_turn < 0.12,
            "lens framing snapped by {:.1} degrees at step {largest_turn_step}",
            largest_turn.to_degrees(),
        );
    }

    #[test]
    fn focused_lens_stays_visible_for_the_whole_flight() {
        let mut director = Director::default();
        for step in 0..2400 {
            let phase = step as f32 / 2400.0;
            let lens_section = TUNNEL_BEATS - LENS_BEATS - LENS_TRANSITION_BEATS;
            let beat_phase = LENS_BEATS + LENS_TRANSITION_BEATS + phase * lens_section;
            let music = Sync {
                time: beat_phase * 0.48,
                beat_phase,
                dt: lens_section * 0.48 / 2400.0,
                low: 0.75 + (phase * 31.0).sin() * 0.65,
                mid: 0.70 + (phase * 47.0).cos() * 0.60,
                ..Default::default()
            };
            let stage = Stage::at(&music);
            let camera = director.update(&music, &stage);
            let forward = (camera.target - camera.eye).normalize_or_zero();
            assert!(
                lens_is_visible(camera.eye, forward, director.lens_focus_index, &music,),
                "step {step}: focused lens {} left the frame",
                director.lens_focus_index,
            );
        }
    }

    /// The whole point of the fractal scene: the string of beads is the
    /// subject, so it has to be on screen. The camera is aimed at a point on
    /// the traced corridor, which is what guarantees this — an independent pan
    /// drifted off the corridor within seconds of every cut.
    #[test]
    fn the_string_stays_in_frame() {
        let mut director = Director::default();
        let stage = Stage {
            collapse: 1.0,
            ..Default::default()
        };

        let mut worst_visible_field = 1.0f32;
        let mut nearest = f32::INFINITY;
        // Two minutes at 60fps, across every shot in the cut list.
        for step in 0..7200 {
            let time = step as f32 / 60.0;
            let music = Sync {
                time,
                // Fast enough to run the cut list several times over.
                beat_phase: time * 2.0,
                hard_hit: step % 37 == 0,
                dt: 1.0 / 60.0,
                ..Default::default()
            };
            let camera = director.update(&music, &stage);

            let forward = (camera.target - camera.eye).normalize_or_zero();
            // Half the vertical field of view, which is the tighter of the two
            // on any normal aspect ratio.
            let limit = (camera.fov_degrees * 0.5).to_radians().cos();

            // The near stretch — the first few units, where beads are close
            // enough to read as beads rather than as a thread. The far end of
            // a corridor runs off behind the structure by design, so counting
            // all of it would pass on a framing that shows only the vanishing
            // point.
            //
            // The dense field deliberately enters from all around the frame,
            // so judge the visible foreground as a whole rather than forcing
            // every outer thread into the centre of every shot.
            let near = crate::fractal::TRACK_POINTS / 4;
            let points = director
                .bundle
                .iter()
                .flat_map(|corridor| &corridor.points[..near]);
            let (visible, total) = points.fold((0usize, 0usize), |(visible, total), point| {
                let to = (*point - camera.eye).normalize_or_zero();
                let visible = visible + usize::from(to.dot(forward) > limit);
                nearest = nearest.min((*point - camera.eye).length());
                (visible, total + 1)
            });
            worst_visible_field = worst_visible_field.min(visible as f32 / total as f32);
        }

        assert!(
            worst_visible_field > 0.4,
            "only {:.0}% of the field's near stretch was ever in frame",
            worst_visible_field * 100.0,
        );
        assert!(
            nearest < 1.5,
            "the closest foreground string was still {nearest:.2} units away",
        );
    }
}
