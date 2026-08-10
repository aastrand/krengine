use super::HDR_FORMAT;
use crate::shader;

/// How many halvings the chain does. Five is enough for a wide, soft glow at
/// 1080p without the smallest level collapsing to a few pixels.
const LEVELS: usize = 5;

/// Threshold, blur and recombine the bright parts of the frame.
///
/// The chain is built by repeated downsampling rather than a wide gaussian:
/// each halving is four bilinear taps, and the result is both cheaper and
/// smoother than a single large kernel.
pub struct BloomPass {
    prefilter: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
    upsample: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// The mip chain, rebuilt on resize.
pub struct BloomTargets {
    views: Vec<wgpu::TextureView>,
    /// Bind group for reading each level.
    reads: Vec<wgpu::BindGroup>,
    /// Bind group for reading the full-resolution scene.
    scene_read: wgpu::BindGroup,
}

impl BloomTargets {
    /// The level the post pass composites — the largest, at half resolution.
    pub fn result(&self) -> &wgpu::BindGroup {
        &self.reads[0]
    }
}

impl BloomPass {
    pub fn new(device: &wgpu::Device, uniform_layout: &wgpu::BindGroupLayout) -> Self {
        let module = shader::module(device, "bloom.wgsl", include_str!("../shaders/bloom.wgsl"));

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom layout"),
            bind_group_layouts: &[Some(uniform_layout), Some(&texture_layout)],
            immediate_size: 0,
        });

        // Clamp to edge, or the blur wraps light from one side of the frame to
        // the other at the smallest levels.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };

        let make = |label: &str, entry: &str, blend: Option<wgpu::BlendState>| {
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
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        Self {
            prefilter: make("bloom prefilter", "fs_prefilter", None),
            downsample: make("bloom downsample", "fs_downsample", None),
            upsample: make(
                "bloom upsample",
                "fs_upsample",
                Some(wgpu::BlendState {
                    color: additive,
                    alpha: additive,
                }),
            ),
            texture_layout,
            sampler,
        }
    }

    pub fn texture_layout(&self) -> &wgpu::BindGroupLayout {
        &self.texture_layout
    }

    fn bind(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom read"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Build the chain. Sizes come from the output resolution, not the
    /// supersampled scene — blurred highlights don't need the extra pixels.
    pub fn targets(
        &self,
        device: &wgpu::Device,
        scene: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> BloomTargets {
        let mut views = Vec::with_capacity(LEVELS);
        for level in 0..LEVELS {
            let shift = level as u32 + 1;
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bloom level"),
                size: wgpu::Extent3d {
                    width: (width >> shift).max(1),
                    height: (height >> shift).max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            views.push(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        }

        let reads = views.iter().map(|v| self.bind(device, v)).collect();
        BloomTargets {
            scene_read: self.bind(device, scene),
            views,
            reads,
        }
    }

    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &BloomTargets,
        uniforms: &wgpu::BindGroup,
    ) {
        // Threshold the scene into the first level.
        self.blit(
            encoder,
            "bloom prefilter",
            &self.prefilter,
            &targets.views[0],
            uniforms,
            &targets.scene_read,
            true,
        );

        // Down the chain.
        for level in 1..LEVELS {
            self.blit(
                encoder,
                "bloom downsample",
                &self.downsample,
                &targets.views[level],
                uniforms,
                &targets.reads[level - 1],
                true,
            );
        }

        // And back up, adding each level onto the one above it.
        for level in (1..LEVELS).rev() {
            self.blit(
                encoder,
                "bloom upsample",
                &self.upsample,
                &targets.views[level - 1],
                uniforms,
                &targets.reads[level],
                false,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn blit(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        label: &str,
        pipeline: &wgpu::RenderPipeline,
        target: &wgpu::TextureView,
        uniforms: &wgpu::BindGroup,
        source: &wgpu::BindGroup,
        clear: bool,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, uniforms, &[]);
        pass.set_bind_group(1, source, &[]);
        pass.draw(0..3, 0..1);
    }
}
