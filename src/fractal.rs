//! The mandelbox, on the CPU.
//!
//! A copy of the distance estimator in the shader, so the camera can ask where
//! the surfaces are. Without it, placing a camera "inside" the structure is
//! guesswork: most points are either buried in a wall or out in empty space,
//! and the interesting ones — open pockets with architecture close by — cannot
//! be found by picking coordinates by hand.

use glam::Vec3;

const ITERATIONS: usize = 8;
/// Must match FRACTAL_FOLD in common.wgsl, or the camera solves a different
/// shape from the one on screen.
const FOLD: f32 = 1.22;

/// Distance estimate to the surface. An Apollonian gasket: fold into the unit
/// cell, invert through a sphere, repeat.
pub fn distance(point: Vec3) -> f32 {
    let mut p = point;
    let mut scale = 1.0f32;

    for _ in 0..ITERATIONS {
        p = -Vec3::ONE + 2.0 * (0.5 * p + Vec3::splat(0.5)).fract_gl();

        let squared = p.dot(p).max(1.0e-6);
        let factor = FOLD / squared;
        p *= factor;
        scale *= factor;
    }

    0.25 * p.y.abs() / scale
}

fn gradient(point: Vec3) -> Vec3 {
    let e = 0.002;
    Vec3::new(
        distance(point + Vec3::X * e) - distance(point - Vec3::X * e),
        distance(point + Vec3::Y * e) - distance(point - Vec3::Y * e),
        distance(point + Vec3::Z * e) - distance(point - Vec3::Z * e),
    )
    .normalize_or_zero()
}

/// The largest value the estimator ever returns, measured by sampling the
/// field (see the `field_is_shallow` test). This is the single most important
/// number here: the estimator folds space into a unit cell and measures to a
/// plane, so it is a heavy *under*estimate of the true distance, and it
/// saturates. Every clearance below is a fraction of this — asking for a
/// margin above it means asking for something no point in space satisfies,
/// and any loop that iterates until it is met will never terminate early.
pub const MAX_CLEARANCE: f32 = 0.234;

/// Clearance the corridor aims for: comfortably inside a pocket, but well
/// under what the field can actually offer.
pub const TRACK_CLEARANCE: f32 = 0.155;
/// The most a single correction step may move a point. Without it a step taken
/// where the gradient is unreliable throws the point across the structure.
const CORRECTION_STEP: f32 = 0.06;
/// And the most the correction may move it in total.
///
/// This is what keeps a placed point where it was placed. Uncapped, the climb
/// would follow the field uphill for as long as it kept improving — up to a
/// unit and a half — so a string the director positioned in front of the
/// camera could set off from somewhere else entirely, which is exactly how
/// they ended up far away and below the architecture. A point that cannot find
/// room within this radius stays in the tightest spot it found and is left to
/// the visibility fade instead.
const MAX_TRAVEL: f32 = 0.4;

/// Walk a point towards more open space, up to `margin`.
///
/// Gradient ascent on the distance estimate: the gradient points away from the
/// nearest surface, so following it uphill lands in the middle of whatever
/// pocket the point started in. It stops as soon as the margin is met, when
/// the field stops improving, or after a fixed budget — a point in a pocket
/// too tight to satisfy `margin` settles at the best spot it found rather than
/// wandering off looking for a better one.
pub fn push_clear(point: Vec3, margin: f32) -> Vec3 {
    push_clear_within(point, margin, MAX_TRAVEL)
}

