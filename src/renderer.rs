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
use crate::passes::{DIST_FORMAT, HDR_FORMAT, particles::ParticlePass, post::PostPass, scene::ScenePass};

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
}

/// Offscreen render targets, rebuilt whenever the window resizes.
struct Targets {
    hdr: wgpu::TextureView,
    dist: wgpu::TextureView,
}

impl Targets {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let make = |label, format| {
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
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };

        Self {
            hdr: make("hdr target", HDR_FORMAT),
            dist: make("distance target", DIST_FORMAT),
        }
    }
}

pub struct Renderer {
    uniform_buf: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,

    scene: ScenePass,
    particles: ParticlePass,
    post: PostPass,

    targets: Targets,
    dist_bind_group: wgpu::BindGroup,
    hdr_bind_group: wgpu::BindGroup,
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
        let post = PostPass::new(device, &uniform_layout, gpu.config.format);

        let targets = Targets::new(device, gpu.config.width, gpu.config.height);
        let dist_bind_group = particles.make_bind_group(device, &targets.dist);
        let hdr_bind_group = post.make_bind_group(device, &targets.hdr);

        Self {
            uniform_buf,
            uniform_bind_group,
            scene,
            particles,
            post,
            targets,
            dist_bind_group,
            hdr_bind_group,
        }
    }

    pub fn resize(&mut self, gpu: &Gpu) {
        self.targets = Targets::new(&gpu.device, gpu.config.width, gpu.config.height);
        self.dist_bind_group = self
            .particles
            .make_bind_group(&gpu.device, &self.targets.dist);
        self.hdr_bind_group = self.post.make_bind_group(&gpu.device, &self.targets.hdr);
    }

    fn uniforms(gpu: &Gpu, time: f32) -> Uniforms {
        // Slow orbit with a gentle rise and fall — no input needed, it's a demo.
        let radius = 3.1 + (time * 0.23).sin() * 0.35;
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
        }
    }

    pub fn render(&self, gpu: &Gpu, time: f32) -> Frame {
        let uniforms = Self::uniforms(gpu, time);
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
            &self.targets.dist,
            &self.uniform_bind_group,
        );
        self.particles.draw(
            &mut encoder,
            &self.targets.hdr,
            &self.uniform_bind_group,
            &self.dist_bind_group,
            PARTICLE_COUNT,
        );
        self.post.draw(
            &mut encoder,
            &view,
            &self.uniform_bind_group,
            &self.hdr_bind_group,
        );

        gpu.queue.submit(Some(encoder.finish()));
        gpu.queue.present(frame);
        Frame::Rendered
    }
}
