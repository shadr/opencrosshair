use clap::Parser;
use image::{ImageBuffer, Rgba};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputInfo, OutputState},
    reexports::protocols_wlr::foreign_toplevel::v1::client::{
        zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
        zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{
        Shm, ShmHandler,
        slot::{Buffer, SlotPool},
    },
};
use std::{collections::HashMap, path::PathBuf, ptr::NonNull};
use wayland_backend::client::ObjectId;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{
        wl_output::{self, WlOutput},
        wl_region, wl_surface,
    },
};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    tex_coords: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ColorUniform {
    color: [f32; 4],
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

#[derive(Parser)]
#[command(name = "opencrosshair")]
#[command(about = "A crosshair overlay for Wayland compositors", long_about = None)]
struct Args {
    /// App IDs for which the crosshair should be visible (comma-separated)
    #[arg(short = 'a', long = "app-ids", value_delimiter = ',')]
    app_ids: Vec<String>,

    /// Color of the crosshair in RGBA format (0.0-1.0 for each channel, comma-separated, e.g., "1.0,0.0,0.0,1.0" for red)
    #[arg(short = 'c', long = "color", default_value_t = String::from("1.0,0.0,0.0,1.0"))]
    color: String,

    /// Scale factor for the crosshair size (default: 0.8)
    #[arg(short = 's', long = "scale", default_value_t = 0.8)]
    scale: f32,

    /// Name of the display/output to show the crosshair on (e.g., HDMI-A-1, DP-1)
    #[arg(short = 'd', long = "display")]
    display: Option<String>,

    /// Path to the crosshair image file (default: embedded cross.png)
    #[arg(short = 'i', long = "image")]
    image: Option<PathBuf>,
}

fn main() {
    env_logger::init();

    let args = Args::parse();

    // Parse color values from command line
    let color_values: Vec<f32> = args
        .color
        .split(',')
        .map(|s| s.trim().parse().expect("Invalid color value"))
        .collect();

    if color_values.len() != 4 {
        eprintln!("Error: Color must have exactly 4 values (RGBA)");
        std::process::exit(1);
    }

    let color_rgba = [
        color_values[0],
        color_values[1],
        color_values[2],
        color_values[3],
    ];

    // Use the specified app IDs or default to an empty vector
    let visible_app_ids = if args.app_ids.is_empty() {
        Vec::new()
    } else {
        args.app_ids
    };

    let scale = args.scale;

    let diffuse_rgba = load_crosshair_image(&args.image);

    let conn = Connection::connect_to_env().unwrap();
    let (globals, mut event_queue) = registry_queue_init(&conn).unwrap();

    let qh = event_queue.handle();

    let layer_shell = LayerShell::bind(&globals, &qh).expect("layer shell is not available");

    // Initialize xdg_shell handlers so we can select the correct adapter
    let compositor_state =
        CompositorState::bind(&globals, &qh).expect("wl_compositor not available");

    let region = compositor_state.wl_compositor().create_region(&qh, ());
    region.add(0, 0, 0, 0);
    let surface = compositor_state.create_surface(&qh);

    let mut output = None;

    let outputs = get_outputs_with_info(&conn);
    if let Some(selected_output_name) = args.display {
        for (wloutput, outputinfo) in &outputs {
            if let Some(outputinfo) = outputinfo {
                if let Some(output_name) = &outputinfo.name {
                    if *output_name == selected_output_name {
                        output = Some(wloutput);
                        break;
                    }
                }
            }
        }
    }

    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some("opencrosshair_layer"),
        output,
    );
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_input_region(Some(&region));
    // Set anchors so we can set an explicit size
    let (size_x, size_y) = diffuse_rgba.dimensions();
    layer.set_size(size_x, size_y);
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

    let shm = Shm::bind(&globals, &qh).expect("wl_shm not available");
    let mut slot_pool = SlotPool::new(4096, &shm).expect("Failed to create slot pool");
    let (drawn_buffer, _) = slot_pool
        .create_buffer(
            size_x as i32,
            size_y as i32,
            size_x as i32 * 4,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        )
        .unwrap();
    let (empty_buffer, _) = slot_pool
        .create_buffer(
            size_x as i32,
            size_y as i32,
            size_x as i32 * 4,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        )
        .unwrap();

    let mut app = OpenCrosshair {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),

        layer,

        toplevels: HashMap::new(),
        visible_app_ids,

        crosshair: diffuse_rgba,

        color_rgba,

        scale,

        exit: false,
        width: size_x,
        height: size_y,
        adapter: Some(adapter),
        device: Some(device),
        queue: Some(queue),
        surface: Some(surface),

        // Initialize as None - will be created on first configure
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
        initialized: false,

        shm,
        slot_pool,
        drawn_buffer,
        empty_buffer,
    };

    app.registry_state
        .bind_one::<ZwlrForeignToplevelManagerV1, _, _>(&qh, 2..=3, ())
        .unwrap();

    // We don't draw immediately, the configure will notify us when to first draw.
    loop {
        event_queue.blocking_dispatch(&mut app).unwrap();

        if app.exit {
            println!("exiting example");
            break;
        }
    }

    // On exit we must destroy the surface before the window is destroyed.
    drop(app.surface);
}