/// The same, with an explicit cap on how far the point may be moved.
///
/// The trace needs a much tighter one than a bare placement does: its steps
/// are TRACK_STEP apart, and a correction free to move a point several times
/// that turns a forward walk into a stagger — the corridor doubles back on
/// itself and the frame carried along it flips.
pub fn push_clear_within(point: Vec3, margin: f32, cap: f32) -> Vec3 {
    // Never ask for more than the field can express, or the loop below simply
    // runs its whole budget every call.
    let margin = margin.min(MAX_CLEARANCE * 0.95);

    let mut p = point;
    let mut best = p;
    let mut best_d = distance(p);

    for _ in 0..24 {
        if best_d >= margin {
            break;
        }
        let g = gradient(p);
        if g == Vec3::ZERO {
            break;
        }
        // The estimator understates distance, so a step of exactly what is
        // missing undershoots rather than overshooting — safe to iterate.
        p += g * (margin - best_d).min(CORRECTION_STEP);

        // Held inside the cap rather than stopped at it, so a point that
        // drifts to the edge still settles on the best spot in reach.
        let travel = p - point;
        if travel.length() > cap {
            p = point + travel.normalize() * cap;
        }

        let d = distance(p);
        if d <= best_d {
            // Uphill has run out: this pocket is as open as it gets.
            break;
        }
        best_d = d;
        best = p;
    }
    best
}

/// How far a ray gets before the structure stops it.
///
/// This is how a vantage point is judged: a stand with a wall a few
/// centimetres in front of it is useless however open the pocket around it is,
/// because the strings have nowhere to run and the view is a close-up of a
/// surface.
pub fn free_run(origin: Vec3, dir: Vec3, limit: f32) -> f32 {
    let dir = dir.normalize_or_zero();
    let mut travel = 0.05;
    for _ in 0..96 {
        let d = distance(origin + dir * travel);
        if d < 0.01 {
            return travel;
        }
        travel += d.max(0.01);
        if travel >= limit {
            return limit;
        }
    }
    travel
}

/// How many strings run through the structure at once. Each gets its own
/// traced corridor, started far enough off the others that the trace settles
/// it into a different tunnel — see `strings_take_different_tunnels`.
pub const STRINGS: usize = 3;
/// Points along each. Fine spacing keeps the curve smooth where it bends around
/// a pillar; the shader interpolates between them.
pub const TRACK_POINTS: usize = 192;
/// Arc length between consecutive points, in world units. The full corridor is
/// therefore (TRACK_POINTS - 1) * TRACK_STEP long.
///
/// Short, because the camera only sees a few units of structure before the
/// detail closes up: a corridor running thirty units into the distance spent
/// most of its beads somewhere too far away and too occluded to read.
pub const TRACK_STEP: f32 = 0.09;

/// How much of the previous heading is kept at each step. High, so the line
/// sweeps through long curves instead of reacting to every pocket it passes.
const MOMENTUM: f32 = 0.82;

/// Steps the raw walk takes before it is resampled down to TRACK_POINTS.
///
/// More than the corridor needs, because a step does not always advance a full
/// TRACK_STEP: the correction can pull sideways or a little backwards, so the
/// walk is shorter than the sum of its steps. Walking only as far as the
/// corridor wanted left the resample with no material for the tail, and it
/// filled it by repeating the last point — a run of identical points, whose
/// tangent is undefined and whose carried frame then spun on the spot.
const WALK_POINTS: usize = TRACK_POINTS * 3 / 2;

/// A traced corridor: where the line goes, a frame to wind the curl around,
/// and how much room there is to wind it in.
pub struct Corridor {
    pub points: [Vec3; TRACK_POINTS],
    /// Unit vector perpendicular to the tangent at each point, carried along
    /// the curve by parallel transport.
    pub normals: [Vec3; TRACK_POINTS],
    /// Distance to the nearest surface at each point.
    ///
    /// The curl is what the string is *for*, but a fixed radius does not fit
    /// a corridor whose width changes: wound wide it puts beads inside the
    /// walls wherever the passage narrows, and those beads are culled, which
    /// is what left the strings as short disconnected fragments. The shader
    /// scales each turn of the helix to the room actually available here.
    pub clearance: [f32; TRACK_POINTS],
}

