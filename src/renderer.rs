use glam::camera::rh::{proj::directx, view};
use glam::{Vec3, Vec4};

/// What happened when we tried to draw a frame.
pub enum Frame {
    Rendered,
    /// Swapchain is stale (resize, monitor change) — reconfigure and try again.
    Reconfigure,
    /// Nothing to do this tick (window occluded, acquire timed out).
    Skipped,
}

use crate::audio::Sync;
use crate::gpu::Gpu;
use crate::passes::bloom::{BloomPass, BloomTargets};
use crate::passes::text::TextPass;
use crate::passes::{
    DEPTH_FORMAT, HDR_FORMAT, particles::ParticlePass, post::PostPass, scene::ScenePass,
};
use crate::timeline::{Camera, Director, Flow, Spin, Stage};

/// Every card: three intro titles, one attribution, six fractal greetings,
/// then two outro cards.
const CARDS: [&str; 12] = [
    "smeuch",
    "is back",
    "2026",
    "a demo by kranken",
    "spinax",
    "zantac",
    "whodini",
    "antimedel",
    "lixus",
    "gammawave",
    "smeuch",
    "2026",
];

/// Supersampling factor, over the *logical* window. The scene renders at this
/// multiple of it and the post pass filters it down — with a linear sampler, 2x
/// over a 1x display is an exact 2x2 box per output pixel. Simple, and no
/// ghosting the way reprojection has.
///
/// Logical, not physical, deliberately. The surface is in physical pixels, so
/// on a Retina display multiplying *that* by 2 raymarches 5120x2880 for a
/// 1280x720 window — four times the pixels of the same demo on an ordinary
/// monitor, at no visible benefit, because the window is 720p either way. The
/// scene pass is ~95% of the frame and scales linearly with pixel count, so
/// that difference alone is the gap between a machine running the demo and not.
/// Sizing off the logical window makes the cost, and the image, the same
/// everywhere.
///
/// `KR_SCALE` overrides the factor for machines that cannot pay even this.
const RENDER_SCALE: f32 = 2.0;

fn render_scale() -> f32 {
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| {
        std::env::var("KR_SCALE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| *v > 0.0)
            .unwrap_or(RENDER_SCALE)
    })
}

pub const PARTICLE_COUNT: u32 = 512;
/// Beads in the fractal scene, split between the strings. This keeps each of
/// the twelve corridors visibly populated, filling the architecture with a
/// field of threads rather than a small bundle.
const FRACTAL_BEADS: u32 = 1152;
/// Sparse droplets for the lens field. More makes circular halos around every
/// membrane, which reads as a diagram of planets rather than suspended fluid.
const LENS_PARTICLES: u32 = 420;
/// Four restrained impact motes across each of eighty nearby cube cells.
const CUBE_PARTICLES: u32 = 320;

/// Mirrors `Uniforms` in shaders/common.wgsl. Keep the field order in sync.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    camera_right: [f32; 4],
    camera_up: [f32; 4],
    resolution: [f32; 2],
    time: f32,
    particle_count: f32,
    audio: [f32; 4],
    music: [f32; 4],
    /// The FFT spectrum, packed four bands to a vec4 for std140 alignment.
    bands: [[f32; 4]; crate::audio::BAND_COUNT / 4],
    debug: [f32; 4],
    frame: [f32; 4],
    /// (card index, card opacity, scene fade, drift).
    intro: [f32; 4],
    card: [f32; 4],
    scene: [f32; 4],
    /// (merge, yaw, tilt, unused).
    motion: [f32; 4],
    /// (aperture seal, membrane crossing, lens field, satellite release).
    lens: [f32; 4],
    /// (covered transition, tunnel field, tentacle growth, travel in beats).
    tunnel: [f32; 4],
    /// (covered transition, cube field, gravity, beats in scene).
    cubes: [f32; 4],
    /// (cube fade to black, remaining cube-wave glow, unused, unused).
    outro: [f32; 4],
    /// (collapse, unused, unused, unused).
    collapse: [f32; 4],
    /// (focus distance, aperture strength, unused, unused).
    dof: [f32; 4],
    /// The traced corridor the bead string runs along, as world positions.
    track: [[f32; 4]; crate::fractal::STRINGS * crate::fractal::TRACK_POINTS],
    /// A perpendicular at each of those points, carried along the curve, for
    /// the curl to wind around.
    track_frame: [[f32; 4]; crate::fractal::STRINGS * crate::fractal::TRACK_POINTS],
}

