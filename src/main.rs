use image::{ImageBuffer, Rgba};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
};
use std::ptr::NonNull;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{
        wl_output,
        wl_region::{self},
        wl_surface,
    },
};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    tex_coords: [f32; 2],
}

impl Vertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
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

// Function to create vertices with scale applied
fn create_vertices(scale: f32) -> [Vertex; 4] {
    [
        Vertex {
            position: [-scale, scale, 0.0],
            tex_coords: [0.0, 0.0],
        }, // A
        Vertex {
            position: [scale, scale, 0.0],
            tex_coords: [1.0, 0.0],
        }, // B
        Vertex {
            position: [scale, -scale, 0.0],
            tex_coords: [1.0, 1.0],
        }, // C
        Vertex {
            position: [-scale, -scale, 0.0],
            tex_coords: [0.0, 1.0],
        }, // D
    ]
}

const INDICES: &[u16] = &[0, 3, 1, 1, 3, 2];

const SCALE: f32 = 0.8; // Controls the size of the crosshair

#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: cgmath::Matrix4<f32> = cgmath::Matrix4::from_cols(
    cgmath::Vector4::new(1.0, 0.0, 0.0, 0.0),
    cgmath::Vector4::new(0.0, 1.0, 0.0, 0.0),
    cgmath::Vector4::new(0.0, 0.0, 0.5, 0.0),
    cgmath::Vector4::new(0.0, 0.0, 0.5, 1.0),
);

fn main() {
    env_logger::init();

    let diffuse_bytes = include_bytes!("../cross.png");
    let diffuse_image = image::load_from_memory(diffuse_bytes).unwrap();
    let diffuse_rgba = diffuse_image.to_rgba8();

    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    let layer_shell = LayerShell::bind(&globals, &qh).expect("layer shell is not available");

    // Initialize xdg_shell handlers so we can select the correct adapter
    let compositor_state =
        CompositorState::bind(&globals, &qh).expect("wl_compositor not available");
    // let xdg_shell_state = XdgShell::bind(&globals, &qh).expect("xdg shell not available");

    let region = compositor_state.wl_compositor().create_region(&qh, ());
    region.add(0, 0, 0, 0);
    let surface = compositor_state.create_surface(&qh);

    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("opencrosshair_layer"),
        None,
    );
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_input_region(Some(&region));
    let (size_x, size_y) = diffuse_rgba.dimensions();
    layer.set_size(size_x, size_y);
    // layer.set_margin(0, 0, 0, 0);
    layer.set_exclusive_zone(-1);
    layer.commit();

    // Initialize wgpu
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });

    // Create the raw window handle for the surface.
    let raw_display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
        NonNull::new(conn.backend().display_ptr() as *mut _).unwrap(),
    ));
    let raw_window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
        NonNull::new(layer.wl_surface().id().as_ptr() as *mut _).unwrap(),
    ));

    let surface = unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle,
                raw_window_handle,
            })
            .unwrap()
    };

    // Pick a supported adapter
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .expect("Failed to find suitable adapter");

    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
        .expect("Failed to request device");

    let mut wgpu = Wgpu {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),

        crosshair: diffuse_rgba,

        exit: false,
        width: size_x,
        height: size_y,
        layer,
        adapter,
        device,
        queue,
        surface,

        // Initialize as None - will be created on first configure
        shader: None,
        vertex_buffer: None,
        index_buffer: None,
        diffuse_texture: None,
        diffuse_texture_view: None,
        diffuse_sampler: None,
        texture_bind_group: None,
        texture_bind_group_layout: None,
        camera_bind_group_layout: None,
        render_pipeline: None,
        initialized: false,
    };

    // We don't draw immediately, the configure will notify us when to first draw.
    loop {
        event_queue.blocking_dispatch(&mut wgpu).unwrap();

        if wgpu.exit {
            println!("exiting example");
            break;
        }
    }

    // On exit we must destroy the surface before the window is destroyed.
    drop(wgpu.surface);
    // drop(wgpu.window);
}

struct Wgpu {
    registry_state: RegistryState,
    output_state: OutputState,

    crosshair: ImageBuffer<Rgba<u8>, Vec<u8>>,

    exit: bool,
    width: u32,
    height: u32,
    layer: LayerSurface,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,

    // Pre-created resources to avoid recreating on every configure
    shader: Option<wgpu::ShaderModule>,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    diffuse_texture: Option<wgpu::Texture>,
    diffuse_texture_view: Option<wgpu::TextureView>,
    diffuse_sampler: Option<wgpu::Sampler>,
    texture_bind_group: Option<wgpu::BindGroup>,
    texture_bind_group_layout: Option<wgpu::BindGroupLayout>,
    camera_bind_group_layout: Option<wgpu::BindGroupLayout>,
    render_pipeline: Option<wgpu::RenderPipeline>,
    initialized: bool,
}