impl Default for Corridor {
    fn default() -> Self {
        Self {
            points: [Vec3::ZERO; TRACK_POINTS],
            normals: [Vec3::X; TRACK_POINTS],
            clearance: [0.0; TRACK_POINTS],
        }
    }
}

/// Trace a line that follows the holes in the structure.
///
/// Each step probes forward, then lets that probe climb into the middle of
/// whatever pocket it landed in, and takes the corrected point as the next one
/// — so the route is chosen by the structure rather than imposed on it. Down a
/// tunnel it runs straight; at an arch it bends around. An analytic curve
/// cannot do this: it has no idea where the walls are, so it clips through
/// them.
///
/// The raw walk has uneven steps, because the correction moves points
/// sideways by varying amounts. It is smoothed and then resampled to uniform
/// arc length, which is what lets the shader place a bead by distance along
/// the line and get even spacing.
pub fn trace_corridor(start: Vec3, heading: Vec3, out: &mut Corridor) {
    let mut walk = [Vec3::ZERO; WALK_POINTS];
    let mut p = push_clear(start, TRACK_CLEARANCE);
    let mut dir = heading.normalize_or_zero();
    if dir == Vec3::ZERO {
        dir = Vec3::Z;
    }

    for slot in walk.iter_mut() {
        *slot = p;

        // Corrected by little more than the step itself, so the walk keeps
        // going forward and the curve stays smooth enough to carry a frame.
        let probe = push_clear_within(p + dir * TRACK_STEP, TRACK_CLEARANCE, TRACK_STEP * 1.2);
        let step = probe - p;

        // A correction that undid the step entirely would fold the line back
        // on itself; keep going on the old heading instead.
        let turned = step.normalize_or_zero();
        if turned != Vec3::ZERO && turned.dot(dir) > 0.0 {
            dir = (dir * MOMENTUM + turned * (1.0 - MOMENTUM)).normalize_or_zero();
        }
        p = probe;
    }

    smooth(&mut walk);
    resample(&walk, &mut out.points);
    transport(&out.points, &mut out.normals);

    for (slot, point) in out.clearance.iter_mut().zip(out.points.iter()) {
        *slot = distance(*point);
    }
    // Smoothed along the corridor, or the helix changes width abruptly where
    // the field does and the string looks pinched rather than tapered.
    for _ in 0..3 {
        let source = out.clearance;
        for i in 1..TRACK_POINTS - 1 {
            out.clearance[i] = (source[i - 1] + source[i] * 2.0 + source[i + 1]) * 0.25;
        }
    }
}

/// How far apart the strings start, across the frame and up it. Wide enough
/// that each settles into a tunnel of its own: the structure repeats every two
/// units, so starts a good fraction of that apart are in different cells, and
/// the trace then keeps them there by following whatever opening it is in.
///
/// Too small and all three converge into the same corridor and read as one
/// thick string; too large and the outer two start outside the frame the
/// camera is holding on the middle one.
const STRING_SPREAD: f32 = 1.15;
/// The strings are also staggered along the heading, so they do not all begin
/// at the same depth and cross the frame in lockstep.
const STRING_STAGGER: f32 = 0.55;

/// Trace the whole bundle: several strings setting off through the structure
/// in the same direction, each down its own tunnel.
///
/// They share a heading rather than being splayed apart, so they read as one
/// flow through the architecture — several threads of the same current — and
/// not as a starburst. Where they end up is left to the structure: given
/// different starting cells the trace routes each around whatever is in front
/// of it, so they diverge and rejoin on their own.
pub fn trace_bundle(
    origin: Vec3,
    heading: Vec3,
    across: Vec3,
    above: Vec3,
    out: &mut [Corridor; STRINGS],
) {
    for (i, corridor) in out.iter_mut().enumerate() {
        // Centred on the origin, so the middle string is the one the camera is
        // framed on and the others sit either side of it.
        let offset = i as f32 - (STRINGS - 1) as f32 * 0.5;

        // Tilted a little as it goes up, so the three are not a flat row.
        // Spread mostly sideways, only slightly up. The open space in this
        // structure runs in sheets, so strings lifted well out of the plane
        // the camera is in end up above the architecture rather than in it.
        let start = origin
            + across * (offset * STRING_SPREAD)
            + above * (offset * STRING_SPREAD * 0.15)
            + heading * (offset.abs() * STRING_STAGGER);

        trace_corridor(push_clear(start, TRACK_CLEARANCE), heading, corridor);
    }
}

