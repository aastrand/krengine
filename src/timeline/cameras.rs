//! Camera rigs specific to the living-lens and tunnel sections.
//!
//! Edit selection and persistent smoothing state remain in `Director`; this
//! module owns the geometric rails, clearance constraints, and focus targets.

use super::*;

/// A low aerial track over the cube field. The diagonal view exposes both the
/// collective wave and each cube's individual ballistic arc.
pub(super) fn cube_camera(music: &Sync) -> Camera {
    let since = (music.beat_phase - CUBE_TRANSITION_END_BEATS).max(0.0);
    let z = -since * 0.34;
    let eye = Vec3::new((since * 0.045).sin() * 2.2, 5.4, z + 7.5);
    Camera {
        eye,
        target: Vec3::new((since * 0.045 + 0.45).sin() * 1.6, 0.5, z - 5.0),
        up: Vec3::Y,
        fov_degrees: 58.0,
        focus_distance: 9.0,
    }
}

/// A steady forward march down the tunnel. Broad lateral drift keeps the wall
/// relief moving in parallax; a restrained roll makes the bore feel endless
/// without turning the camera into another orbit shot.
pub(super) fn tunnel_camera(music: &Sync) -> Camera {
    let since = (music.beat_phase - TUNNEL_BEATS - TUNNEL_TRANSITION_BEATS).max(0.0);
    let travel = since * 0.48;
    let tunnel_center = |z: f32| Vec3::new((z * 0.24).sin() * 0.38, (z * 0.19).cos() * 0.30, z);
    let eye = tunnel_center(-travel);
    let target = tunnel_center(-travel - 4.0);
    let forward = (target - eye).normalize_or_zero();
    // Follow the tunnel's rotating three-lobed cross-section rather than
    // banking independently of it.
    let roll = -travel * 0.18 + (-travel * 0.12).sin() * 0.38;
    let world_up = Vec3::Y;
    let up = world_up * roll.cos()
        + forward.cross(world_up) * roll.sin()
        + forward * forward.dot(world_up) * (1.0 - roll.cos());

    // Rack toward the nearest tentacle while it is pushing out of the wall.
    // The gate timing mirrors scene.wgsl; keeping this authored instead of
    // sampling scene depth prevents the reflective wall from stealing focus.
    let spacing = 5.2;
    let first_gate = (travel / spacing).floor() + 1.0;
    let mut focus_distance = 3.8;
    let mut best_entrance = 0.0;
    for i in 0..3 {
        let fi = i as f32;
        let cycle = ((since + fi * 1.65) / 8.0).fract();
        let entrance = smoothstep(0.02, 0.16, cycle) * (1.0 - smoothstep(0.30, 0.52, cycle))
            / (1.0 + fi * 0.35);
        if entrance > best_entrance {
            let gate_z = -(first_gate + fi) * spacing - 0.48;
            // Tentacles cross close to the bore centre; include their small
            // lateral offset and ignore entrances too near the lens to frame.
            focus_distance = ((eye.z - gate_z).powi(2) + 0.55).sqrt().clamp(2.0, 7.0);
            best_entrance = entrance;
        }
    }
    Camera {
        eye,
        target,
        up: up.normalize_or_zero(),
        fov_degrees: 72.0,
        focus_distance,
    }
}

