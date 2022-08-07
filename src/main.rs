#![windows_subsystem = "windows"]

use std::{collections::HashMap, fs, iter, mem, time::Instant};

use rand::Rng;
use serde::{Deserialize, Serialize};
use wgpu::{include_wgsl, util::DeviceExt};
use winit::{
    dpi::PhysicalSize,
    event::{ElementState, Event, KeyboardInput, VirtualKeyCode, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Fullscreen, WindowBuilder},
};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    tex_coords: [f32; 2],
}

#[repr(C, align(16))]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Agent {
    position: [f32; 2],
    angle: f32,
    species: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Species {
    color: [f32; 3],
    _p0: u32,
    move_speed: f32,
    turn_speed: f32,
    sensor_angle: f32,
    sensor_offset: f32,
    sensor_size: u32,
    decay_rate: f32,
    diffuse_rate: f32,
    like_index: u32,
    like_length: u32,
    hate_index: u32,
    hate_length: u32,
    _p1: u32,
}

#[derive(Serialize, Deserialize)]
struct Settings {
    width: u32,
    height: u32,
    fullscreen: bool,
    species: Vec<SpeciesSettings>,
}

#[derive(Serialize, Deserialize)]
struct SpeciesSettings {
    name: String,
    color: [u8; 3],
    amount: u32,
    likes: Vec<String>,
    hates: Vec<String>,
    move_speed: f32,
    turn_speed: f32,
    sensor_angle: f32,
    sensor_offset: f32,
    sensor_size: u32,
    decay_rate: f32,
    diffuse_rate: f32,
}