/// Two passes of a three-point average. The correction is not continuous —
/// neighbouring points can settle into pockets a little off from each other —
/// and unsmoothed that shows up as the string twitching bead to bead.
fn smooth(points: &mut [Vec3; WALK_POINTS]) {
    for _ in 0..2 {
        let source = *points;
        for i in 1..WALK_POINTS - 1 {
            points[i] = (source[i - 1] + source[i] * 2.0 + source[i + 1]) * 0.25;
        }
    }
}

/// Resample a polyline to uniform arc length, so index maps to distance.
fn resample(walk: &[Vec3; WALK_POINTS], out: &mut [Vec3; TRACK_POINTS]) {
    let mut arc = [0.0f32; WALK_POINTS];
    for i in 1..WALK_POINTS {
        arc[i] = arc[i - 1] + (walk[i] - walk[i - 1]).length();
    }
    let total = arc[WALK_POINTS - 1];
    if total <= 1.0e-4 {
        return;
    }

    // Normally the walk is longer than the corridor and this is exactly
    // TRACK_STEP. Where a trace got badly obstructed and came up short, the
    // corridor is squeezed into what there is instead: slightly closer
    // spacing than the shader assumes, which nothing can see, and never the
    // run of duplicate points that clamping to the end would leave.
    let wanted = (TRACK_POINTS - 1) as f32 * TRACK_STEP;
    let step = TRACK_STEP.min(total / (TRACK_POINTS - 1) as f32);
    debug_assert!(total > 0.0 && (total >= wanted || step < TRACK_STEP));

    let mut cursor = 1;
    for (i, slot) in out.iter_mut().enumerate() {
        let target = i as f32 * step;
        while cursor < WALK_POINTS - 1 && arc[cursor] < target {
            cursor += 1;
        }
        let span = (arc[cursor] - arc[cursor - 1]).max(1.0e-6);
        let f = ((target - arc[cursor - 1]) / span).clamp(0.0, 1.0);
        *slot = walk[cursor - 1].lerp(walk[cursor], f);
    }
}

