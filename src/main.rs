mod app;
mod color;
mod list_outputs;
mod renderer;

use clap::Parser;
use image::{ImageBuffer, Rgba};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::shell::{
    WaylandSurface,
    wlr_layer::{KeyboardInteractivity, Layer, LayerShell},
};
use smithay_client_toolkit::shm::{Shm, slot::SlotPool};
use std::path::PathBuf;
use std::ptr::NonNull;
use wayland_client::{Connection, Proxy, globals::registry_queue_init};

use crate::app::App;
use crate::color::parse_hex_color;

#[derive(Parser)]
#[command(name = "opencrosshair")]
#[command(about = "A crosshair overlay for Wayland compositors", long_about = None)]
struct Args {
    /// App IDs for which the crosshair should be visible (comma-separated)
    #[arg(short = 'a', long = "app-ids", value_delimiter = ',')]
    app_ids: Vec<String>,

    /// Color of the crosshair in hex format (e.g., "FF0000" or "#FF0000" for red, "#FF0000FF" for red with alpha)
    #[arg(short = 'c', long = "color", default_value_t = String::from("#FF0000"))]
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

    let color_rgba = parse_hex_color(&args.color).unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    });

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
    let compositor_state =
        CompositorState::bind(&globals, &qh).expect("wl_compositor not available");

    let region = compositor_state.wl_compositor().create_region(&qh, ());
    region.add(0, 0, 0, 0);
    let surface = compositor_state.create_surface(&qh);

    let mut output = None;
    let outputs = list_outputs::get_outputs_with_info(&conn);
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

    let (size_x, size_y) = diffuse_rgba.dimensions();
    layer.set_size(size_x, size_y);
    layer.set_exclusive_zone(-1);
    layer.commit();

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });

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

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .expect("Failed to find suitable adapter");

    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
        .expect("Failed to request device");

    let shm = Shm::bind(&globals, &qh).expect("wl_shm not available");
    let mut slot_pool = SlotPool::new(4096, &shm).expect("Failed to create slot pool");

    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let bytes_per_row_unaligned = size_x * 4;
    let bytes_per_row = (bytes_per_row_unaligned + align - 1) & !(align - 1);

    let (drawn_buffer, _) = slot_pool
        .create_buffer(
            size_x as i32,
            size_y as i32,
            bytes_per_row as i32,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        )
        .unwrap();
    let (empty_buffer, _) = slot_pool
        .create_buffer(
            size_x as i32,
            size_y as i32,
            bytes_per_row as i32,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        )
        .unwrap();

    let registry_state = RegistryState::new(&globals);
    let output_state = OutputState::new(&globals, &qh);

    let mut app = App::new(
        layer,
        visible_app_ids,
        diffuse_rgba,
        color_rgba,
        scale,
        size_x,
        size_y,
        registry_state,
        output_state,
        adapter,
        device,
        queue,
        surface,
        shm,
        slot_pool,
        drawn_buffer,
        empty_buffer,
    );

    app.registry_state
        .bind_one::<smithay_client_toolkit::reexports::protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, _, _>(
            &qh, 2..=3, (),
        )
        .unwrap();

    loop {
        event_queue.blocking_dispatch(&mut app).unwrap();

        if app.exit {
            println!("exiting example");
            break;
        }
    }

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
