// Eulerian fluid on a 2D grid, in screen space.
//
// 2D rather than 3D on purpose. Fine vortex filaments need hundreds of cells
// across the domain, and in 3D that cost is cubic: a 64^3 grid is only 64 cells
// per axis, which smears every filament flat within a few frames. The same
// budget buys a thousand cells per axis in 2D, which is what makes the
// structure visible at all.
//
// Per frame: advect velocity through itself, inject where the beads project
// onto the screen, restore curl with vorticity confinement, make the field
// divergence-free with a Jacobi pressure solve, then advect dye through it.

const GRID: vec2<f32> = vec2<f32>(1024.0, 576.0);

/// How strongly a bead's own motion is imposed on the fluid touching it.
/// This is a constraint, not a force: near the bead the fluid simply moves
/// with it, the way liquid does against a dragged solid. Injecting velocity
/// instead makes a jet, which billows rather than shedding a wake.
const COUPLING: f32 = 82.0;
/// How much dye a bead sheds.
const EMISSION: f32 = 1.15;
/// Bead influence radius, in UV units.
const EMITTER_RADIUS: f32 = 0.018;
const EMITTER_CUTOFF: f32 = 0.005;

const VELOCITY_DAMPING: f32 = 0.16;
const DYE_DISSIPATION: f32 = 0.28;
/// Vorticity confinement: pushes the field back toward its own vortices.
/// This is what keeps small eddies alive instead of letting numerical
/// diffusion eat them, and the filaments come from it as much as from
/// the grid resolution.
const VORTICITY: f32 = 24.0;

@group(1) @binding(0) var lin: sampler;
@group(1) @binding(1) var tex0: texture_2d<f32>;
@group(1) @binding(2) var tex1: texture_2d<f32>;
@group(1) @binding(3) var out_vec: texture_storage_2d<rgba16float, write>;
@group(1) @binding(4) var out_scalar: texture_storage_2d<rgba16float, write>;
@group(1) @binding(5) var<storage, read> emitters: array<vec4<f32>>;

fn cell_to_uv(cell: vec2<u32>) -> vec2<f32> {
    return (vec2<f32>(cell) + 0.5) / GRID;
}

/// Keeps round things round on a non-square grid.
fn aspect_scale() -> vec2<f32> {
    return vec2<f32>(GRID.x / GRID.y, 1.0);
}

fn velocity_at(uv: vec2<f32>) -> vec2<f32> {
    return textureSampleLevel(tex0, lin, uv, 0.0).xy;
}

@compute @workgroup_size(8, 8)
fn cs_advect_velocity(@builtin(global_invocation_id) id: vec3<u32>) {
    let uv = cell_to_uv(id.xy);
    let dt = u.frame.x;

    // Semi-Lagrangian: trace backwards to find what arrives here. Stable at
    // any timestep, which is why every real-time fluid uses it.
    let back = uv - velocity_at(uv) * dt;
    var result = textureSampleLevel(tex0, lin, back, 0.0).xy;

    result *= exp(-VELOCITY_DAMPING * dt);

    let count = arrayLength(&emitters);
    for (var i = 0u; i < count; i = i + 1u) {
        let emitter = emitters[i];
        let offset = (uv - emitter.xy) * aspect_scale();
        let squared = dot(offset, offset);
        if squared < EMITTER_CUTOFF {
            // zw is the bead's screen-space velocity. Blend the fluid toward
            // it: inside the bead the fluid matches it exactly, and the
            // pressure solve turns that into flow around the obstacle and a
            // vortex street behind it.
            let falloff = exp(-squared / (EMITTER_RADIUS * EMITTER_RADIUS));
            let grip = clamp(falloff * COUPLING * dt, 0.0, 1.0);
            result = mix(result, emitter.zw, grip);
        }
    }

    textureStore(out_vec, id.xy, vec4<f32>(result, 0.0, 0.0));
}

/// Curl of the velocity field. A single scalar in 2D, which is exactly why
/// vorticity confinement is affordable here and was not in 3D.
fn curl_at(uv: vec2<f32>, h: vec2<f32>) -> f32 {
    let right = textureSampleLevel(tex0, lin, uv + vec2<f32>(h.x, 0.0), 0.0).y;
    let left = textureSampleLevel(tex0, lin, uv - vec2<f32>(h.x, 0.0), 0.0).y;
    let up = textureSampleLevel(tex0, lin, uv + vec2<f32>(0.0, h.y), 0.0).x;
    let down = textureSampleLevel(tex0, lin, uv - vec2<f32>(0.0, h.y), 0.0).x;
    return (right - left) - (up - down);
}