/// Carry a perpendicular frame along the curve by parallel transport.
///
/// The curl needs an axis to wind around. Rebuilding one per point from a
/// fixed reference (world up, say) flips wherever the tangent passes near that
/// reference, and every bead there jumps a half turn. Transporting one frame
/// from the previous point cannot flip: it only ever rotates by the small
/// angle between neighbouring tangents.
fn transport(points: &[Vec3; TRACK_POINTS], out: &mut [Vec3; TRACK_POINTS]) {
    let tangent_at = |i: usize| {
        let a = points[i.saturating_sub(1)];
        let b = points[(i + 1).min(TRACK_POINTS - 1)];
        (b - a).normalize_or_zero()
    };

    let mut tangent = tangent_at(0);
    if tangent == Vec3::ZERO {
        tangent = Vec3::Z;
    }
    let tangent = tangent;
    // Any perpendicular will do to start; the transport keeps it consistent.
    let seed = if tangent.dot(Vec3::Y).abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let mut normal = (seed - tangent * seed.dot(tangent)).normalize_or_zero();

    for (i, slot) in out.iter_mut().enumerate() {
        let next = tangent_at(i);
        if next != Vec3::ZERO {
            // Project the old normal onto the new tangent's plane: the minimal
            // rotation that keeps it perpendicular.
            let projected = normal - next * normal.dot(next);
            if projected.length_squared() > 1.0e-8 {
                normal = projected.normalize();
            }
        }
        *slot = normal;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The estimator saturates well below 1.0. Clearances are written as
    /// fractions of this, so if the shape ever changes, this is the number to
    /// re-measure.
    #[test]
    fn field_is_shallow() {
        let n = 60;
        let mut max: f32 = 0.0;
        for i in 0..n {
            for j in 0..n {
                for k in 0..n {
                    let p =
                        Vec3::new(i as f32, j as f32, k as f32) / n as f32 * 8.0 - Vec3::splat(4.0);
                    max = max.max(distance(p));
                }
            }
        }
        assert!(
            max <= MAX_CLEARANCE,
            "field reaches {max}, above MAX_CLEARANCE"
        );
        assert!(
            max > MAX_CLEARANCE * 0.9,
            "field only reaches {max}; retune"
        );
    }

    /// Every point on a traced corridor should sit in open space, and the
    /// spacing should be even enough for the shader to index by distance.
    #[test]
    fn corridor_stays_in_the_open() {
        for seed in 0..24 {
            let a = seed as f32 * 2.399_963;
            let start = Vec3::new(a.sin() * 3.0, (a * 1.7).cos() * 3.0, (a * 0.3).sin() * 3.0);
            let heading = Vec3::new((a * 0.7).cos(), (a * 1.3).sin() * 0.5, (a * 0.7).sin());

            let mut corridor = Corridor::default();
            trace_corridor(start, heading, &mut corridor);

            let buried = corridor
                .points
                .iter()
                .filter(|p| distance(**p) < TRACK_CLEARANCE * 0.35)
                .count();
            assert!(
                buried * 20 < TRACK_POINTS,
                "seed {seed}: {buried}/{TRACK_POINTS} points against a wall",
            );

            for pair in corridor.points.windows(2) {
                let gap = (pair[1] - pair[0]).length();
                assert!(
                    gap < TRACK_STEP * 1.6,
                    "seed {seed}: {gap} gap, above the {TRACK_STEP} spacing",
                );
            }

            // The same central difference the transport uses, so this checks
            // the frame rather than the choice of tangent.
            for (i, n) in corridor
                .normals
                .iter()
                .enumerate()
                .take(TRACK_POINTS - 1)
                .skip(1)
            {
                assert!((n.length() - 1.0).abs() < 1.0e-3, "frame not unit length");
                let tangent = (corridor.points[i + 1] - corridor.points[i - 1]).normalize_or_zero();
                assert!(
                    n.dot(tangent).abs() < 1.0e-3,
                    "seed {seed}: frame not perpendicular at {i}",
                );
            }

            // And it must not flip. Neighbouring frames turn by the angle
            // between neighbouring tangents, which on a tight bend is a real
            // rotation — the bound is not there to keep that small, only to
            // catch the half-turn reversal that a frame rebuilt from a fixed
            // reference produces, and which throws every bead at that point to
            // the other side of the string.
            for (i, pair) in corridor.normals.windows(2).enumerate() {
                let turn = pair[0].dot(pair[1]);
                assert!(turn > 0.5, "seed {seed}: frame turned by {turn} at {i}");
            }
        }
    }

    /// Clearing a point must not relocate it. Everything that places a string
    /// — the director putting the bundle in front of the camera, the trace
    /// stepping forward — assumes the point comes back near where it went in.
    #[test]
    fn clearing_a_point_keeps_it_where_it_was() {
        for seed in 0..400 {
            let a = seed as f32 * 2.399_963;
            let p = Vec3::new(a.sin() * 4.0, (a * 1.7).cos() * 4.0, (a * 0.3).sin() * 4.0);
            let moved = (push_clear(p, TRACK_CLEARANCE) - p).length();
            assert!(
                moved <= MAX_TRAVEL + 1.0e-3,
                "seed {seed}: clearing moved the point {moved}, past the {MAX_TRAVEL} cap",
            );
        }
    }

    /// The bundle has to start where the director put it, and set off away
    /// from there. A string that begins somewhere else is off frame however
    /// the camera is aimed.
    #[test]
    fn the_bundle_starts_where_it_is_put() {
        // The spread itself, plus what clearing is allowed to add.
        let reach = STRING_SPREAD + STRING_STAGGER + MAX_TRAVEL * 2.0;

        for seed in 0..16 {
            let a = seed as f32 * 2.399_963;
            let origin = Vec3::new(a.sin() * 3.0, (a * 1.7).cos() * 3.0, (a * 0.3).sin() * 3.0);
            let heading =
                Vec3::new((a * 0.7).cos(), (a * 1.3).sin() * 0.5, (a * 0.7).sin()).normalize();
            let across = heading.cross(Vec3::Y).normalize();
            let above = across.cross(heading).normalize();

            let mut bundle = std::array::from_fn(|_| Corridor::default());
            trace_bundle(origin, heading, across, above, &mut bundle);

            for (i, corridor) in bundle.iter().enumerate() {
                let start = (corridor.points[0] - origin).length();
                assert!(
                    start < reach,
                    "seed {seed}: string {i} starts {start} from the origin, past {reach}",
                );

                // And it must go somewhere. Not *downrange* — a corridor that
                // follows a tunnel round a bend and comes back on itself is a
                // good string, and the curl is half the point — but it must
                // cover ground rather than coiling up in one spot, which is
                // what a trace that cannot get out of its starting pocket
                // produces.
                let walked: f32 = corridor
                    .points
                    .windows(2)
                    .map(|pair| (pair[1] - pair[0]).length())
                    .sum();
                let wanted = (TRACK_POINTS - 1) as f32 * TRACK_STEP;
                assert!(
                    walked > wanted * 0.95,
                    "seed {seed}: string {i} covered {walked} of the {wanted} it should have",
                );

                let net = (corridor.points[TRACK_POINTS - 1] - corridor.points[0]).length();
                assert!(
                    net > 1.0,
                    "seed {seed}: string {i} ends up {net} from where it started, coiled in place",
                );
            }
        }
    }

    /// The point of several strings is that they run through *different* parts
    /// of the structure. If the traces converge on one tunnel the strings
    /// overlap and read as a single thick one, which is the same picture as
    /// before but more expensive to draw.
    #[test]
    fn strings_take_different_tunnels() {
        // The curl winds out to about this much either side of the corridor,
        // so two corridors closer than twice it have overlapping strings.
        let touching = 0.24;

        for seed in 0..16 {
            let a = seed as f32 * 2.399_963;
            let origin = Vec3::new(a.sin() * 3.0, (a * 1.7).cos() * 3.0, (a * 0.3).sin() * 3.0);
            let heading =
                Vec3::new((a * 0.7).cos(), (a * 1.3).sin() * 0.5, (a * 0.7).sin()).normalize();
            let across = heading.cross(Vec3::Y).normalize();
            let above = across.cross(heading).normalize();

            let mut bundle = std::array::from_fn(|_| Corridor::default());
            trace_bundle(origin, heading, across, above, &mut bundle);

            for i in 0..STRINGS {
                for j in i + 1..STRINGS {
                    // Compared at equal arc length: the strings advance
                    // together, so this is where beads are actually
                    // neighbours. Nearest-point distance would instead flag a
                    // crossing that the two never occupy at the same time.
                    let apart = bundle[i]
                        .points
                        .iter()
                        .zip(bundle[j].points.iter())
                        .filter(|(a, b)| (**a - **b).length() < touching)
                        .count();
                    assert!(
                        apart * 8 < TRACK_POINTS,
                        "seed {seed}: strings {i} and {j} share {apart}/{TRACK_POINTS} of their length",
                    );
                }
            }
        }
    }
}
