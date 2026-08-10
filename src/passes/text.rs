use wgpu::util::DeviceExt;

use super::HDR_FORMAT;
use crate::shader;
use crate::text::{FontAtlas, GlyphInstance};

/// The atlas covers the whole lowercase set rather than only what today's
/// cards use, so new text does not mean regenerating it.
const CHARACTERS: &str = "abcdefghijklmnopqrstuvwxyz0123456789:. ";

/// Draws the intro cards and the fireflies lighting them.
pub struct TextPass {
    text: wgpu::RenderPipeline,
    /// One bind group per card: its own laid-out glyphs.
    cards: Vec<(wgpu::BindGroup, wgpu::Buffer, u32)>,
}

impl TextPass {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        uniform_layout: &wgpu::BindGroupLayout,
        cards: &[&str],
    ) -> anyhow::Result<Self> {
        let atlas = FontAtlas::new(
            include_bytes!("../../assets/DejaVuSans-Bold.ttf"),
            CHARACTERS,
        )?;

        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("font atlas"),
                size: wgpu::Extent3d {
                    width: atlas.width,
                    height: atlas.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &atlas.pixels,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("font sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // One glyph buffer per card, laid out once at startup.
        let cards = cards
            .iter()
            .map(|card| {
                let instances: Vec<GlyphInstance> = atlas.layout(card);
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("card glyphs"),
                    contents: bytemuck::cast_slice(&instances),
                    usage: wgpu::BufferUsages::STORAGE,
                });
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("card bind"),
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: buffer.as_entire_binding(),
                        },
                    ],
                });
                log::info!("card {card:?}: {} glyphs", instances.len());
                (bind, buffer, instances.len() as u32)
            })
            .collect();

        let module = shader::module(device, "text.wgsl", include_str!("../shaders/text.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text pipeline layout"),
            bind_group_layouts: &[Some(uniform_layout), Some(&layout)],
            immediate_size: 0,
        });

        let make = |label: &str, vs: &str, fs: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        blend: Some(blend),
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

        Ok(Self {
            // Premultiplied, since the shader scales ink by coverage.
            text: make(
                "text",
                "vs_text",
                "fs_text",
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
            ),
            cards,
        })
    }

    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::TextureView,
        uniforms: &wgpu::BindGroup,
        card: i32,
    ) {
        let Some((bind, _buffer, glyphs)) = self.cards.get(card.max(0) as usize) else {
            return;
        };
        if card < 0 {
            return;
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("text pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: hdr,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        pass.set_bind_group(0, uniforms, &[]);
        pass.set_bind_group(1, bind, &[]);

        pass.set_pipeline(&self.text);
        pass.draw(0..6, 0..*glyphs);
    }
}