@compute @workgroup_size(8, 8)
fn cs_vorticity(@builtin(global_invocation_id) id: vec3<u32>) {
    let uv = cell_to_uv(id.xy);
    let h = 1.0 / GRID;

    let curl = curl_at(uv, h);
    let gradient = vec2<f32>(
        abs(curl_at(uv + vec2<f32>(h.x, 0.0), h)) - abs(curl_at(uv - vec2<f32>(h.x, 0.0), h)),
        abs(curl_at(uv + vec2<f32>(0.0, h.y), h)) - abs(curl_at(uv - vec2<f32>(0.0, h.y), h)),
    );

    // Perpendicular to the gradient, signed by the curl: a force that spins.
    let n = gradient / (length(gradient) + 1.0e-6);
    let force = vec2<f32>(n.y, -n.x) * curl * VORTICITY;

    textureStore(out_vec, id.xy, vec4<f32>(velocity_at(uv) + force * u.frame.x, 0.0, 0.0));
}

@compute @workgroup_size(8, 8)
fn cs_divergence(@builtin(global_invocation_id) id: vec3<u32>) {
    let uv = cell_to_uv(id.xy);
    let h = 1.0 / GRID;

    let right = textureSampleLevel(tex0, lin, uv + vec2<f32>(h.x, 0.0), 0.0).x;
    let left = textureSampleLevel(tex0, lin, uv - vec2<f32>(h.x, 0.0), 0.0).x;
    let up = textureSampleLevel(tex0, lin, uv + vec2<f32>(0.0, h.y), 0.0).y;
    let down = textureSampleLevel(tex0, lin, uv - vec2<f32>(0.0, h.y), 0.0).y;

    let divergence = 0.5 * ((right - left) + (up - down));
    textureStore(out_scalar, id.xy, vec4<f32>(divergence, 0.0, 0.0, 0.0));
}

@compute @workgroup_size(8, 8)
fn cs_jacobi(@builtin(global_invocation_id) id: vec3<u32>) {
    let uv = cell_to_uv(id.xy);
    let h = 1.0 / GRID;

    var sum = textureSampleLevel(tex0, lin, uv + vec2<f32>(h.x, 0.0), 0.0).x;
    sum += textureSampleLevel(tex0, lin, uv - vec2<f32>(h.x, 0.0), 0.0).x;
    sum += textureSampleLevel(tex0, lin, uv + vec2<f32>(0.0, h.y), 0.0).x;
    sum += textureSampleLevel(tex0, lin, uv - vec2<f32>(0.0, h.y), 0.0).x;

    let divergence = textureSampleLevel(tex1, lin, uv, 0.0).x;
    textureStore(out_scalar, id.xy, vec4<f32>((sum - divergence) * 0.25, 0.0, 0.0, 0.0));
}

@compute @workgroup_size(8, 8)
fn cs_project(@builtin(global_invocation_id) id: vec3<u32>) {
    let uv = cell_to_uv(id.xy);
    let h = 1.0 / GRID;

    let px = textureSampleLevel(tex1, lin, uv + vec2<f32>(h.x, 0.0), 0.0).x
        - textureSampleLevel(tex1, lin, uv - vec2<f32>(h.x, 0.0), 0.0).x;
    let py = textureSampleLevel(tex1, lin, uv + vec2<f32>(0.0, h.y), 0.0).x
        - textureSampleLevel(tex1, lin, uv - vec2<f32>(0.0, h.y), 0.0).x;

    var velocity = velocity_at(uv) - vec2<f32>(px, py) * 0.5;

    // Nothing flows through the edges of the domain.
    let edge = smoothstep(0.0, 0.012, min(uv.x, uv.y))
        * smoothstep(0.0, 0.012, min(1.0 - uv.x, 1.0 - uv.y));

    textureStore(out_vec, id.xy, vec4<f32>(velocity * edge, 0.0, 0.0));
}

@compute @workgroup_size(8, 8)
fn cs_advect_dye(@builtin(global_invocation_id) id: vec3<u32>) {
    let uv = cell_to_uv(id.xy);
    let dt = u.frame.x;

    // tex1 is the projected velocity, tex0 the dye.
    let velocity = textureSampleLevel(tex1, lin, uv, 0.0).xy;
    var dye = textureSampleLevel(tex0, lin, uv - velocity * dt, 0.0).x;

    dye *= exp(-DYE_DISSIPATION * dt);

    let count = arrayLength(&emitters);
    for (var i = 0u; i < count; i = i + 1u) {
        let emitter = emitters[i];
        let offset = (uv - emitter.xy) * aspect_scale();
        let squared = dot(offset, offset);
        if squared < EMITTER_CUTOFF {
            dye += exp(-squared / (EMITTER_RADIUS * EMITTER_RADIUS)) * EMISSION * dt;
        }
    }

    textureStore(out_scalar, id.xy, vec4<f32>(min(dye, 1.5), 0.0, 0.0, 0.0));
}
