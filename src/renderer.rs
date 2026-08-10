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

use crate::gpu::Gpu;
use crate::audio::Sync;
use crate::passes::bloom::{BloomPass, BloomTargets};
use crate::passes::fluid::FluidPass;
use crate::passes::{
    DEPTH_FORMAT, HDR_FORMAT, particles::ParticlePass, post::PostPass, scene::ScenePass,
};

/// Supersampling factor. The scene renders at this multiple of the window and
/// the post pass filters it down — with a linear sampler, 2x is an exact 2x2
/// box per output pixel. Simple, and no ghosting the way reprojection has.
const RENDER_SCALE: u32 = 2;

pub const PARTICLE_COUNT: u32 = 512;

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
    post: PostPass,

    targets: Targets,
    bloom_targets: BloomTargets,
    hdr_bind_group: wgpu::BindGroup,
    /// Draws the spectrum as bars over the frame, for picking a band by eye.
    pub show_bands: bool,
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

        Self {
            uniform_buf,
            uniform_bind_group,
            scene,
            fluid,
            particles,
            bloom,
            post,
            targets,
            bloom_targets,
            hdr_bind_group,
            show_bands: false,
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

    fn uniforms(gpu: &Gpu, music: &Sync, show_bands: bool) -> Uniforms {
        let time = music.time;
        // Slow orbit with a gentle rise and fall — no input needed, it's a demo.
        // Beats nudge the camera back a touch, so hits register even in a wide.
        let radius = 3.1 + (time * 0.23).sin() * 0.35 + music.beat * 0.12;
        let eye = Vec3::new(
            (time * 0.17).cos() * radius,
            0.6 + (time * 0.31).sin() * 0.5,
            (time * 0.17).sin() * radius,
        );

        let aspect = gpu.config.width as f32 / gpu.config.height as f32;
        let view = view::look_at_mat4(eye, Vec3::ZERO, Vec3::Y);
        // directx variant maps depth to 0..1, which is what wgpu expects.
        let proj = directx::perspective(60f32.to_radians(), aspect, 0.05, 100.0);
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
            particle_count: PARTICLE_COUNT as f32,
            audio: [music.low, music.mid, music.high, music.beat],
            music: [
                music.row as f32,
                music.pattern as f32,
                music.beat_phase,
                music.bar_phase,
            ],
            bands: bytemuck::cast(music.bands),
            debug: [if show_bands { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            // Clamped: a long stall must not blow the simulation up.
            frame: [music.dt.min(1.0 / 30.0), 0.0, 0.0, 0.0],
        }
    }

    pub fn render(&mut self, gpu: &Gpu, music: &Sync) -> Frame {
        let uniforms = Self::uniforms(gpu, music, self.show_bands);
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

        self.fluid
            .simulate(&mut encoder, &self.uniform_bind_group, PARTICLE_COUNT);

        self.scene.draw(
            &mut encoder,
            &self.targets.hdr,
            &self.targets.depth,
            &self.uniform_bind_group,
        );
        self.particles.draw(
            &mut encoder,
            &self.targets.hdr,
            &self.targets.depth,
            &self.uniform_bind_group,
            PARTICLE_COUNT,
        );
        self.fluid
            .draw(&mut encoder, &self.targets.hdr, &self.uniform_bind_group);
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
