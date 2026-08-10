//! When things happen.
//!
//! The demo's structure lives here rather than being scattered through the
//! shaders: a section list keyed on the audio clock, producing the handful of
//! values the renderer needs this frame. Sections and camera direction will
//! extend this same table.

use glam::Vec3;

use crate::audio::Sync;

/// Cards shown before the scene appears, as (start, end) in seconds.
const CARDS: [(f32, f32); 3] = [(0.4, 2.9), (3.2, 5.5), (5.8, 8.1)];
/// Apparent size of each card. The name sits closest to the camera; the rest
/// fall back behind it.
const CARD_SCALES: [f32; 3] = [1.9, 1.0, 1.25];
/// How long a card takes to fade in and out.
const CARD_FADE: f32 = 0.35;
/// When the scene begins to appear, and how long it takes.
const SCENE_START: f32 = 7.7;
const SCENE_FADE: f32 = 2.4;
/// When the ferrofluid takes over, in beats from the start.
const SPIKE_BEATS: f32 = 64.0;

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
    /// How far the blob has turned into a ferrofluid, 0 to 1.
    pub spike: f32,
    /// Transition mask threshold: 0 is fully the old scene, 1 the new one.
    pub dissolve: f32,
}

impl Stage {
    pub fn at(music: &Sync) -> Self {
        let t = music.time;

        let mut card = -1;
        let mut card_alpha = 0.0;
        let mut scroll = 0.0;
        let mut scale = 1.0;
        let mut card_progress = 0.0;
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

        // Cards breathe on the beat rather than sitting flat.
        card_alpha *= 0.82 + music.beat * 0.35;

        // Scene changes, in beats from the start of the tune. The blob spends
        // the opening as a fluid, then bristles.
        let beats = music.beat_phase;
        let spike = smoothstep(SPIKE_BEATS, SPIKE_BEATS + 16.0, beats);
        let dissolve = smoothstep(SPIKE_BEATS - 4.0, SPIKE_BEATS + 12.0, beats);

        Self {
            card,
            card_alpha,
            spike,
            dissolve,
            scene: smoothstep(SCENE_START, SCENE_START + SCENE_FADE, t),
            scroll,
            scale,
            card_progress,
        }
    }
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
    pub fov_degrees: f32,
}

/// A single shot. `beats` is how long it holds before cutting to the next.
struct Shot {
    beats: f32,
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
        kind: Kind::Close {
            azimuth: 2.3,
            radius: 1.95,
            height: -0.35,
            sweep: 0.5,
        },
    },
    Shot {
        beats: 16.0,
        kind: Kind::Spline {
            points: &FLIGHT,
            fov: 56.0,
        },
    },
    Shot {
        beats: 8.0,
        kind: Kind::Rise {
            azimuth: 1.2,
            radius: 2.6,
            from_height: -1.2,
            to_height: 1.6,
        },
    },
    Shot {
        beats: 8.0,
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
}

/// How long past its length a shot waits for an accent before cutting anyway.
const CUT_GRACE: f32 = 4.0;

impl Default for Director {
    fn default() -> Self {
        Self {
            shot: 0,
            started: 0.0,
        }
    }
}

impl Director {
    pub fn update(&mut self, music: &Sync) -> Camera {
        let length = SHOTS[self.shot].beats;
        let elapsed = music.beat_phase - self.started;

        // Willing to cut, and either an accent arrived or patience ran out.
        if elapsed >= length && (music.hard_hit || elapsed >= length + CUT_GRACE) {
            self.shot = (self.shot + 1) % SHOTS.len();
            self.started = music.beat_phase;
        }

        let shot = &SHOTS[self.shot];
        // Clamped, so a shot held past its length holds its final framing
        // rather than running off the end of the move.
        let t = ((music.beat_phase - self.started) / shot.beats).clamp(0.0, 1.0);
        Camera::compose(shot, t, music)
    }
}

impl Camera {
    fn compose(shot: &Shot, t: f32, music: &Sync) -> Self {

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
                let e = if snap > 0.0 { ease_out(t, snap) } else { ease(t) };
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
            Kind::Spline { points, fov: shot_fov } => {
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
        let recoil = 1.0 + music.beat * 0.02;

        Self {
            eye: eye * recoil,
            target,
            fov_degrees: fov,
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