/// A single continuous flight beside and between the membranes. The eye follows
/// the authored rail, while the view direction chooses the richest cluster it
/// can see without turning needlessly far away from the rail's tangent.
pub(super) fn lens_camera(music: &Sync) -> Camera {
    let since = (music.beat_phase - LENS_BEATS - LENS_TRANSITION_BEATS).max(0.0);
    let phase = (since / LENS_FLIGHT_BEATS).fract();
    let eye = clear_lens_camera(lens_flight_point(phase), music);
    // Never project the look-ahead point out of a membrane: doing so changes
    // the camera's angle abruptly even when the eye itself moves smoothly.
    let path_forward = (lens_flight_point((phase + 0.035).fract()) - eye).normalize_or_zero();
    let forward = lens_view_direction(eye, path_forward);
    let target = eye + forward * 4.0;

    // A slow full roll over the take, plus a smaller bank following the broad
    // turns. Rodrigues' formula rotates world-up around the viewing axis.
    let roll = phase * std::f32::consts::TAU + (phase * std::f32::consts::TAU * 2.0).sin() * 0.22;
    let world_up = Vec3::Y;
    let up = world_up * roll.cos()
        + forward.cross(world_up) * roll.sin()
        + forward * forward.dot(world_up) * (1.0 - roll.cos());

    Camera {
        eye,
        target,
        up: up.normalize_or_zero(),
        fov_degrees: LENS_FOV_DEGREES,
        focus_distance: 4.0,
    }
}

/// Pick one comfortably framed membrane at the start of a focus phrase.
/// Candidates are ordered near, far, second-near, second-far, so successive
/// pulls traverse an unmistakable amount of depth instead of landing on two
/// lenses that happen to be almost coplanar.
pub(super) fn choose_lens_focus(eye: Vec3, forward: Vec3, music: &Sync, phrase: usize) -> usize {
    let half_fov = (LENS_FOV_DEGREES * 0.5).to_radians();
    let mut safe = Vec::with_capacity(LENS_CENTERS.len());
    let mut onscreen = Vec::with_capacity(LENS_CENTERS.len());
    let mut fallback = (0usize, f32::INFINITY);
    for i in 0..LENS_CENTERS.len() {
        let center = animated_lens_center(i, music);
        let delta = center - eye;
        let distance = delta.length().max(1.0e-4);
        let direction = delta / distance;
        let facing = forward.dot(direction).clamp(-1.0, 1.0);
        if facing <= 0.0 {
            continue;
        }
        let direction = (eye - center).normalize_or_zero();
        let radius = animated_lens_radius(direction, i, music);
        let surface_distance = (distance - radius).max(0.35);
        let angle = facing.acos();
        let angular_radius = (radius / distance).clamp(0.0, 0.999).asin();
        if angle <= half_fov + angular_radius {
            onscreen.push((i, surface_distance));
        }
        // Keep the selected subject away from the edge so the pull is always
        // readable as landing on something, not on an object leaving frame.
        if angle + angular_radius * 0.35 < half_fov * 0.88 {
            safe.push((i, surface_distance));
        }
        if angle < fallback.1 {
            fallback = (i, angle);
        }
    }

    let visible = if safe.is_empty() {
        &mut onscreen
    } else {
        &mut safe
    };
    if visible.is_empty() {
        return fallback.0;
    }
    visible.sort_by(|a, b| a.1.total_cmp(&b.1));
    let rank = phrase % visible.len();
    let slot = if rank.is_multiple_of(2) {
        rank / 2
    } else {
        visible.len() - 1 - rank / 2
    };
    visible[slot].0
}

pub(super) fn lens_is_visible(eye: Vec3, forward: Vec3, lens: usize, music: &Sync) -> bool {
    let center = animated_lens_center(lens, music);
    let delta = center - eye;
    let distance = delta.length().max(1.0e-4);
    let direction = delta / distance;
    let radius = animated_lens_radius(-direction, lens, music);
    let angular_radius = (radius / distance).clamp(0.0, 0.999).asin();
    let angle = forward.dot(direction).clamp(-1.0, 1.0).acos();
    angle <= (LENS_FOV_DEGREES * 0.5).to_radians() + angular_radius
}

/// Stable nominal front-surface distance for the chosen membrane. Audio still
/// morphs its rendering, but cannot make the focus ring breathe between the
/// scheduled lens-to-lens pulls.
pub(super) fn lens_focus_distance(eye: Vec3, lens: usize) -> f32 {
    (eye.distance(LENS_CENTERS[lens]) - LENS_RADII[lens]).max(0.35)
}