/// Load crosshair image from specified path or use embedded default
fn load_crosshair_image(image: &Option<PathBuf>) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
    let diffuse_image = if let Some(image_path) = image {
        image::ImageReader::open(image_path)
            .unwrap_or_else(|e| {
                eprintln!("Error opening image '{:?}': {}", image_path, e);
                std::process::exit(1);
            })
            .decode()
            .unwrap_or_else(|e| {
                eprintln!("Error decoding image '{:?}': {}", image_path, e);
                std::process::exit(1);
            })
    } else {
        let diffuse_bytes = include_bytes!("../cross.png");
        image::load_from_memory(diffuse_bytes).unwrap()
    };
    diffuse_image.to_rgba8()
}

struct OpenCrosshair {
    registry_state: RegistryState,
    output_state: OutputState,

    layer: LayerSurface,

    toplevels: HashMap<ObjectId, String>,

    visible_app_ids: Vec<String>,

    crosshair: ImageBuffer<Rgba<u8>, Vec<u8>>,

    color_rgba: [f32; 4],

    scale: f32,

    exit: bool,
    width: u32,
    height: u32,
    adapter: Option<wgpu::Adapter>,
    device: Option<wgpu::Device>,
    queue: Option<wgpu::Queue>,
    surface: Option<wgpu::Surface<'static>>,

    // Pre-created resources to avoid recreating on every configure
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
    initialized: bool,

    // Wayland buffer for layer shell
    shm: Shm,
    slot_pool: SlotPool,
    drawn_buffer: Buffer,
    empty_buffer: Buffer,
}

impl CompositorHandler for OpenCrosshair {
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

impl OutputHandler for OpenCrosshair {
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

impl LayerShellHandler for OpenCrosshair {
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
        if self.surface.is_none() {
            return;
        }
        let surface = self.surface.as_ref().unwrap();
        let _device = self.device.as_ref().unwrap();
        let _queue = self.queue.as_ref().unwrap();

        let cap = surface.get_capabilities(self.adapter.as_ref().unwrap());
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: cap.formats[0],
            view_formats: vec![cap.formats[0]],
            alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
            width: self.width,
            height: self.height,
            desired_maximum_frame_latency: 1,
            present_mode: wgpu::PresentMode::Fifo,
        };

        surface.configure(self.device.as_ref().unwrap(), &surface_config);

        // Initialize resources only once
        if !self.initialized {
            self.initialize_resources();
            self.initialized = true;
        }

        // Render the frame
        self.render_frame();

        // if let SurfaceKind::Wlr(zwlr_surface) = self.layer.kind() {
        //     zwlr_surface.ack_configure(serial);
        // }
    }
}

impl OpenCrosshair {
    fn hide_layer(&mut self) {
        self.layer.set_size(self.width, self.height);
        self.layer.attach(Some(self.empty_buffer.wl_buffer()), 0, 0);
        self.layer.commit();
    }