/// Offscreen render targets, rebuilt whenever the window resizes.
struct Targets {
    hdr: wgpu::TextureView,
    depth: wgpu::TextureView,
}

impl Targets {
    /// `width`/`height` are the surface's physical pixels. The scene is
    /// allocated supersampled over the logical window — see `RENDER_SCALE` —
    /// while bloom and the swapchain stay at the surface's own size.
    fn new(device: &wgpu::Device, width: u32, height: u32, dpi: f32) -> Self {
        let scale = render_scale() / dpi.max(1.0);
        let width = ((width as f32 * scale) as u32).max(1);
        let height = ((height as f32 * scale) as u32).max(1);
        log::info!(
            "scene target: {width}x{height} ({:.1} Mpx) at {}x over a {dpi}x display",
            (width as f64 * height as f64) / 1.0e6,
            render_scale(),
        );
        let make = |label, format, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };

        Self {
            hdr: make(
                "hdr target",
                HDR_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
            depth: make(
                "depth buffer",
                DEPTH_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
        }
    }
}

pub struct Renderer {
    uniform_buf: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,

    scene: ScenePass,
    particles: ParticlePass,
    bloom: BloomPass,
    text: TextPass,
    post: PostPass,

    targets: Targets,
    bloom_targets: BloomTargets,
    hdr_bind_group: wgpu::BindGroup,
    /// Draws the spectrum as bars over the frame, for picking a band by eye.
    pub show_bands: bool,
    director: Director,
    spin: Spin,
    flow: Flow,
    focus_distance: f32,
    focus_initialized: bool,
}

impl Renderer {
    pub fn new(gpu: &Gpu) -> Self {
        let device = &gpu.device;

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniform layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniform bind group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });

        let scene = ScenePass::new(device, &uniform_layout);

        let particles = ParticlePass::new(device, &uniform_layout);
        let bloom = BloomPass::new(device, &uniform_layout);
        let text = TextPass::new(device, &gpu.queue, &uniform_layout, &CARDS).expect("font atlas");
        let post = PostPass::new(
            device,
            &uniform_layout,
            bloom.texture_layout(),
            gpu.config.format,
        );

        let targets = Targets::new(
            device,
            gpu.config.width,
            gpu.config.height,
            gpu.scale_factor(),
        );
        let bloom_targets =
            bloom.targets(device, &targets.hdr, gpu.config.width, gpu.config.height);
        let hdr_bind_group = post.make_scene_bind_group(device, &targets.hdr, &targets.depth);

