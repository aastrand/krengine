use super::HDR_FORMAT;
use crate::shader;

/// Simulation resolution. Must match GRID in fluid.wgsl.
const WIDTH: u32 = 1024;
const HEIGHT: u32 = 576;
/// Workgroup size per axis. Must match the @workgroup_size in fluid.wgsl.
const GROUP: u32 = 8;
/// Pressure iterations. An even count leaves the result in the first pressure
/// texture, which is where the projection step looks for it.
const JACOBI: usize = 30;
/// Emitters sampled from the swarm.
const EMITTERS: u32 = 40;

const VELOCITY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Scalar fields are rgba16float rather than r32float: advection samples them
/// trilinearly, and r32float is not filterable. Three channels go unused.
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
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// A GPU fluid: velocity and dye on a 3D grid, stepped every frame.
pub struct FluidPass {
    emitter_pipeline: wgpu::ComputePipeline,
    advect_velocity: wgpu::ComputePipeline,
    vorticity: wgpu::ComputePipeline,
    divergence: wgpu::ComputePipeline,
    jacobi: wgpu::ComputePipeline,
    project: wgpu::ComputePipeline,
    advect_dye: wgpu::ComputePipeline,
    render: wgpu::RenderPipeline,

    emitter_bind: wgpu::BindGroup,

    // One bind group per step of the frame; the fields ping-pong between them.
    step_advect_velocity: wgpu::BindGroup,
    step_vorticity: wgpu::BindGroup,
    step_divergence: wgpu::BindGroup,
    step_jacobi: [wgpu::BindGroup; 2],
    step_project: wgpu::BindGroup,
    step_advect_dye: [wgpu::BindGroup; 2],
    read_dye: [wgpu::BindGroup; 2],

    /// Which dye texture holds the current frame's result.
    parity: usize,
}

