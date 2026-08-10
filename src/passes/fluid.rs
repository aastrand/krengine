use wgpu::util::DeviceExt;

use super::HDR_FORMAT;
use crate::shader;

/// Simulation resolution per layer. Must match GRID in fluid.wgsl.
const WIDTH: u32 = 768;
const HEIGHT: u32 = 432;
/// Workgroup size per axis. Must match the @workgroup_size in fluid.wgsl.
const GROUP: u32 = 8;
/// Pressure iterations. An even count leaves the result in the first pressure
/// texture, which is where the projection step looks for it.
const JACOBI: usize = 18;

/// Independent sheets, each claiming a band of depth. A bead stirs the sheet
/// its own distance falls in, so smoke behind the blob is occluded by it while
/// smoke in front covers it — the depth separation a single sheet can never
/// have, however it is shaded.
const LAYERS: usize = 3;
/// Where each sheet sits relative to the origin along the view axis, ordered
/// far to near so they can be drawn back to front.
const LAYER_OFFSETS: [f32; LAYERS] = [1.15, 0.0, -1.15];
/// Depth band each sheet claims. Wide enough to overlap its neighbours, so a
/// bead drifting between bands hands over gradually instead of popping.
const LAYER_WIDTH: f32 = 1.3;

const VELOCITY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Scalar fields are rgba16float rather than r32float: advection samples them
/// bilinearly, and r32float is not filterable. Three channels go unused.
const SCALAR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

fn field(device: &wgpu::Device, label: &str, format: wgpu::TextureFormat) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// One sheet: its own fields, its own emitters, its own depth band.
struct FluidLayer {
    splat_bind: wgpu::BindGroup,
    /// The velocity buffer the splat pass writes into, and the dye pair.
    velocity_target: wgpu::TextureView,
    step_advect_velocity: wgpu::BindGroup,
    step_vorticity: wgpu::BindGroup,
    step_divergence: wgpu::BindGroup,
    step_jacobi: [wgpu::BindGroup; 2],
    step_project: wgpu::BindGroup,
    step_advect_dye: [wgpu::BindGroup; 2],
    read_dye: [wgpu::BindGroup; 2],
    dye: [wgpu::TextureView; 2],
    params: wgpu::Buffer,
    /// Which dye texture holds the current frame's result.
    parity: usize,
}

/// A stack of 2D fluid sheets at different depths.
pub struct FluidPass {
    velocity_splat: wgpu::RenderPipeline,
    dye_splat: wgpu::RenderPipeline,
    advect_velocity: wgpu::ComputePipeline,
    vorticity: wgpu::ComputePipeline,
    divergence: wgpu::ComputePipeline,
    jacobi: wgpu::ComputePipeline,
    project: wgpu::ComputePipeline,
    advect_dye: wgpu::ComputePipeline,
    render: wgpu::RenderPipeline,

    layers: Vec<FluidLayer>,
    render_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl FluidPass {
    pub fn new(
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        depth: &wgpu::TextureView,
    ) -> Self {
        let sim = shader::module(device, "fluid.wgsl", include_str!("../shaders/fluid.wgsl"));
        let splat = shader::module(device, "splat.wgsl", include_str!("../shaders/splat.wgsl"));
        let view = shader::module(
            device,
            "fluid_view.wgsl",
            include_str!("../shaders/fluid_view.wgsl"),
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fluid sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // --- layouts ---
        let sim_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid sim layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                storage_entry(3, VELOCITY_FORMAT),
                storage_entry(4, SCALAR_FORMAT),
            ],
        });