impl CompositorHandler for Wgpu {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
        // Not needed for this example.
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        // Not needed for this example.
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // Not needed for this example.
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // Not needed for this example.
    }
}

impl OutputHandler for Wgpu {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for Wgpu {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        _configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let surface = &self.surface;
        let _device = &self.device;
        let _queue = &self.queue;

        let cap = surface.get_capabilities(&self.adapter);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: cap.formats[0],
            view_formats: vec![cap.formats[0]],
            alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
            width: self.width,
            height: self.height,
            desired_maximum_frame_latency: 1,
            present_mode: wgpu::PresentMode::Fifo,
        };

        surface.configure(&self.device, &surface_config);

        // Initialize resources only once
        if !self.initialized {
            self.initialize_resources();
            self.initialized = true;
        }

        // Render the frame
        self.render_frame();
    }
}

impl Wgpu {
    fn initialize_resources(&mut self) {
        let device = &self.device;
        let queue = &self.queue;

        // Get surface capabilities to determine the correct format
        let cap = self.surface.get_capabilities(&self.adapter);

        // Load the crosshair image once
        let dimensions = self.crosshair.dimensions();

        let texture_size = wgpu::Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 1,
        };

        // Create texture
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
            &self.crosshair,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * dimensions.0),
                rows_per_image: Some(dimensions.1),
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

        // Create bind group layout for textures
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
                ],
                label: Some("texture_bind_group_layout"),
            });

        // Create bind group for textures
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
            ],
            label: Some("diffuse_bind_group"),
        });

        // Create vertices and buffers
        let vertices = create_vertices(SCALE);
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

        // Create shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // Create camera bind group layout
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
                label: Some("camera_bind_group_layout"),
            });

        // Create render pipeline layout
        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&texture_bind_group_layout, &camera_bind_group_layout],
                immediate_size: 0,
            });

        // Create render pipeline - use the same format as the surface
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
                    format: cap.formats[0], // Use the same format as the surface
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

        // Store all resources
        self.shader = Some(shader);
        self.vertex_buffer = Some(vertex_buffer);
        self.index_buffer = Some(index_buffer);
        self.diffuse_texture = Some(diffuse_texture);
        self.diffuse_texture_view = Some(diffuse_texture_view);
        self.diffuse_sampler = Some(diffuse_sampler);
        self.texture_bind_group = Some(texture_bind_group);
        self.texture_bind_group_layout = Some(texture_bind_group_layout);
        self.camera_bind_group_layout = Some(camera_bind_group_layout);
        self.render_pipeline = Some(render_pipeline);
    }

    fn render_frame(&mut self) {
        let _device = &self.device;
        let _queue = &self.queue;
        let surface = &self.surface;

        // Get current texture
        let surface_texture = surface
            .get_current_texture()
            .expect("failed to acquire next swapchain texture");
        dbg!(surface_texture.texture.size());
        let texture_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Update camera matrix for current dimensions (small fixed surface)
        let proj = cgmath::ortho(0.0, self.width as f32, 0.0, self.height as f32, -1.0, 1.0);

        // Center the crosshair in the small surface
        let pos_x = self.width as f32 / 2.0;
        let pos_y = self.height as f32 / 2.0;

        let translation =
            cgmath::Matrix4::from_translation(cgmath::Vector3::new(pos_x, pos_y, 0.0));

        let view_proj = OPENGL_TO_WGPU_MATRIX * proj * translation;
        let view_proj_array: [[f32; 4]; 4] = view_proj.into();

        // Create temporary camera buffer for this frame
        let camera_buffer = _device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Temp Camera Buffer"),
            contents: bytemuck::cast_slice(&[view_proj_array]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        // Create temporary camera bind group for this frame
        let camera_bind_group = _device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: self.camera_bind_group_layout.as_ref().unwrap(),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
            label: Some("temp_camera_bind_group"),
        });

        // Create command encoder and render pass
        let mut encoder = _device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
            renderpass.set_bind_group(1, &camera_bind_group, &[]);
            renderpass.set_vertex_buffer(0, self.vertex_buffer.as_ref().unwrap().slice(..));
            renderpass.set_index_buffer(
                self.index_buffer.as_ref().unwrap().slice(..),
                wgpu::IndexFormat::Uint16,
            );
            renderpass.draw_indexed(0..INDICES.len() as u32, 0, 0..1);
        }

        // Submit the command
        self.queue.submit(Some(encoder.finish()));
        surface_texture.present();
    }
}

delegate_compositor!(Wgpu);
delegate_output!(Wgpu);

delegate_layer!(Wgpu);

delegate_registry!(Wgpu);

impl ProvidesRegistryState for Wgpu {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

impl Dispatch<wl_region::WlRegion, ()> for Wgpu {
    fn event(
        _: &mut Self,
        _: &wl_region::WlRegion,
        _: wl_region::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Wgpu>,
    ) {
    }
}