#[tokio::main]
async fn main() {
    let settings: Settings = toml::from_str(&fs::read_to_string("settings.toml").unwrap()).unwrap();

    let event_loop = EventLoop::new();
    let mut window_builder = WindowBuilder::new().with_resizable(false);

    if settings.fullscreen {
        window_builder = window_builder.with_fullscreen(Some(Fullscreen::Borderless(None)));
    } else {
        window_builder =
            window_builder.with_inner_size(PhysicalSize::new(settings.width, settings.height));
    };

    let window = window_builder.build(&event_loop).unwrap();
    let PhysicalSize { width, height } = window.inner_size();

    let instance = wgpu::Instance::new(wgpu::Backends::all());
    let surface = unsafe { instance.create_surface(&window) };

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
        .unwrap();

    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor {
                features: wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
                limits: wgpu::Limits::default(),
                label: None,
            },
            None,
        )
        .await
        .unwrap();

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface.get_supported_formats(&adapter)[0],
        width,
        height,
        present_mode: wgpu::PresentMode::Fifo,
    };
    surface.configure(&device, &config);

    let width = settings.width;
    let height = settings.height;

    let species_settings = settings.species;
    let mut species_map = HashMap::new();

    for (i, species_setting) in species_settings.iter().enumerate() {
        species_map.insert(species_setting.name.to_string(), i);
    }

    let mut relations = Vec::new();

    let species = species_settings
        .iter()
        .map(|st| {
            let r = st.color[0] as f32 / 255.0;
            let g = st.color[1] as f32 / 255.0;
            let b = st.color[2] as f32 / 255.0;
            let color = [r, g, b];

            let like_index = relations.len() as u32;

            for name in &st.likes {
                relations.push(*species_map.get(name).unwrap() as u32);
            }

            let hate_index = relations.len() as u32;

            for name in &st.hates {
                relations.push(*species_map.get(name).unwrap() as u32);
            }

            let like_length = hate_index - like_index;
            let hate_length = relations.len() as u32 - hate_index;

            Species {
                color,
                _p0: 0,
                move_speed: st.move_speed,
                turn_speed: st.turn_speed,
                sensor_angle: st.sensor_angle,
                sensor_offset: st.sensor_offset,
                sensor_size: st.sensor_size,
                decay_rate: st.decay_rate,
                diffuse_rate: st.diffuse_rate,
                like_index,
                like_length,
                hate_index,
                hate_length,
                _p1: 0,
            }
        })
        .collect::<Vec<_>>();

    let radius = height as f32 * 0.4;
    let center_x = width as f32 / 2.0;
    let center_y = height as f32 / 2.0;

    let agents = species_settings
        .iter()
        .flat_map(|s| {
            (0..s.amount).map(|_| {
                let mut rng = rand::thread_rng();

                let (x, y) = loop {
                    let x = rng.gen_range((center_x - radius)..(center_x + radius));
                    let y = rng.gen_range((center_y - radius)..(center_y + radius));

                    if (x - center_x) * (x - center_x) + (y - center_y) * (y - center_y)
                        <= radius * radius
                    {
                        break (x, y);
                    }
                };

                let angle = (center_y - y).atan2(center_x - x);

                Agent {
                    position: [x, y],
                    angle,
                    species: *species_map.get(&s.name).unwrap() as u32,
                }
            })
        })
        .collect::<Vec<_>>();

    let (x, y, z) = {
        let len = agents.len() as u32;
        let floor = (len as f32).cbrt() as u32;
        let ceil = floor + 1;

        if floor * floor * floor >= len {
            (floor, floor, floor)
        } else if ceil * floor * floor >= len {
            (ceil, floor, floor)
        } else if ceil * ceil * floor >= len {
            (ceil, ceil, floor)
        } else {
            (ceil, ceil, ceil)
        }
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING,
    });

    let width_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[width]),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let height_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[height]),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let x_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[x]),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let y_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[y]),
        usage: wgpu::BufferUsages::UNIFORM,
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let species_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&species),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let relations_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&relations),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let agents_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&agents),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let weights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&vec![
            0.0f32;
            width as usize * height as usize * species.len()
        ]),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let time_delta_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: mem::size_of::<f32>() as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let compute_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::ReadWrite,
                        format: wgpu::TextureFormat::Rgba32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
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

    let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &compute_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: width_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: height_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: x_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: y_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: species_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: relations_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: agents_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: weights_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: time_delta_buffer.as_entire_binding(),
            },
        ],
    });

    let compute_shader = device.create_shader_module(include_wgsl!("compute.wgsl"));

    let compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&compute_bind_group_layout],
        push_constant_ranges: &[],
    });

    let main_1_compute_pipeline =
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: "main_1",
        });

    let main_2_compute_pipeline =
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: "main_2",
        });

    let main_3_compute_pipeline =
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: "main_3",
        });

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());

    let vertices: &[Vertex] = &[
        Vertex {
            position: [-1.0, 1.0],
            tex_coords: [0.0, 0.0],
        },
        Vertex {
            position: [-1.0, -1.0],
            tex_coords: [0.0, 1.0],
        },
        Vertex {
            position: [1.0, -1.0],
            tex_coords: [1.0, 1.0],
        },
        Vertex {
            position: [1.0, 1.0],
            tex_coords: [1.0, 0.0],
        },
    ];

    let indices: &[u16] = &[0, 1, 2, 0, 2, 3];

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    let render_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
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

    let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &render_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    let render_shader = device.create_shader_module(include_wgsl!("render.wgsl"));

    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&render_bind_group_layout],
        push_constant_ranges: &[],
    });

    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&render_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &render_shader,
            entry_point: "vs_main",
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 0,
                        format: wgpu::VertexFormat::Float32x2,
                    },
                    wgpu::VertexAttribute {
                        offset: mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                        shader_location: 1,
                        format: wgpu::VertexFormat::Float32x2,
                    },
                ],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: &render_shader,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format: config.format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview: None,
    });

    let mut start = Instant::now();

    event_loop.run(move |event, _, control_flow| match event {
        Event::WindowEvent {
            ref event,
            window_id,
        } if window_id == window.id() => match event {
            WindowEvent::CloseRequested
            | WindowEvent::KeyboardInput {
                input:
                    KeyboardInput {
                        state: ElementState::Pressed,
                        virtual_keycode: Some(VirtualKeyCode::Escape),
                        ..
                    },
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => {}
        },
        Event::RedrawRequested(window_id) if window_id == window.id() => {
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

            let time_delta = start.elapsed().as_secs_f32();
            start = Instant::now();
            queue.write_buffer(&time_delta_buffer, 0, bytemuck::cast_slice(&[time_delta]));

            {
                let mut compute_pass =
                    encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None });
                compute_pass.set_bind_group(0, &compute_bind_group, &[]);

                compute_pass.set_pipeline(&main_1_compute_pipeline);
                compute_pass.dispatch_workgroups(x, y, z);

                compute_pass.set_pipeline(&main_2_compute_pipeline);
                compute_pass.dispatch_workgroups(width, height, 1);

                compute_pass.set_pipeline(&main_3_compute_pipeline);
                compute_pass.dispatch_workgroups(width, height, 1);
            }

            let output = surface.get_current_texture().unwrap();
            let view = output
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: true,
                        },
                    })],
                    depth_stencil_attachment: None,
                });

                render_pass.set_pipeline(&render_pipeline);
                render_pass.set_bind_group(0, &render_bind_group, &[]);
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
            }

            queue.submit(iter::once(encoder.finish()));
            output.present();
        }
        Event::MainEventsCleared => {
            window.request_redraw();
        }
        _ => {}
    });
}