/// Catmull-Rom's parameter is not distance: using it directly makes the camera
/// accelerate near some control points and brake near others. Re-map the phase
/// through a small arc-length table so the eye glides at an even speed.
pub(super) fn lens_flight_point(phase: f32) -> Vec3 {
    const STEPS: usize = 128;
    let target_fraction = phase.fract();
    let mut lengths = [0.0f32; STEPS + 1];
    let mut previous = spline(&LENS_FLIGHT, 0.0);
    for step in 1..=STEPS {
        let point = spline(&LENS_FLIGHT, step as f32 / STEPS as f32);
        lengths[step] = lengths[step - 1] + point.distance(previous);
        previous = point;
    }

    let target = lengths[STEPS] * target_fraction;
    let upper = lengths
        .partition_point(|length| *length < target)
        .clamp(1, STEPS);
    let lower = upper - 1;
    let span = (lengths[upper] - lengths[lower]).max(1.0e-5);
    let local = (target - lengths[lower]) / span;
    let spline_phase = (lower as f32 + local) / STEPS as f32;
    spline(&LENS_FLIGHT, spline_phase)
}

/// Aim through the apparent centre of the whole membrane cluster. Giving every
/// membrane a base vote optimises for quantity, while its angular size adds
/// enough weight that a nearby lens cannot be abandoned at the edge of frame.
/// Unlike choosing a winning lens or pair, this mean moves continuously when
/// two possible framings trade places.
fn lens_view_direction(eye: Vec3, path_forward: Vec3) -> Vec3 {
    let mut cluster = path_forward * 0.55;
    for i in 0..LENS_CENTERS.len() {
        // Frame the stable underlying layout. Letting audio deformation alter
        // these weights made the camera nod and wobble along with every blob.
        let delta = LENS_CENTERS[i] - eye;
        let distance = delta.length().max(1.0e-4);
        let direction = delta / distance;
        let apparent_size = (LENS_RADII[i] / distance).clamp(0.0, 0.999).asin();
        cluster += direction * (0.42 + apparent_size);
    }
    cluster.normalize_or_zero()
}

/// Give the virtual camera rig some mass. If a leisurely interpolation would
/// temporarily leave every membrane outside the frame, project onto the
/// nearest valid framing edge instead of snapping to a new subject.
pub(super) fn stabilize_lens_view(eye: Vec3, previous: Vec3, desired: Vec3, music: &Sync) -> Vec3 {
    let base_alpha = 1.0 - (-music.dt.max(0.0) / 0.72).exp();
    let candidate = previous
        .lerp(desired, base_alpha.max(0.001))
        .normalize_or_zero();
    if lens_visibility_score(eye, candidate, music).0 > 0 {
        return candidate;
    }

    // Project onto the nearest valid framing cone. This is the smallest turn
    // that restores a membrane, unlike jumping all the way to `desired`.
    let half_fov = (LENS_FOV_DEGREES * 0.5).to_radians();
    let mut anchor = Vec3::ZERO;
    let mut nearest_edge = f32::INFINITY;
    for i in 0..LENS_CENTERS.len() {
        let center = animated_lens_center(i, music);
        let delta = center - eye;
        let distance = delta.length().max(1.0e-4);
        let direction = delta / distance;
        let radius = animated_lens_radius(-direction, i, music);
        let allowance = half_fov + (radius / distance).clamp(0.0, 0.999).asin();
        let edge = candidate.dot(direction).clamp(-1.0, 1.0).acos() - allowance;
        if edge < nearest_edge {
            nearest_edge = edge;
            anchor = direction;
        }
    }

    let mut outside = 0.0f32;
    let mut inside = 1.0f32;
    for _ in 0..14 {
        let middle = (outside + inside) * 0.5;
        let probe = candidate.lerp(anchor, middle).normalize_or_zero();
        if lens_visibility_score(eye, probe, music).0 > 0 {
            inside = middle;
        } else {
            outside = middle;
        }
    }
    candidate.lerp(anchor, inside).normalize_or_zero()
}