impl FluidPass {
    pub fn new(device: &wgpu::Device, uniform_layout: &wgpu::BindGroupLayout) -> Self {
        let sim = shader::module(device, "fluid.wgsl", include_str!("../shaders/fluid.wgsl"));
        let emit = shader::module(
            device,
            "emitters.wgsl",
            include_str!("../shaders/emitters.wgsl"),
        );
        let view_shader = shader::module(
            device,
            "fluid_view.wgsl",
            include_str!("../shaders/fluid_view.wgsl"),
        );

        // --- resources ---
        // Three, not two: advect, vorticity and project each read one field
        // and write another, and a step may never read and write the same
        // texture. Three lets the chain run without a copy.
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

        // Every kernel binds all five slots, but each uses only some of them.
        // A texture may not be both sampled and storage-written inside one
        // dispatch, so idle slots point at scratch textures that are only ever
        // used one way.
        let scratch_read = field(device, "fluid scratch read", SCALAR_FORMAT);
        let scratch_vec = field(device, "fluid scratch write vec", VELOCITY_FORMAT);
        let scratch_scalar = field(device, "fluid scratch write scalar", SCALAR_FORMAT);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fluid sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let emitter_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fluid emitters"),
            size: (EMITTERS as u64) * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let emitter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid emitter layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
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
            ],
        });

        let sim_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fluid sim pipeline layout"),
                bind_group_layouts: &[Some(uniform_layout), Some(&sim_layout)],
                immediate_size: 0,
            });
        let emitter_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fluid emitter pipeline layout"),
                bind_group_layouts: &[Some(uniform_layout), Some(&emitter_layout)],
                immediate_size: 0,
            });

        let compute = |label: &str, layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };

        // --- bind groups ---
        // Slots the kernel doesn't touch still need something bound, so unused
        // outputs point at whichever field is idle during that step.
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
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: emitter_buffer.as_entire_binding(),
                    },
                ],
            })
        };

        let read_dye_bind = |view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fluid dye read"),
                layout: &render_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };

        Self {
            emitter_pipeline: compute(
                "fluid emitters",
                &emitter_pipeline_layout,
                &emit,
                "cs_main",
            ),
            advect_velocity: compute(
                "fluid advect velocity",
                &sim_pipeline_layout,
                &sim,
                "cs_advect_velocity",
            ),
            vorticity: compute(
                "fluid vorticity",
                &sim_pipeline_layout,
                &sim,
                "cs_vorticity",
            ),
            divergence: compute(
                "fluid divergence",
                &sim_pipeline_layout,
                &sim,
                "cs_divergence",
            ),
            jacobi: compute("fluid jacobi", &sim_pipeline_layout, &sim, "cs_jacobi"),
            project: compute("fluid project", &sim_pipeline_layout, &sim, "cs_project"),
            advect_dye: compute(
                "fluid advect dye",
                &sim_pipeline_layout,
                &sim,
                "cs_advect_dye",
            ),
            render: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("fluid view"),
                layout: Some(&device.create_pipeline_layout(
                    &wgpu::PipelineLayoutDescriptor {
                        label: Some("fluid view layout"),
                        bind_group_layouts: &[Some(uniform_layout), Some(&render_layout)],
                        immediate_size: 0,
                    },
                )),
                vertex: wgpu::VertexState {
                    module: &view_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &view_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR_FORMAT,
                        // Premultiplied: the shader returns light already scaled
                        // by coverage, so source colour adds and the background
                        // is attenuated by alpha.
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
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
            }),

            emitter_bind: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fluid emitter bind"),
                layout: &emitter_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: emitter_buffer.as_entire_binding(),
                }],
            }),

            // The frame's chain: a -> b -> c -> a, with pressure ping-ponging
            // between its own pair in the middle.
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
                bind(
                    "advect dye a",
                    &dye[0],
                    &velocity[0],
                    &scratch_vec,
                    &dye[1],
                ),
                bind(
                    "advect dye b",
                    &dye[1],
                    &velocity[0],
                    &scratch_vec,
                    &dye[0],
                ),
            ],
            read_dye: [read_dye_bind(&dye[1]), read_dye_bind(&dye[0])],
            parity: 0,
        }
    }

    /// Step the simulation one frame.
    pub fn simulate(&mut self, encoder: &mut wgpu::CommandEncoder, uniforms: &wgpu::BindGroup) {
        let groups_x = WIDTH / GROUP;
        let groups_y = HEIGHT / GROUP;
        self.parity ^= 1;

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fluid step"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, uniforms, &[]);

        pass.set_pipeline(&self.emitter_pipeline);
        pass.set_bind_group(1, &self.emitter_bind, &[]);
        pass.dispatch_workgroups(EMITTERS.div_ceil(64), 1, 1);

        pass.set_pipeline(&self.advect_velocity);
        pass.set_bind_group(1, &self.step_advect_velocity, &[]);
        pass.dispatch_workgroups(groups_x, groups_y, 1);

        pass.set_pipeline(&self.vorticity);
        pass.set_bind_group(1, &self.step_vorticity, &[]);
        pass.dispatch_workgroups(groups_x, groups_y, 1);

        pass.set_pipeline(&self.divergence);
        pass.set_bind_group(1, &self.step_divergence, &[]);
        pass.dispatch_workgroups(groups_x, groups_y, 1);

        pass.set_pipeline(&self.jacobi);
        for iteration in 0..JACOBI {
            pass.set_bind_group(1, &self.step_jacobi[iteration % 2], &[]);
            pass.dispatch_workgroups(groups_x, groups_y, 1);
        }

        pass.set_pipeline(&self.project);
        pass.set_bind_group(1, &self.step_project, &[]);
        pass.dispatch_workgroups(groups_x, groups_y, 1);

        pass.set_pipeline(&self.advect_dye);
        pass.set_bind_group(1, &self.step_advect_dye[self.parity], &[]);
        pass.dispatch_workgroups(groups_x, groups_y, 1);
    }

    /// Composite the dye field over the scene.
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
        pass.set_bind_group(1, &self.read_dye[self.parity], &[]);
        pass.draw(0..3, 0..1);
    }
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

