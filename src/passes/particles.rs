use super::{DEPTH_FORMAT, HDR_FORMAT};
use crate::shader;

/// Instanced billboards orbiting the blob. Positions are computed in the vertex
/// shader, so there are no buffers to feed — only a draw call.
pub struct ParticlePass {
    /// Writes depth, for the scenes with smoke in them.
    occluding: wgpu::RenderPipeline,
    /// Does not, for the fractal, where the beads overlap each other.
    blending: wgpu::RenderPipeline,
    /// Writes only a small central depth core after the blended pass, allowing
    /// depth of field to focus the string without clipping its soft edges.
    focus_depth: wgpu::RenderPipeline,
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

        // Whether the beads write depth is the one thing that differs, and it
        // cannot be the same in both scenes.
        //
        // Writing it makes them clip each other rather than blend: a bead's
        // quad covers a square, most of it the soft fringe the fragment shader
        // fades out, and the depth goes down across all of it — so the bead
        // behind is cut off along that square's edge, the hard crescent that
        // showed wherever two overlapped. Alpha blending and depth writing the
        // same geometry cannot both be right.
        //
        // But the fluid composites its smoke sheets against this same depth
        // buffer, so beads that write nothing are buried under the smoke —
        // which is where the first scene's particles went when this pass
        // stopped writing depth outright.
        //
        // So: the scenes with smoke keep the depth and put up with the
        // crescents, which barely arise there because the beads are small and
        // spread thinly around the blob. The fractal, which has no fluid at
        // all and whose strings are dense enough that beads constantly
        // overlap, drops it and blends properly.
        let targets = [Some(wgpu::ColorTargetState {
            format: HDR_FORMAT,
            blend: Some(wgpu::BlendState {
                color: over,
                alpha: over,
            }),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let make = |label, depth_write| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
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
                    targets: &targets,
                }),
                primitive: wgpu::PrimitiveState::default(),
                // Always tests, so whatever the scene pass drew occludes them.
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let occluding = make("beads over smoke", true);
        let blending = make("beads on a string", false);
        let focus_depth = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bead focus depth"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_focus_depth"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
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

        Self {
            occluding,
            blending,
            focus_depth,
        }
    }

    /// Loads the existing HDR contents — this pass draws on top of the scene.
    ///
    /// `over_smoke` picks the pipeline: set it wherever the fluid is drawing,
    /// so the beads lay down depth for its sheets to composite against.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        uniforms: &wgpu::BindGroup,
        count: u32,
        over_smoke: bool,
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
        pass.set_pipeline(if over_smoke {
            &self.occluding
        } else {
            &self.blending
        });
        pass.set_bind_group(0, uniforms, &[]);
        pass.draw(0..6, 0..count);
        if !over_smoke {
            pass.set_pipeline(&self.focus_depth);
            pass.draw(0..6, 0..count);
        }
    }
}