pub(super) fn lens_visibility_score(eye: Vec3, forward: Vec3, music: &Sync) -> (usize, f32) {
    let half_fov = (LENS_FOV_DEGREES * 0.5).to_radians();
    let mut visible = 0usize;
    let mut margin = 0.0f32;
    for i in 0..LENS_CENTERS.len() {
        let center = animated_lens_center(i, music);
        let delta = center - eye;
        let distance = delta.length().max(1.0e-4);
        let direction = delta / distance;
        let radius = animated_lens_radius(-direction, i, music);
        let angular_radius = (radius / distance).clamp(0.0, 0.999).asin();
        let allowance = half_fov + angular_radius;
        let angle = forward.dot(direction).clamp(-1.0, 1.0).acos();
        if angle <= allowance {
            visible += 1;
            margin += allowance - angle;
        }
    }
    (visible, margin)
}

pub(super) fn animated_lens_center(i: usize, music: &Sync) -> Vec3 {
    let fi = i as f32;
    let drift = Vec3::new(
        (music.time * 0.13 + fi * 1.9).sin(),
        (music.time * 0.11 + fi * 2.3).cos(),
        (music.time * 0.09 + fi * 0.7).sin(),
    ) * (0.025 + music.low * 0.035);
    LENS_CENTERS[i] + drift
}

/// CPU copy of the shader's directional radius, used only to keep the camera
/// outside the live surface. Keeping the formulas identical is what makes the
/// guarantee hold while the bass and mids reshape the membranes.
pub(super) fn animated_lens_radius(direction: Vec3, i: usize, music: &Sync) -> f32 {
    let fi = i as f32;
    let broad = ((direction.x * 2.7 + direction.y * 1.3 + music.time * 0.23 + fi).sin()
        + (direction.y * 3.1 - direction.z * 1.7 - music.time * 0.19 + fi * 1.9).sin()
        + (direction.z * 2.4 + direction.x * 1.5 + music.time * 0.17 + fi * 2.7).sin())
        / 3.0;
    let fold_axis = Vec3::new(
        (fi * 1.7).sin() + 0.3,
        (fi * 2.1).cos(),
        (fi * 0.8).sin() + 0.2,
    )
    .normalize_or_zero();
    let folds = (direction.dot(fold_axis) * 6.0 + music.time * 0.31 + fi * 2.2).sin();
    let travel_axis = Vec3::new(0.7, 0.25, -0.45).normalize();
    let travelling = (direction.dot(travel_axis) * 10.0 - music.beat_phase * std::f32::consts::TAU
        + fi * 1.4)
        .sin();
    let deformation = broad * (0.145 + music.low * 0.08)
        + folds * 0.045
        + travelling * (0.020 + music.mid * 0.060);
    LENS_RADII[i] * (1.0 + deformation)
}

fn clear_lens_camera(point: Vec3, music: &Sync) -> Vec3 {
    let mut result = point;
    // Resolve only the deepest overlap at each iteration. Moving through every
    // lens in sequence can oscillate at an overlap seam; deepest-first follows
    // the shortest route out of the union and keeps the resulting rail tight.
    for _ in 0..96 {
        let mut correction = Vec3::ZERO;
        let mut deepest = 0.0f32;
        for i in 0..LENS_CENTERS.len() {
            let center = animated_lens_center(i, music);
            let delta = result - center;
            let distance = delta.length().max(1.0e-5);
            let direction = delta / distance;
            let boundary = animated_lens_radius(direction, i, music) + LENS_CAMERA_CLEARANCE;
            let penetration = boundary - distance;
            if penetration > deepest {
                deepest = penetration;
                correction = direction * (penetration + 0.003);
            }
        }
        if deepest <= 0.0 {
            break;
        }
        result += correction;
    }
    result
}
