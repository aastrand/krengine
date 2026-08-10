// Fills the emitter buffer the fluid kernels read.
//
// The fluid is a screen-space field, so beads are projected to UV here and the
// per-cell kernels just read positions. Doing this per cell would mean
// evaluating the particle curves half a million times a frame.

@group(1) @binding(0) var<storage, read_write> emitter_out: array<vec4<f32>>;

/// World point to UV, or a far-away point when it lands behind the camera.
fn project(p: vec3<f32>) -> vec2<f32> {
    let clip = u.view_proj * vec4<f32>(p, 1.0);
    if clip.w <= 0.0 {
        return vec2<f32>(-10.0);
    }
    let ndc = clip.xy / clip.w;
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= arrayLength(&emitter_out) {
        return;
    }

    // Stride through the swarm rather than taking the first N, which would all
    // sit on the same curve.
    let bead = i * 7u;
    let dt = 0.03;
    let here = project(particle_pos(bead, u.time));
    let before = project(particle_pos(bead, u.time - dt));

    // zw is screen-space velocity: what the bead drags the fluid with.
    emitter_out[i] = vec4<f32>(here, (here - before) / dt);
}
