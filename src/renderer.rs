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
use crate::passes::fluid::FluidPass;
use crate::passes::text::TextPass;
use crate::passes::{
    DEPTH_FORMAT, HDR_FORMAT, particles::ParticlePass, post::PostPass, scene::ScenePass,
};
use crate::timeline::{Camera, Director, Flow, Spin, Stage};

/// Every card, in the order the timeline indexes them: the three intro
/// titles, then the credits that run under the ferrofluid.
const CARDS: [&str; 6] = [
    "smeuch",
    "is back",
    "2026",
    "code: kranken",
    "ideas: spinax",
    "dagspress: whodini",
];

/// Supersampling factor. The scene renders at this multiple of the window and
/// the post pass filters it down — with a linear sampler, 2x is an exact 2x2
/// box per output pixel. Simple, and no ghosting the way reprojection has.
const RENDER_SCALE: u32 = 2;

pub const PARTICLE_COUNT: u32 = 512;
/// Beads in the fractal scene, split between the strings. At the spacing in
/// common.wgsl this gives each about half the corridor's length — so a string
/// reads as a cord with a head and a tail, rather than as a loop with no ends.
const FRACTAL_BEADS: u32 = 180;

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
    /// (collapse, unused, unused, unused).
    collapse: [f32; 4],
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
    /// Note the scene is allocated supersampled; bloom and the swapchain are
    /// not.
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let width = width * RENDER_SCALE;
        let height = height * RENDER_SCALE;
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
    fluid: FluidPass,
    particles: ParticlePass,
    bloom: BloomPass,
    text: TextPass,
    post: PostPass,

    targets: Targets,
    bloom_targets: BloomTargets,
    hdr_bind_group: wgpu::BindGroup,
    /// One per dye buffer; the fluid says which is current each frame.
    mask_bind_groups: [wgpu::BindGroup; 2],
    /// Draws the spectrum as bars over the frame, for picking a band by eye.
    pub show_bands: bool,
    director: Director,
    spin: Spin,
    flow: Flow,
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
                // Compute too: the fluid kernels read time and dt from here.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
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

        let targets = Targets::new(device, gpu.config.width, gpu.config.height);
        let fluid = FluidPass::new(device, &uniform_layout, &targets.depth);
        let bloom_targets =
            bloom.targets(device, &targets.hdr, gpu.config.width, gpu.config.height);
        let hdr_bind_group = post.make_bind_group(device, &targets.hdr);
        let masks = fluid.mask_views();
        let mask_bind_groups = [
            post.make_bind_group(device, masks[0]),
            post.make_bind_group(device, masks[1]),
        ];

        Self {
            uniform_buf,
            uniform_bind_group,
            scene,
            fluid,
            particles,
            bloom,
            text,
            post,
            targets,
            bloom_targets,
            hdr_bind_group,
            mask_bind_groups,
            show_bands: false,
            director: Director::default(),
            spin: Spin::default(),
            flow: Flow::default(),
        }
    }

    pub fn resize(&mut self, gpu: &Gpu) {
        self.targets = Targets::new(&gpu.device, gpu.config.width, gpu.config.height);
        self.fluid.resize(&gpu.device, &self.targets.depth);
        self.bloom_targets = self.bloom.targets(
            &gpu.device,
            &self.targets.hdr,
            gpu.config.width,
            gpu.config.height,
        );
        self.hdr_bind_group = self.post.make_bind_group(&gpu.device, &self.targets.hdr);
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
    ) -> Uniforms {
        let time = music.time;
        let eye = shot.eye;

        let aspect = gpu.config.width as f32 / gpu.config.height as f32;
        let view = view::look_at_mat4(eye, shot.target, Vec3::Y);
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
            scene: [stage.spike, stage.dissolve, stage.burst, stage.smoke],
            collapse: [stage.collapse, stage.bleed, along, radius],
            track,
            track_frame,
            motion: [stage.merge, spin.yaw, spin.tilt, stage.palette],
            frame: [music.dt.min(1.0 / 30.0), stage.wash, flow, stage.beads],
        }
    }

    pub fn render(&mut self, gpu: &Gpu, music: &Sync) -> Frame {
        let stage = Stage::at(music);

        let beads = if stage.collapse > 0.85 {
            FRACTAL_BEADS
        } else {
            PARTICLE_COUNT
        };
        let shot = self.director.update(music, &stage);

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

        // Once the smoke has cleared there is nothing to solve or draw. The
        // dissolve mask still samples the dye, but by then its threshold has
        // swept past everything, so a frozen field reads the same.
        let fluid_visible = stage.smoke > 0.01;
        if fluid_visible {
            self.fluid
                .simulate(&mut encoder, &self.uniform_bind_group, PARTICLE_COUNT);
        }

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
                // The same condition the fluid is drawn on: the beads only
                // need to lay down depth where there are smoke sheets to
                // composite against them.
                fluid_visible,
            );
        }
        if fluid_visible {
            self.fluid
                .draw(&mut encoder, &self.targets.hdr, &self.uniform_bind_group);
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
            &self.mask_bind_groups[self.fluid.mask_parity()],
        );

        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(frame);
        Frame::Rendered
    }
}