    fn show_layer(&mut self) {
        self.layer.set_size(self.width, self.height);
        self.layer.attach(Some(self.drawn_buffer.wl_buffer()), 0, 0);
        self.layer.commit();
    }

    fn initialize_resources(&mut self) {
        let device = self.device.as_ref().unwrap();
        let queue = self.queue.as_ref().unwrap();

        // Get surface capabilities to determine the correct format
        let cap = self
            .surface
            .as_ref()
            .unwrap()
            .get_capabilities(self.adapter.as_ref().unwrap());

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

        // Create bind group for textures
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

        // Create vertices and buffers
        let vertices = create_vertices(self.scale);
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

        // Create render pipeline layout
        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Render Pipeline Layout"),
                bind_group_layouts: &[&texture_bind_group_layout],
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
        self.render_pipeline = Some(render_pipeline);
        self.color_uniform_buffer = Some(color_uniform_buffer);
    }

    fn render_frame(&mut self) {
        let device = self.device.as_ref().unwrap();
        let queue = self.queue.as_ref().unwrap();
        let surface = self.surface.as_ref().unwrap();

        // Set color uniform using the specified color
        let color = ColorUniform {
            color: self.color_rgba,
        };
        queue.write_buffer(
            self.color_uniform_buffer.as_ref().unwrap(),
            0,
            bytemuck::cast_slice(&[color]),
        );

        // Get current texture
        let surface_texture = surface
            .get_current_texture()
            .expect("failed to acquire next swapchain texture");
        let texture_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Create command encoder and render pass
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

        // Create a buffer to read back the rendered texture
        // bytes_per_row must be aligned to COPY_BYTES_PER_ROW_ALIGNMENT (256 bytes)
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

        // Copy texture to read buffer
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

        // Submit the commands and get submission index
        let submission_index = queue.submit(Some(encoder.finish()));
        surface_texture.present();

        // Wait for the GPU to finish
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission_index),
            timeout: None,
        });

        // Map the buffer to read the data
        let buffer_slice = read_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        let _ = device.poll(wgpu::PollType::Poll);
        rx.recv().unwrap().unwrap();

        let mapped_data = buffer_slice.get_mapped_range();

        let (buffer, canvas) = self
            .slot_pool
            .create_buffer(
                self.width as i32,
                self.height as i32,
                bytes_per_row as i32,
                wayland_client::protocol::wl_shm::Format::Argb8888,
            )
            .expect("Failed to create wl_buffer");

        canvas.copy_from_slice(&mapped_data[..canvas.len()]);

        self.drawn_buffer = buffer;

        // Clean up wgpu resources
        drop(mapped_data);
        drop(read_buffer);

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
        self.adapter = None;
        self.device = None;
        self.queue = None;
        self.surface = None;
    }
}

impl ProvidesRegistryState for OpenCrosshair {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

impl Dispatch<wl_region::WlRegion, ()> for OpenCrosshair {
    fn event(
        _: &mut Self,
        _: &wl_region::WlRegion,
        _: wl_region::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<OpenCrosshair>,
    ) {
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for OpenCrosshair {
    fn event(
        _: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        _: <ZwlrForeignToplevelManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<OpenCrosshair>,
    ) {
    }

    fn event_created_child(
        _opcode: u16,
        qhandle: &QueueHandle<Self>,
    ) -> std::sync::Arc<dyn wayland_backend::client::ObjectData> {
        qhandle.make_data::<ZwlrForeignToplevelHandleV1, ()>(())
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for OpenCrosshair {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: <ZwlrForeignToplevelHandleV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<OpenCrosshair>,
    ) {
        match event {
            // zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
            //     dbg!(&title);
            // }
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                state.toplevels.insert(handle.id(), app_id);
            }
            // zwlr_foreign_toplevel_handle_v1::Event::OutputEnter { output } => todo!(),
            // zwlr_foreign_toplevel_handle_v1::Event::OutputLeave { output } => todo!(),
            zwlr_foreign_toplevel_handle_v1::Event::State { state: states } => {
                if states
                    .chunks_exact(4)
                    .any(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) == 2)
                {
                    let handle_id = handle.id();
                    if let Some(app_id) = state.toplevels.get(&handle_id) {
                        // Check if the app_id is in the list of visible app IDs
                        if state.visible_app_ids.is_empty()
                            || state.visible_app_ids.iter().any(|id| id == app_id)
                        {
                            state.show_layer();
                        } else {
                            state.hide_layer();
                        }
                    }
                }
            }
            // zwlr_foreign_toplevel_handle_v1::Event::Done => todo!(),
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.toplevels.remove(&handle.id());
            }
            // zwlr_foreign_toplevel_handle_v1::Event::Parent { parent } => todo!(),
            _ => (),
        }
    }

