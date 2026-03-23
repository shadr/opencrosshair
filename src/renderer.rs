use image::ImageBuffer;
use image::Rgba;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub tex_coords: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ColorUniform {
    pub color: [f32; 4],
}

impl Vertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

/// Create vertices with scale applied
pub fn create_vertices(scale: f32) -> [Vertex; 4] {
    [
        Vertex {
            position: [-scale, scale, 0.0],
            tex_coords: [0.0, 0.0],
        },
        Vertex {
            position: [scale, scale, 0.0],
            tex_coords: [1.0, 0.0],
        },
        Vertex {
            position: [scale, -scale, 0.0],
            tex_coords: [1.0, 1.0],
        },
        Vertex {
            position: [-scale, -scale, 0.0],
            tex_coords: [0.0, 1.0],
        },
    ]
}

pub const INDICES: &[u16] = &[0, 3, 1, 1, 3, 2];

pub struct Renderer {
    shader: Option<wgpu::ShaderModule>,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    diffuse_texture: Option<wgpu::Texture>,
    diffuse_texture_view: Option<wgpu::TextureView>,
    diffuse_sampler: Option<wgpu::Sampler>,
    texture_bind_group: Option<wgpu::BindGroup>,
    texture_bind_group_layout: Option<wgpu::BindGroupLayout>,
    render_pipeline: Option<wgpu::RenderPipeline>,
    color_uniform_buffer: Option<wgpu::Buffer>,
    width: u32,
    height: u32,
}

impl Renderer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            shader: None,
            vertex_buffer: None,
            index_buffer: None,
            diffuse_texture: None,
            diffuse_texture_view: None,
            diffuse_sampler: None,
            texture_bind_group: None,
            texture_bind_group_layout: None,
            render_pipeline: None,
            color_uniform_buffer: None,
            width,
            height,
        }
    }

    pub fn initialize(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        adapter: &wgpu::Adapter,
        surface: &wgpu::Surface<'static>,
        crosshair: &ImageBuffer<Rgba<u8>, Vec<u8>>,
        scale: f32,
    ) {
        let cap = surface.get_capabilities(adapter);

        let texture_size = wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };

        let diffuse_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("crosshair_texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &diffuse_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            crosshair,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * self.width),
                rows_per_image: Some(self.height),
            },
            texture_size,
        );

        let diffuse_texture_view =
            diffuse_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let diffuse_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
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
                        ty: wgpu::BindingType::Buffer {
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(
                                std::mem::size_of::<ColorUniform>() as u64,
                            ),
                            ty: wgpu::BufferBindingType::Uniform,
                        },
                        count: None,
                    },
                ],
                label: Some("texture_bind_group_layout"),
            });

        let color_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("color_uniform_buffer"),
            size: std::mem::size_of::<ColorUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&diffuse_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&diffuse_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &color_uniform_buffer,
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<ColorUniform>() as u64),
                    }),
                },
            ],
            label: Some("diffuse_bind_group"),
        });

        let vertices = create_vertices(scale);
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&texture_bind_group_layout],
                immediate_size: 0,
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: cap.formats[0],
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
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
            multiview_mask: None,
            cache: None,
        });

        self.shader = Some(shader);
        self.vertex_buffer = Some(vertex_buffer);
        self.index_buffer = Some(index_buffer);
        self.diffuse_texture = Some(diffuse_texture);
        self.diffuse_texture_view = Some(diffuse_texture_view);
        self.diffuse_sampler = Some(diffuse_sampler);
        self.texture_bind_group = Some(texture_bind_group);
        self.texture_bind_group_layout = Some(texture_bind_group_layout);
        self.render_pipeline = Some(render_pipeline);
        self.color_uniform_buffer = Some(color_uniform_buffer);
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: &wgpu::Surface<'static>,
        color_rgba: [f32; 4],
        slot_pool: &mut SlotPool,
    ) -> Buffer {
        let color = ColorUniform { color: color_rgba };
        queue.write_buffer(
            self.color_uniform_buffer.as_ref().unwrap(),
            0,
            bytemuck::cast_slice(&[color]),
        );

        let surface_texture = surface
            .get_current_texture()
            .expect("failed to acquire next swapchain texture");
        let texture_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        {
            let mut renderpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            renderpass.set_pipeline(self.render_pipeline.as_ref().unwrap());
            renderpass.set_bind_group(0, self.texture_bind_group.as_ref().unwrap(), &[]);
            renderpass.set_vertex_buffer(0, self.vertex_buffer.as_ref().unwrap().slice(..));
            renderpass.set_index_buffer(
                self.index_buffer.as_ref().unwrap().slice(..),
                wgpu::IndexFormat::Uint16,
            );
            renderpass.draw_indexed(0..INDICES.len() as u32, 0, 0..1);
        }

        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let bytes_per_row_unaligned = self.width * 4;
        let bytes_per_row = (bytes_per_row_unaligned + align - 1) & !(align - 1);
        let buffer_size = bytes_per_row as u64 * self.height as u64;

        let read_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Readback Buffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &surface_texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &read_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        let submission_index = queue.submit(Some(encoder.finish()));
        surface_texture.present();

        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission_index),
            timeout: None,
        });

        let buffer_slice = read_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        let _ = device.poll(wgpu::PollType::Poll);
        rx.recv().unwrap().unwrap();

        let mapped_data = buffer_slice.get_mapped_range();

        let (drawn_buffer, canvas) = slot_pool
            .create_buffer(
                self.width as i32,
                self.height as i32,
                bytes_per_row as i32,
                wayland_client::protocol::wl_shm::Format::Argb8888,
            )
            .expect("Failed to create wl_buffer");
        canvas.copy_from_slice(&mapped_data[..canvas.len()]);

        drop(mapped_data);
        drop(read_buffer);

        drawn_buffer
    }

    pub fn cleanup(&mut self) {
        self.shader = None;
        self.vertex_buffer = None;
        self.index_buffer = None;
        self.diffuse_texture = None;
        self.diffuse_texture_view = None;
        self.diffuse_sampler = None;
        self.texture_bind_group = None;
        self.texture_bind_group_layout = None;
        self.render_pipeline = None;
        self.color_uniform_buffer = None;
    }

    pub fn is_initialized(&self) -> bool {
        self.texture_bind_group_layout.is_some()
    }
}