        Self {
            uniform_buf,
            uniform_bind_group,
            scene,
            particles,
            bloom,
            text,
            post,
            targets,
            bloom_targets,
            hdr_bind_group,
            show_bands: false,
            director: Director::default(),
            spin: Spin::default(),
            flow: Flow::default(),
            focus_distance: 4.0,
            focus_initialized: false,
        }
    }

    pub fn resize(&mut self, gpu: &Gpu) {
        self.targets = Targets::new(
            &gpu.device,
            gpu.config.width,
            gpu.config.height,
            gpu.scale_factor(),
        );
        self.bloom_targets = self.bloom.targets(
            &gpu.device,
            &self.targets.hdr,
            gpu.config.width,
            gpu.config.height,
        );
        self.hdr_bind_group =
            self.post
                .make_scene_bind_group(&gpu.device, &self.targets.hdr, &self.targets.depth);
    }

    // Every one of these is a distinct thing the frame needs; bundling them
    // into a struct would only move the list somewhere else.
    #[allow(clippy::too_many_arguments)]
    fn uniforms(
        gpu: &Gpu,
        music: &Sync,
        show_bands: bool,
        stage: &Stage,
        shot: &Camera,
        beads: u32,
        track: [[f32; 4]; crate::fractal::STRINGS * crate::fractal::TRACK_POINTS],
        track_frame: [[f32; 4]; crate::fractal::STRINGS * crate::fractal::TRACK_POINTS],
        spin: &Spin,
        flow: f32,
        along: f32,
        radius: f32,
        focus_distance: f32,
        dof_strength: f32,
    ) -> Uniforms {
        let time = music.time;
        let eye = shot.eye;

        let aspect = gpu.config.width as f32 / gpu.config.height as f32;
        let view = view::look_at_mat4(eye, shot.target, shot.up);
        // directx variant maps depth to 0..1, which is what wgpu expects.
        let proj = directx::perspective(shot.fov_degrees.to_radians(), aspect, 0.05, 100.0);
        let view_proj = proj * view;

        // Billboard basis: rows of the view matrix are the camera's axes.
        let right = Vec3::new(view.x_axis.x, view.y_axis.x, view.z_axis.x);
        let up = Vec3::new(view.x_axis.y, view.y_axis.y, view.z_axis.y);

        Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            camera_pos: Vec4::from((eye, 1.0)).to_array(),
            camera_right: Vec4::from((right, 0.0)).to_array(),
            camera_up: Vec4::from((up, 0.0)).to_array(),
            resolution: [gpu.config.width as f32, gpu.config.height as f32],
            time,
            particle_count: beads as f32,
            audio: [music.low, music.mid, music.high, music.beat],
            music: [
                music.row as f32,
                music.pattern as f32,
                music.beat_phase,
                music.bar_phase,
            ],
            bands: bytemuck::cast(music.bands),
            debug: [
                if show_bands { 1.0 } else { 0.0 },
                // KR_BEADTEST parks the beads in front of the camera, to tell
                // "the pass is not drawing" from "the positions are wrong".
                if std::env::var("KR_BEADTEST").is_ok() {
                    1.0
                } else {
                    0.0
                },
                0.0,
                0.0,
            ],
            // Clamped: a long stall must not blow the simulation up.
            intro: [
                stage.card as f32,
                stage.card_alpha,
                stage.scene,
                stage.scroll,
            ],
            card: [
                stage.scale,
                stage.card_progress,
                stage.card_offset[0],
                stage.card_offset[1],
            ],
            scene: [stage.spike, stage.dissolve, 0.0, 0.0],
            lens: [
                stage.lens_seal,
                stage.lens_cross,
                stage.lens_field,
                stage.lens_particles,
            ],
            tunnel: [
                stage.tunnel_cross,
                stage.tunnel_field,
                stage.tunnel_tentacles,
                stage.tunnel_travel,
            ],
            cubes: [
                stage.cube_cross,
                stage.cube_field,
                stage.cube_gravity,
                stage.cube_travel,
            ],
            outro: [stage.outro_fade, stage.outro_beam, 0.0, 0.0],
            collapse: [stage.collapse, stage.bleed, along, radius],
            dof: [focus_distance, dof_strength, 0.0, 0.0],
            track,
            track_frame,
            motion: [stage.merge, spin.yaw, spin.tilt, stage.palette],
            frame: [music.dt.min(1.0 / 30.0), stage.wash, flow, stage.beads],
        }
    }

    pub fn render(&mut self, gpu: &Gpu, music: &Sync) -> Frame {
        let stage = Stage::at(music);

        let beads = if stage.cube_field > 0.001 {
            CUBE_PARTICLES
        } else if stage.tunnel_field > 0.999 {
            0
        } else if stage.lens_field > 0.999 {
            LENS_PARTICLES
        } else if stage.collapse > 0.85 {
            FRACTAL_BEADS
        } else {
            PARTICLE_COUNT
        };
        let shot = self.director.update(music, &stage);

        // The camera target is the subject selected by the director. A damped
        // physical focus ring follows it: cuts initiate a pull instead of
        // making the focal plane teleport with the camera.
        let desired_focus = shot.focus_distance.clamp(0.35, 14.0);
        let blob_focus_locked = stage.collapse <= 0.85;
        let fractal_focus_locked =
            stage.collapse > 0.85 && stage.lens_field < 0.01 && stage.tunnel_field < 0.01;
        if !self.focus_initialized || blob_focus_locked || fractal_focus_locked {
            // Blob and fractal shots are hard cuts. Arrive with the subject
            // already focused: a delayed rack makes the hero object look like
            // the camera briefly lost it, rather than like an authored pull.
            self.focus_distance = desired_focus;
            self.focus_initialized = true;
        } else {
            // Lens-to-lens pulls are slow enough to be read as an optical
            // gesture. Earlier scenes retain the shorter, subtler response.
            let focus_time = if stage.tunnel_field > 0.01 {
                // Catch a growing tentacle promptly, but ease back to the
                // tunnel's resting plane rather than snapping optically.
                0.30
            } else if stage.lens_field > 0.01 {
                1.05
            } else {
                0.55
            };
            let focus_alpha = 1.0 - (-music.dt.max(0.0) / focus_time).exp();
            self.focus_distance += (desired_focus - self.focus_distance) * focus_alpha;
        }
        let dof_strength = if stage.tunnel_field > 0.01 {
            // Keep the tunnel predominantly crisp. The changing focal plane
            // should gently isolate a tentacle, never turn the bore to fog.
            0.26 * stage.tunnel_field
        } else if stage.lens_field > 0.01 {
            1.16
        } else if stage.collapse > 0.85 {
            // The fractal is dense enough that selective focus reads as a
            // generally blurry image. Keep its architecture and strings
            // uniformly crisp; hard camera cuts provide its depth rhythm.
            0.0
        } else if stage.spike > 0.2 {
            0.84
        } else {
            0.68
        };

        let mut track = [[0.0f32; 4]; crate::fractal::STRINGS * crate::fractal::TRACK_POINTS];
        let mut track_frame = [[0.0f32; 4]; crate::fractal::STRINGS * crate::fractal::TRACK_POINTS];
        // The corridors laid end to end, so the shader indexes one as
        // string * TRACK_POINTS + point.
        let flat = self.director.bundle.iter().flat_map(|c| {
            c.points
                .iter()
                .zip(c.normals.iter())
                .zip(c.clearance.iter())
        });
        for ((slot, frame), ((point, normal), clearance)) in
            track.iter_mut().zip(track_frame.iter_mut()).zip(flat)
        {
            // The clearance rides in the position's spare w: the shader needs
            // it wherever it samples the corridor, and it is one lookup there.
            *slot = [point.x, point.y, point.z, *clearance];
            *frame = [normal.x, normal.y, normal.z, 0.0];
        }
        let spin = self.spin.update(music, &stage);
        let flow = self.flow.swell(music);
        let uniforms = Self::uniforms(
            gpu,
            music,
            self.show_bands,
            &stage,
            &shot,
            beads,
            track,
            track_frame,
            spin,
            flow,
            self.director.along,
            self.director.radius,
            self.focus_distance,
            dof_strength,
        );
        gpu.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        use wgpu::CurrentSurfaceTexture as Acquired;
        let frame = match gpu.surface.get_current_texture() {
            Acquired::Success(t) | Acquired::Suboptimal(t) => t,
            Acquired::Outdated | Acquired::Lost => return Frame::Reconfigure,
            Acquired::Timeout | Acquired::Occluded => return Frame::Skipped,
            other => {
                log::warn!("surface acquire failed: {other:?}");
                return Frame::Skipped;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });

        self.scene.draw(
            &mut encoder,
            &self.targets.hdr,
            &self.targets.depth,
            &self.uniform_bind_group,
        );
        // The beads fade out as the ferrofluid takes over; once they are gone
        // there is no reason to keep drawing them.
        if stage.spike < 0.99 || stage.collapse > 0.85 {
            self.particles.draw(
                &mut encoder,
                &self.targets.hdr,
                &self.targets.depth,
                &self.uniform_bind_group,
                beads,
            );
        }
        // Before bloom, so the fireflies and letterforms feed it.
        self.text.draw(
            &mut encoder,
            &self.targets.hdr,
            &self.uniform_bind_group,
            stage.card,
        );

        self.bloom
            .draw(&mut encoder, &self.bloom_targets, &self.uniform_bind_group);
        self.post.draw(
            &mut encoder,
            &view,
            &self.uniform_bind_group,
            &self.hdr_bind_group,
            self.bloom_targets.result(),
        );

        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(frame);
        Frame::Rendered
    }
}