    fn event_created_child(
        _opcode: u16,
        qhandle: &QueueHandle<Self>,
    ) -> std::sync::Arc<dyn wayland_backend::client::ObjectData> {
        qhandle.make_data::<ZwlrForeignToplevelHandleV1, ()>(())
    }
}

delegate_compositor!(OpenCrosshair);
delegate_output!(OpenCrosshair);
delegate_shm!(OpenCrosshair);

delegate_layer!(OpenCrosshair);

delegate_registry!(OpenCrosshair);

impl ShmHandler for OpenCrosshair {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

fn get_outputs_with_info(conn: &Connection) -> Vec<(WlOutput, Option<OutputInfo>)> {
    // Now create an event queue and a handle to the queue so we can create objects.
    let (globals, mut event_queue) = registry_queue_init(conn).unwrap();
    let qh = event_queue.handle();

    // Initialize the registry handling so other parts of Smithay's client toolkit may bind
    // globals.
    let registry_state = RegistryState::new(&globals);

    // Initialize the delegate we will use for outputs.
    let output_delegate = OutputState::new(&globals, &qh);

    // Set up application state.
    //
    // This is where you will store your delegates and any data you wish to access/mutate while the
    // application is running.
    let mut list_outputs = ListOutputs {
        registry_state,
        output_state: output_delegate,
    };

    // `OutputState::new()` binds the output globals found in `registry_queue_init()`.
    //
    // After the globals are bound, we need to dispatch again so that events may be sent to the newly
    // created objects.
    event_queue.roundtrip(&mut list_outputs).unwrap();

    // Now our outputs have been initialized with data, we may access what outputs exist and information about
    // said outputs using the output delegate.
    let mut outputs = Vec::new();
    for output in list_outputs.output_state.outputs() {
        let info = list_outputs.output_state.info(&output);
        outputs.push((output, info));
    }
    outputs
}

// Copy pasted from smithay-client-toolkit examples/list_outputs.rs
struct ListOutputs {
    registry_state: RegistryState,
    output_state: OutputState,
}

// In order to use OutputDelegate, we must implement this trait to indicate when something has happened to an
// output and to provide an instance of the output state to the delegate when dispatching events.
impl OutputHandler for ListOutputs {
    // First we need to provide a way to access the delegate.
    //
    // This is needed because delegate implementations for handling events use the application data type in
    // their function signatures. This allows the implementation to access an instance of the type.
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    // Then there exist these functions that indicate the lifecycle of an output.
    // These will be called as appropriate by the delegate implementation.

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

// Now we need to say we are delegating the responsibility of output related events for our application data
// type to the requisite delegate.
delegate_output!(ListOutputs);

// In order for our delegate to know of the existence of globals, we need to implement registry
// handling for the program. This trait will forward events to the RegistryHandler trait
// implementations.
delegate_registry!(ListOutputs);

// In order for delegate_registry to work, our application data type needs to provide a way for the
// implementation to access the registry state.
//
// We also need to indicate which delegates will get told about globals being created. We specify
// the types of the delegates inside the array.
impl ProvidesRegistryState for ListOutputs {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers! {
        // Here we specify that OutputState needs to receive events regarding the creation and destruction of
        // globals.
        OutputState,
    }
}
