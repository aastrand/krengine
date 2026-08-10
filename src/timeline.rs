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
/// Credits, in beats from the start, running under the ferrofluid. Indexed
/// after the three intro cards.
const CREDITS: [(f32, f32); 3] = [
    (MERGE_BEATS + 4.0, MERGE_BEATS + 12.0),
    (MERGE_BEATS + 13.0, MERGE_BEATS + 21.0),
    (MERGE_BEATS + 22.0, MERGE_BEATS + 30.0),
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
/// When the ferrofluid takes over, in beats from the start. The intro ends
/// around beat 16, so this leaves a short first scene rather than a long one.
const SPIKE_BEATS: f32 = 32.0;
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

/// How far along the corridor the camera looks, as a fraction of its length,
/// and how far that aim drifts either side of it. The drift is the whole of
/// the camera's motion in this scene: the view tracks along the string rather
/// than turning on a clock of its own.
///
/// Near the start, because that is the part of the corridor within a few units
/// of the camera — where beads still read as beads. Aimed halfway along, the
/// view points at something twelve units off and the near string, which is the
/// part worth looking at, sits out at the edge of frame.
const FRACTAL_AIM: f32 = 0.12;
const FRACTAL_AIM_DRIFT: f32 = 0.06;
/// Radians per second the aim sweeps back and forth along the corridor.
const FRACTAL_PAN: f32 = 0.035;

/// Where the corridor starts relative to the camera: this far in front, and
/// this far off to one side, so the string enters frame from the edge and
/// recedes rather than flying straight at the lens.
const FRACTAL_FOCUS: f32 = 1.9;
const FRACTAL_OFFSET: f32 = 1.4;

/// Clearance the glide keeps from the structure, and how hard the resulting
/// path is smoothed, in seconds. The clearance has to be something the
/// distance field can actually offer — see fractal::MAX_CLEARANCE.
const FRACTAL_CLEARANCE: f32 = 0.14;
const GLIDE_SMOOTHING: f32 = 0.55;

/// How close in the shot settles once it is inside.
const FRACTAL_INSIDE: f32 = 3.8;

/// How much longer shots hold in the fractal scene.
const FRACTAL_SHOT_HOLD: f32 = 2.2;

/// How much closer the shot drifts over the whole scene, as a fraction.
const FRACTAL_DIVE: f32 = 0.22;

/// When the room contracts and takes the scene with it.
const COLLAPSE_BEATS: f32 = MERGE_BEATS + 34.0;
const COLLAPSE_RAMP: f32 = 12.0;

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
/// How much smoke the arms shed once they are trailing. A wisp, not a scene.
const OCTOPUS_SMOKE: f32 = 0.15;

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
    /// Extra dye shed during a transition, to give the wipe a front.
    pub burst: f32,
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
    /// The room's palette shift, on its own slower ramp than the body's. A
    /// background caught changing draws attention to itself; the point is that
    /// it has receded, not that it receded just now.
    pub palette: f32,
    /// How much smoke there is: full in the first scene, gone through the
    /// change, then a little again once the arms are trailing.
    pub smoke: f32,
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

        // Cards breathe on the beat rather than sitting flat.
        card_alpha *= 0.82 + music.beat * 0.35;

        // Scene changes, in beats from the start of the tune. The blob spends
        // the opening as a fluid, then bristles.
        let spike = smoothstep(SPIKE_BEATS, SPIKE_BEATS + SPIKE_RAMP, beats);

        // The wipe leads the change slightly, so the new scene is revealed
        // rather than appearing and then being wiped to.
        let dissolve = smoothstep(SPIKE_BEATS - 2.0, SPIKE_BEATS + SPIKE_RAMP * 0.7, beats);

        // A burst of dye just before the wipe, so the mask has a front to
        // sweep instead of only the thin wakes the beads leave.
        let since = beats - (SPIKE_BEATS - 3.0);
        let burst = if since > 0.0 {
            (1.0 - since / 5.0).clamp(0.0, 1.0).powf(0.6)
        } else {
            0.0
        };

        let merge = smoothstep(MERGE_BEATS, MERGE_BEATS + MERGE_RAMP, beats);
        let palette = smoothstep(MERGE_BEATS, MERGE_BEATS + PALETTE_RAMP, beats);
        let collapse = smoothstep(COLLAPSE_BEATS, COLLAPSE_BEATS + COLLAPSE_RAMP, beats);
        let bleed = smoothstep(COLLAPSE_BEATS - 6.0, COLLAPSE_BEATS + 2.0, beats);

        // The gap between the two is deliberate: the smoke clears completely
        // before the arms start shedding their own.
        // The bleed puts the body into the water, then the water clears with
        // the room — the fractal scene has no smoke in it.
        // The arms stop shedding well before the room goes, so the frame is
        // clear when the transition starts. Smoke still hanging around during
        // a scene change reads as the old scene refusing to leave — and the
        // bleed sheds none at all now, since ink over the collapse looked like
        // calligraphy rather than a body coming apart.
        let clearing = 1.0 - smoothstep(COLLAPSE_BEATS - 12.0, COLLAPSE_BEATS - 5.0, beats);
        let smoke = (1.0 - spike).max(merge * OCTOPUS_SMOKE * clearing);

        Self {
            card,
            card_alpha,
            merge,
            palette,
            collapse,
            bleed,
            dive: smoothstep(COLLAPSE_BEATS, COLLAPSE_BEATS + 160.0, beats),
            smoke,
            winding: smoothstep(SPIN_BEATS, SPIN_BEATS + 12.0, beats),
            spike,
            dissolve,
            burst,
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
    /// Smoothed camera position for the fractal glide. The raw position is
    /// corrected against the structure every frame, and those corrections are
    /// not continuous — filtering them is what keeps the glide smooth.
    glide: Vec3,
    glide_started: bool,
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
    /// The corridor the bead string runs along, retraced on each cut.
    pub corridor: crate::fractal::Corridor,
}

/// How much closer the second scene frames, and how far each of its shots
/// creeps in over its life.
const OCTOPUS_FRAMING: f32 = 0.78;
const OCTOPUS_CREEP: f32 = 0.12;

/// How long past its length a shot waits for an accent before cutting anyway.
const CUT_GRACE: f32 = 4.0;

impl Default for Director {
    fn default() -> Self {
        Self {
            shot: 0,
            started: 0.0,
            glide: Vec3::ZERO,
            glide_started: false,
            along: 0.0,
            radius: FRACTAL_INSIDE,
            placed_for: usize::MAX,
            traced_for: usize::MAX,
            corridor: crate::fractal::Corridor::default(),
        }
    }
}

impl Director {
    pub fn update(&mut self, music: &Sync, stage: &Stage) -> Camera {
        // The fractal wants long takes: it is a place to look around, not a
        // subject to cut around.
        let length = SHOTS[self.shot].beats * (1.0 + stage.collapse * FRACTAL_SHOT_HOLD);
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
        let mut camera = Camera::compose(shot, t, music, stage.merge, stage.collapse);

        // The fractal is metres across, so the shot list's radii would put the
        // camera in a wall. Frame the whole structure first, then work inward
        // to a vantage point found by asking the distance estimator where the
        // open pockets are.
        if stage.collapse > 0.0 {
            // The camera stands still and pans. Flying it through the
            // structure fought the structure: the eye cannot follow a shape
            // this dense while the viewpoint is also moving, and every
            // correction against a wall showed up as a lurch.
            //
            // It does move, but by cutting: each shot stands somewhere else on
            // the path, so a long scene is a series of held views rather than
            // one. The offset is an irrational-ish step, so successive stands
            // do not land near each other.
            let along = (FRACTAL_STAND + self.shot as f32 * 0.137).rem_euclid(1.0);
            let radius = FRACTAL_INSIDE
                * (1.0 - stage.dive * FRACTAL_DIVE)
                * (0.9 + (self.shot % 3) as f32 * 0.1);
            let path = spline(&FRACTAL_PATH, along) * radius;

            self.along = along;
            self.radius = radius;

            let corrected = crate::fractal::push_clear(path, FRACTAL_CLEARANCE);

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
            // beads. Roughly towards the middle, turned a little per shot so
            // successive stands do not look the same way.
            let turn = self.shot as f32 * 1.31;
            let facing = (Vec3::ZERO - self.glide).normalize_or_zero();
            let forward = Vec3::new(
                facing.x * turn.cos() - facing.z * turn.sin(),
                facing.y,
                facing.x * turn.sin() + facing.z * turn.cos(),
            )
            .normalize_or_zero();
            let across = forward.cross(Vec3::Y).normalize_or_zero();
            let above = across.cross(forward).normalize_or_zero();

            // The corridor's heading: mostly away from the camera, angled
            // across the frame so the line reads as travelling through the
            // structure rather than as a dot coming towards the lens.
            let heading = (forward + across * 0.55 - above * 0.18).normalize_or_zero();

            // Started off to one side and in front, then cleared — a start
            // buried in the structure puts the head of the string inside a
            // wall, where every bead on it is culled.
            let start = crate::fractal::push_clear(
                self.glide + forward * FRACTAL_FOCUS - across * FRACTAL_OFFSET,
                FRACTAL_CLEARANCE,
            );

            // Traced once per shot, not per frame. The trace steers on the
            // distance field, so a slightly different start finds a slightly
            // different route — retracing every frame redrew the corridor
            // underneath the beads, which is what had them blinking and
            // jumping about.
            if self.traced_for != self.shot {
                self.traced_for = self.shot;
                crate::fractal::trace_corridor(start, heading, &mut self.corridor);

                if std::env::var("KR_DEBUG").is_ok() {
                    let clear = self
                        .corridor
                        .points
                        .iter()
                        .filter(|point| {
                            crate::fractal::distance(**point)
                                > crate::fractal::TRACK_CLEARANCE * 0.5
                        })
                        .count();
                    log::info!(
                        "corridor: {clear}/{} points clear, start {:.2?}",
                        crate::fractal::TRACK_POINTS,
                        self.corridor.points[0],
                    );
                }
            }

            // Not find_vantage: choosing the best of a set of candidates is a
            // discrete pick, and as the path advances the winner flips from one
            // to another and the camera jumps. Only the continuous correction
            // is used here.
            let arrival = smoothstep(0.85, 1.0, stage.collapse);
            camera.eye = camera.eye.lerp(inside, arrival);

            // The view sits on the string and slides slowly along it. That is
            // the only motion in the scene — the camera stands still — and
            // because the aim is a point on the corridor itself, the string
            // cannot leave the frame however the trace happened to route.
            let sweep = FRACTAL_AIM + (music.time * FRACTAL_PAN).sin() * FRACTAL_AIM_DRIFT;
            let aim = self.corridor_point(sweep);
            camera.target = camera.target.lerp(aim, arrival);
        }
        camera
    }

    /// A point on the traced corridor, by fraction of its length.
    fn corridor_point(&self, t: f32) -> Vec3 {
        let last = crate::fractal::TRACK_POINTS - 1;
        let scaled = t.clamp(0.0, 1.0) * last as f32;
        let index = (scaled.floor() as usize).min(last - 1);
        let points = &self.corridor.points;
        points[index].lerp(points[index + 1], scaled.fract())
    }
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
        let recoil = 1.0 + music.beat * 0.02 * (1.0 - collapse);

        // The second scene sits closer, and every shot pushes in a little over
        // its life — eased, so the drift is never a visible start or stop.
        let closer = 1.0 + (OCTOPUS_FRAMING - 1.0) * octopus;
        let creep = 1.0 - ease(t) * OCTOPUS_CREEP * octopus;

        Self {
            eye: eye * recoil * closer * creep,
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

        let mut worst = 1.0f32;
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
            // the corridor runs off behind the structure by design, so
            // counting all of it would pass on a framing that shows only the
            // vanishing point.
            let near = crate::fractal::TRACK_POINTS / 4;
            let visible = director.corridor.points[..near]
                .iter()
                .filter(|point| {
                    let to = (**point - camera.eye).normalize_or_zero();
                    to.dot(forward) > limit
                })
                .count();
            worst = worst.min(visible as f32 / near as f32);
        }

        assert!(
            worst > 0.5,
            "only {:.0}% of the near corridor was ever in frame",
            worst * 100.0,
        );
    }
}
