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

/// Walk a point towards more open space, up to `margin`.
///
/// Gradient ascent on the distance estimate: the gradient points away from the
/// nearest surface, so following it uphill lands in the middle of whatever
/// pocket the point started in. It stops as soon as the margin is met, when
/// the field stops improving, or after a fixed budget — a point in a pocket
/// too tight to satisfy `margin` settles at the best spot it found rather than
/// wandering off looking for a better one.
pub fn push_clear(point: Vec3, margin: f32) -> Vec3 {
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

/// The corridor is a single line — one string of beads through the structure,
/// not a braid of them.
pub const TRACKS: usize = 1;
/// Points along it. Fine spacing keeps the curve smooth where it bends around
/// a pillar; the shader interpolates between them.
pub const TRACK_POINTS: usize = 192;
/// Arc length between consecutive points, in world units. The full corridor is
/// therefore (TRACK_POINTS - 1) * TRACK_STEP long.
pub const TRACK_STEP: f32 = 0.16;

/// How much of the previous heading is kept at each step. High, so the line
/// sweeps through long curves instead of reacting to every pocket it passes.
const MOMENTUM: f32 = 0.82;

/// A traced corridor: where the line goes, and a frame to wind the curl around.
pub struct Corridor {
    pub points: [Vec3; TRACK_POINTS],
    /// Unit vector perpendicular to the tangent at each point, carried along
    /// the curve by parallel transport.
    pub normals: [Vec3; TRACK_POINTS],
}

impl Default for Corridor {
    fn default() -> Self {
        Self {
            points: [Vec3::ZERO; TRACK_POINTS],
            normals: [Vec3::X; TRACK_POINTS],
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
    let mut walk = [Vec3::ZERO; TRACK_POINTS];
    let mut p = push_clear(start, TRACK_CLEARANCE);
    let mut dir = heading.normalize_or_zero();
    if dir == Vec3::ZERO {
        dir = Vec3::Z;
    }

    for slot in walk.iter_mut() {
        *slot = p;

        let probe = push_clear(p + dir * TRACK_STEP, TRACK_CLEARANCE);
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
}

/// Two passes of a three-point average. The correction is not continuous —
/// neighbouring points can settle into pockets a little off from each other —
/// and unsmoothed that shows up as the string twitching bead to bead.
fn smooth(points: &mut [Vec3; TRACK_POINTS]) {
    for _ in 0..2 {
        let source = *points;
        for i in 1..TRACK_POINTS - 1 {
            points[i] = (source[i - 1] + source[i] * 2.0 + source[i + 1]) * 0.25;
        }
    }
}

/// Resample a polyline to uniform arc length, so index maps to distance.
fn resample(walk: &[Vec3; TRACK_POINTS], out: &mut [Vec3; TRACK_POINTS]) {
    let mut arc = [0.0f32; TRACK_POINTS];
    for i in 1..TRACK_POINTS {
        arc[i] = arc[i - 1] + (walk[i] - walk[i - 1]).length();
    }
    let total = arc[TRACK_POINTS - 1];
    if total <= 1.0e-4 {
        *out = *walk;
        return;
    }

    // The walk is at least as long as the corridor we want, since the forward
    // step is TRACK_STEP before any sideways correction is added.
    let wanted = (TRACK_POINTS - 1) as f32 * TRACK_STEP;
    let mut cursor = 1;
    for (i, slot) in out.iter_mut().enumerate() {
        let target = (i as f32 * TRACK_STEP).min(total.min(wanted));
        while cursor < TRACK_POINTS - 1 && arc[cursor] < target {
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

            // And it must not flip: neighbouring frames turn by the small
            // angle between neighbouring tangents, never by half a turn.
            for pair in corridor.normals.windows(2) {
                assert!(pair[0].dot(pair[1]) > 0.9, "frame flipped");
            }
        }
    }
}