        let splat_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid splat layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });

        let render_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid render layout"),
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
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                uniform_entry(3, wgpu::ShaderStages::FRAGMENT),
            ],
        });

        let sim_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fluid sim pipeline layout"),
            bind_group_layouts: &[Some(uniform_layout), Some(&sim_layout)],
            immediate_size: 0,
        });
        let splat_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fluid splat pipeline layout"),
                bind_group_layouts: &[Some(uniform_layout), Some(&splat_layout)],
                immediate_size: 0,
            });

        let compute = |label: &str, entry: &str, module: &wgpu::ShaderModule, layout| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };

        let over = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };

        let splat_pipeline = |label: &str, entry: &str, blend: wgpu::BlendState, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&splat_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &splat,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &splat,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
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

        // Straight alpha is the constraint: dst*(1-a) + src*a is exactly
        // mix(fluid, bead velocity, grip).
        let velocity_splat = splat_pipeline(
            "fluid splat velocity",
            "fs_velocity",
            wgpu::BlendState::ALPHA_BLENDING,
            VELOCITY_FORMAT,
        );
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let dye_splat = splat_pipeline(
            "fluid splat dye",
            "fs_dye",
            wgpu::BlendState {
                color: additive,
                alpha: additive,
            },
            SCALAR_FORMAT,
        );

        let render = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fluid view"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("fluid view layout"),
                    bind_group_layouts: &[Some(uniform_layout), Some(&render_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &view,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &view,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    // Premultiplied: the shader scales colour by coverage.
                    blend: Some(wgpu::BlendState {
                        color: over,
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // --- one set of fields per layer ---
        let mut layers = Vec::with_capacity(LAYERS);
        for (index, offset) in LAYER_OFFSETS.iter().enumerate() {
            // Three velocity buffers, not two: advect, vorticity and project
            // each read one and write another, and no step may read and write
            // the same texture.
            let velocity = [
                field(device, "fluid velocity a", VELOCITY_FORMAT),
                field(device, "fluid velocity b", VELOCITY_FORMAT),
                field(device, "fluid velocity c", VELOCITY_FORMAT),
            ];
            let dye = [
                field(device, "fluid dye a", SCALAR_FORMAT),
                field(device, "fluid dye b", SCALAR_FORMAT),
            ];
            let pressure = [
                field(device, "fluid pressure a", SCALAR_FORMAT),
                field(device, "fluid pressure b", SCALAR_FORMAT),
            ];
            let divergence_tex = field(device, "fluid divergence", SCALAR_FORMAT);

            // Every kernel binds all five slots but uses only some. A texture
            // may not be both sampled and storage-written in one dispatch, so
            // idle slots point at scratch that is only ever used one way.
            let scratch_read = field(device, "fluid scratch read", SCALAR_FORMAT);
            let scratch_vec = field(device, "fluid scratch write vec", VELOCITY_FORMAT);
            let scratch_scalar = field(device, "fluid scratch write scalar", SCALAR_FORMAT);

            let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fluid layer params"),
                contents: bytemuck::cast_slice(&[*offset, LAYER_WIDTH, 0.0, 0.0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });

            let bind = |label: &str,
                        a: &wgpu::TextureView,
                        b: &wgpu::TextureView,
                        out_vec: &wgpu::TextureView,
                        out_scalar: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &sim_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(a),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(b),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(out_vec),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(out_scalar),
                        },
                    ],
                })
            };

            let read = |view: &wgpu::TextureView| {
                read_bind(device, &render_layout, view, &sampler, depth, &params)
            };

            layers.push(FluidLayer {
                splat_bind: device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fluid splat bind"),
                    layout: &splat_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: params.as_entire_binding(),
                    }],
                }),
                velocity_target: velocity[1].clone(),
                // The chain: a -> b -> c -> a, pressure ping-ponging between
                // its own pair in the middle.
                step_advect_velocity: bind(
                    "advect velocity",
                    &velocity[0],
                    &scratch_read,
                    &velocity[1],
                    &scratch_scalar,
                ),
                step_vorticity: bind(
                    "vorticity",
                    &velocity[1],
                    &scratch_read,
                    &velocity[2],
                    &scratch_scalar,
                ),
                step_divergence: bind(
                    "divergence",
                    &velocity[2],
                    &scratch_read,
                    &scratch_vec,
                    &divergence_tex,
                ),
                step_jacobi: [
                    bind(
                        "jacobi even",
                        &pressure[0],
                        &divergence_tex,
                        &scratch_vec,
                        &pressure[1],
                    ),
                    bind(
                        "jacobi odd",
                        &pressure[1],
                        &divergence_tex,
                        &scratch_vec,
                        &pressure[0],
                    ),
                ],
                step_project: bind(
                    "project",
                    &velocity[2],
                    &pressure[0],
                    &velocity[0],
                    &scratch_scalar,
                ),
                step_advect_dye: [
                    bind("advect dye a", &dye[0], &velocity[0], &scratch_vec, &dye[1]),
                    bind("advect dye b", &dye[1], &velocity[0], &scratch_vec, &dye[0]),
                ],
                read_dye: [read(&dye[1]), read(&dye[0])],
                dye,
                params,
                // Stagger the layers so they don't all update in lockstep.
                parity: index % 2,
            });
        }

        Self {
            velocity_splat,
            dye_splat,
            advect_velocity: compute(
                "fluid advect velocity",
                "cs_advect_velocity",
                &sim,
                &sim_pipeline_layout,
            ),
            vorticity: compute(
                "fluid vorticity",
                "cs_vorticity",
                &sim,
                &sim_pipeline_layout,
            ),
            divergence: compute(
                "fluid divergence",
                "cs_divergence",
                &sim,
                &sim_pipeline_layout,
            ),
            jacobi: compute("fluid jacobi", "cs_jacobi", &sim, &sim_pipeline_layout),
            project: compute("fluid project", "cs_project", &sim, &sim_pipeline_layout),
            advect_dye: compute(
                "fluid advect dye",
                "cs_advect_dye",
                &sim,
                &sim_pipeline_layout,
            ),
            render,
            layers,
            render_layout,
            sampler,
        }
    }

    /// The middle sheet's dye, which post uses as a transition mask, and
    /// which of the pair currently holds this frame's result.
    pub fn mask_views(&self) -> [&wgpu::TextureView; 2] {
        let layer = &self.layers[self.layers.len() / 2];
        [&layer.dye[0], &layer.dye[1]]
    }

    pub fn mask_parity(&self) -> usize {
        // advect writes into the buffer the parity does *not* name.
        1 - self.layers[self.layers.len() / 2].parity
    }

    /// Rebuild the bind groups that reference the depth buffer.
    pub fn resize(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView) {
        for layer in &mut self.layers {
            layer.read_dye = [
                read_bind(
                    device,
                    &self.render_layout,
                    &layer.dye[1],
                    &self.sampler,
                    depth,
                    &layer.params,
                ),
                read_bind(
                    device,
                    &self.render_layout,
                    &layer.dye[0],
                    &self.sampler,
                    depth,
                    &layer.params,
                ),
            ];
        }
    }

    /// Step every sheet one frame.
    ///
    /// Injection is a render pass rather than part of the compute chain, so the
    /// work splits: advect, splat the beads in, then solve and advect dye, then
    /// splat dye in.
    pub fn simulate(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        uniforms: &wgpu::BindGroup,
        beads: u32,
    ) {
        let groups_x = WIDTH / GROUP;
        let groups_y = HEIGHT / GROUP;

        for layer in &mut self.layers {
            layer.parity ^= 1;

            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("fluid advect velocity"),
                    timestamp_writes: None,
                });
                pass.set_bind_group(0, uniforms, &[]);
                pass.set_pipeline(&self.advect_velocity);
                pass.set_bind_group(1, &layer.step_advect_velocity, &[]);
                pass.dispatch_workgroups(groups_x, groups_y, 1);
            }

            splat(
                encoder,
                "fluid splat velocity",
                &self.velocity_splat,
                &layer.velocity_target,
                uniforms,
                &layer.splat_bind,
                beads,
            );

            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("fluid solve"),
                    timestamp_writes: None,
                });
                pass.set_bind_group(0, uniforms, &[]);

                pass.set_pipeline(&self.vorticity);
                pass.set_bind_group(1, &layer.step_vorticity, &[]);
                pass.dispatch_workgroups(groups_x, groups_y, 1);

                pass.set_pipeline(&self.divergence);
                pass.set_bind_group(1, &layer.step_divergence, &[]);
                pass.dispatch_workgroups(groups_x, groups_y, 1);

                pass.set_pipeline(&self.jacobi);
                for iteration in 0..JACOBI {
                    pass.set_bind_group(1, &layer.step_jacobi[iteration % 2], &[]);
                    pass.dispatch_workgroups(groups_x, groups_y, 1);
                }

                pass.set_pipeline(&self.project);
                pass.set_bind_group(1, &layer.step_project, &[]);
                pass.dispatch_workgroups(groups_x, groups_y, 1);

                pass.set_pipeline(&self.advect_dye);
                pass.set_bind_group(1, &layer.step_advect_dye[layer.parity], &[]);
                pass.dispatch_workgroups(groups_x, groups_y, 1);
            }

            splat(
                encoder,
                "fluid splat dye",
                &self.dye_splat,
                &layer.dye[1 - layer.parity],
                uniforms,
                &layer.splat_bind,
                beads,
            );
        }
    }

    /// Composite the sheets over the scene, back to front.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::TextureView,
        uniforms: &wgpu::BindGroup,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fluid view pass"),
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
        pass.set_pipeline(&self.render);
        pass.set_bind_group(0, uniforms, &[]);

        // LAYER_OFFSETS runs far to near, so plain order is back to front.
        for layer in &self.layers {
            pass.set_bind_group(1, &layer.read_dye[layer.parity], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// One quad per bead, blended into a simulation field.
fn splat(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    pipeline: &wgpu::RenderPipeline,
    target: &wgpu::TextureView,
    uniforms: &wgpu::BindGroup,
    layer: &wgpu::BindGroup,
    beads: u32,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
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
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, uniforms, &[]);
    pass.set_bind_group(1, layer, &[]);
    pass.draw(0..6, 0..beads);
}

fn read_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    dye: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    depth: &wgpu::TextureView,
    params: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fluid dye read"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(dye),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(depth),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: params.as_entire_binding(),
            },
        ],
    })
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn storage_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
