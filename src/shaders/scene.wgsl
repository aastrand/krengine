// Pass 1 — the liquid-metal blob against the room. Outputs HDR colour, and
// writes depth from the hit distance so rasterized geometry sorts against it.

struct SceneOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};


struct RayScene {
    color: vec3<f32>,
    depth: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> FullscreenOut {
    return fullscreen_vertex(vi);
}

@fragment
fn fs_main(in: FullscreenOut) -> SceneOut {
    let ro = u.camera_pos.xyz;
    let rd = camera_ray(in.uv);

    // u.intro.z fades the scene up out of black. It is applied here rather
    // than in post, or it would fade the intro text along with the scene.
    let fade = u.intro.z;

    var out: SceneOut;
    out.color = vec4<f32>(environment(rd) * fade, 1.0);
    out.depth = 1.0; // background sits at the far plane

    // Only once the room has actually gone. Swapping at half way meant the
    // collapse was over before it could be seen.
    //
    // This threshold sits inside the white wash's plateau — see WASH_HOLD and
    // WASH_BACK in timeline.rs, which bracket it — so the hard change from one
    // geometry to the other happens on a frame that is solid white. Move it
    // outside that window and the swap becomes a visible cut again.
    if u.collapse.x > 0.9 {
        if u.tunnel.y > 0.999 {
            let tunnel = render_tunnel_scene(ro, rd);
            out.color = vec4<f32>(tunnel.color * fade, 1.0);
            out.depth = tunnel.depth;
            return out;
        }

        if u.tunnel.x > 0.0 {
            let lenses = render_lens_scene(ro, rd);
            let tunnel = render_tunnel_scene(ro, rd);
            let cover = 4.0 * u.tunnel.x * (1.0 - u.tunnel.x);
            let liquid = vec3<f32>(0.035, 0.028, 0.030)
                + LENS_PEACH * (0.12 + u.audio.x * 0.08);
            let switched = select(lenses.color, tunnel.color, u.tunnel.x >= 0.5);
            out.color = vec4<f32>(mix(switched, liquid, cover) * fade, 1.0);
            out.depth = select(lenses.depth, tunnel.depth, u.tunnel.x >= 0.5);
            return out;
        }

        if u.lens.z > 0.999 {
            let lenses = render_lens_scene(ro, rd);
            out.color = vec4<f32>(lenses.color * fade, 1.0);
            out.depth = lenses.depth;
            return out;
        }

        // One circular aperture seals over the fractal, closes over the whole
        // frame, then opens onto the lens field. The scene and camera swap at
        // the fully opaque midpoint; neither is cross-faded while visible.
        let aspect = u.resolution.x / max(u.resolution.y, 1.0);
        let membrane_center = vec2<f32>(0.13, -0.04);
        let q = vec2<f32>((in.uv.x - membrane_center.x) * aspect, in.uv.y - membrane_center.y);
        let radial = length(q);
        let closing = clamp(u.lens.y * 2.0, 0.0, 1.0);
        let opening = clamp((u.lens.y - 0.5) * 2.0, 0.0, 1.0);
        let close_radius = mix(0.58, 2.45, closing * closing * (3.0 - 2.0 * closing));
        let open_radius = mix(0.0, 2.45, opening * opening * (3.0 - 2.0 * opening));
        let closed_disc = 1.0 - smoothstep(close_radius - 0.035, close_radius + 0.035, radial);
        let unrevealed = smoothstep(open_radius - 0.035, open_radius + 0.035, radial);

        var sample_uv = in.uv;
        if closed_disc > 0.0 && radial > 1.0e-4 {
            let bend = (1.0 - clamp(radial / max(close_radius, 0.01), 0.0, 1.0))
                * u.lens.x * (1.0 - closing) * 0.10;
            sample_uv = sample_uv + vec2<f32>(q.x / aspect, q.y) / radial * bend;
        }
        let old = render_fractal_scene(ro, camera_ray(sample_uv));
        let lenses = render_lens_scene(ro, rd);
        let film = mix(LENS_SHADOW, LENS_IVORY, 0.72)
            + LENS_PEACH * (0.08 + u.audio.z * 0.08);

        var color: vec3<f32>;
        if u.lens.y < 0.5 {
            // The sealed aperture begins as refracted fractal and gains body
            // only as it closes, so it reads as one membrane approaching.
            color = mix(old.color, film, closed_disc * closing);
            out.depth = old.depth;
        } else {
            // At the midpoint `unrevealed` covers the complete frame. The new
            // room then opens inside-out without ever sharing visible pixels
            // with the old camera.
            color = mix(lenses.color, film, unrevealed);
            out.depth = select(lenses.depth, 1.0, unrevealed > 0.5);
        }

        let edge_radius = select(close_radius, open_radius, u.lens.y >= 0.5);
        let rim = exp(-abs(radial - edge_radius) * 72.0) * u.lens.x;
        color = color + LENS_PEACH * rim * (0.38 + u.audio.z * 0.24);
        out.color = vec4<f32>(color * fade, 1.0);
        return out;
    }

    let bounds = intersect_sphere(
        ro,
        rd,
        BOUND_RADIUS + SPIKE_BOUND * u.scene.x * (1.0 + u.motion.x * SPIKE_GROWTH),
    );
    if bounds.y < 0.0 {
        return out;
    }

    let t = march_inner(ro, rd, max(bounds.x, 0.0), bounds.y);
    if t < 0.0 {
        return out;
    }

    let hit = ro + rd * t;
    // During the opening cards the body stays entirely black. Only its merge
    // seams are allowed through as a narrow orange HDR signal; bloom turns
    // that signal into the faint glow that originally appeared by accident.
    let title_presence = intro_title_presence();
    let title_seam = pow(inner_field(hit).y, 3.2) * title_presence;
    let title_vein = mix(VEIN_COLOR, VEIN_CORE, title_seam * 0.45)
        * title_seam
        * (0.72 + u.audio.w * 0.18);
    out.color = vec4<f32>(shade_inner(hit, rd) * fade + title_vein, 1.0);
    out.depth = clip_depth(hit);
    return out;
}
