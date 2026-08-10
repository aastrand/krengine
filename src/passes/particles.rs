use super::{DEPTH_FORMAT, HDR_FORMAT};
use crate::shader;

/// Instanced billboards orbiting the blob. Positions are computed in the vertex
/// shader, so there are no buffers to feed — only a draw call.
pub struct ParticlePass {
    pipeline: wgpu::RenderPipeline,
}

impl ParticlePass {
    pub fn new(device: &wgpu::Device, uniform_layout: &wgpu::BindGroupLayout) -> Self {
        let module = shader::module(
            device,
            "particles.wgsl",
            include_str!("../shaders/particles.wgsl"),
        );

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle layout"),
            bind_group_layouts: &[Some(uniform_layout)],
            immediate_size: 0,
        });

        // Straight alpha, not additive: these particles are dark ink laid over
        // the scene, and additive blending can't darken anything.
        let over = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: over,
                        alpha: over,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            // Tests against the blob the scene pass wrote, and writes its own
            // depth so beads sort against each other too.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self { pipeline }
    }

    /// Loads the existing HDR contents — this pass draws on top of the scene.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        uniforms: &wgpu::BindGroup,
        count: u32,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particle pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: hdr,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, uniforms, &[]);
        pass.draw(0..6, 0..count);
    }
}
