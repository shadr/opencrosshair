use crate::renderer::Renderer;
use image::ImageBuffer;
use image::Rgba;
use smithay_client_toolkit::{
    compositor::CompositorHandler,
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{WaylandSurface, wlr_layer::{LayerShellHandler, LayerSurface, LayerSurfaceConfigure}},
    shm::{Shm, ShmHandler, slot::{Buffer, SlotPool}},
    reexports::protocols_wlr::foreign_toplevel::v1::client::{
        zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
        zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
    },
};
use std::collections::HashMap;
use wayland_backend::client::ObjectId;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    protocol::{
        wl_output::{self},
        wl_region, wl_surface,
    },
};

pub struct App {
    pub registry_state: RegistryState,
    pub output_state: OutputState,
    pub layer: LayerSurface,
    pub toplevels: HashMap<ObjectId, String>,
    pub visible_app_ids: Vec<String>,
    pub crosshair: ImageBuffer<Rgba<u8>, Vec<u8>>,
    pub color_rgba: [f32; 4],
    pub scale: f32,
    pub exit: bool,
    pub width: u32,
    pub height: u32,
    pub adapter: Option<wgpu::Adapter>,
    pub device: Option<wgpu::Device>,
    pub queue: Option<wgpu::Queue>,
    pub surface: Option<wgpu::Surface<'static>>,
    pub renderer: Renderer,
    pub shm: Shm,
    pub slot_pool: SlotPool,
    pub drawn_buffer: Buffer,
    pub empty_buffer: Buffer,
}

impl App {
    pub fn new(
        layer: LayerSurface,
        visible_app_ids: Vec<String>,
        crosshair: ImageBuffer<Rgba<u8>, Vec<u8>>,
        color_rgba: [f32; 4],
        scale: f32,
        width: u32,
        height: u32,
        registry_state: RegistryState,
        output_state: OutputState,
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        shm: Shm,
        slot_pool: SlotPool,
        drawn_buffer: Buffer,
        empty_buffer: Buffer,
    ) -> Self {
        Self {
            registry_state,
            output_state,
            layer,
            toplevels: HashMap::new(),
            visible_app_ids,
            crosshair,
            color_rgba,
            scale,
            exit: false,
            width,
            height,
            adapter: Some(adapter),
            device: Some(device),
            queue: Some(queue),
            surface: Some(surface),
            renderer: Renderer::new(width, height),
            shm,
            slot_pool,
            drawn_buffer,
            empty_buffer,
        }
    }

    pub fn hide_layer(&mut self) {
        self.layer.set_size(self.width, self.height);
        self.layer.attach(Some(self.empty_buffer.wl_buffer()), 0, 0);
        self.layer.commit();
    }

    pub fn show_layer(&mut self) {
        self.layer.set_size(self.width, self.height);
        self.layer.attach(Some(self.drawn_buffer.wl_buffer()), 0, 0);
        self.layer.commit();
    }

    pub fn initialize_resources(&mut self) {
        let device = self.device.as_ref().unwrap();
        let queue = self.queue.as_ref().unwrap();
        let adapter = self.adapter.as_ref().unwrap();
        let surface = self.surface.as_ref().unwrap();

        self.renderer.initialize(
            device,
            queue,
            adapter,
            surface,
            &self.crosshair,
            self.scale,
        );
    }

    pub fn render_frame(&mut self) {
        let device = self.device.as_ref().unwrap();
        let queue = self.queue.as_ref().unwrap();
        let surface = self.surface.as_ref().unwrap();

        self.drawn_buffer = self.renderer.render(
            device,
            queue,
            surface,
            self.color_rgba,
            &mut self.slot_pool,
        );

        self.renderer.cleanup();
        self.adapter = None;
        self.device = None;
        self.queue = None;
        self.surface = None;
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
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
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
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

impl LayerShellHandler for App {
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
        let device = self.device.as_ref().unwrap();

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

        surface.configure(device, &surface_config);

        if !self.renderer.is_initialized() {
            self.initialize_resources();
        }

        self.render_frame();
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

impl Dispatch<wl_region::WlRegion, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_region::WlRegion,
        _: wl_region::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        _: <ZwlrForeignToplevelManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }

    fn event_created_child(
        _opcode: u16,
        qhandle: &QueueHandle<Self>,
    ) -> std::sync::Arc<dyn wayland_backend::client::ObjectData> {
        qhandle.make_data::<ZwlrForeignToplevelHandleV1, ()>(())
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for App {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: <ZwlrForeignToplevelHandleV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                state.toplevels.insert(handle.id(), app_id);
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: states } => {
                if states
                    .chunks_exact(4)
                    .any(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) == 2)
                {
                    let handle_id = handle.id();
                    if let Some(app_id) = state.toplevels.get(&handle_id) {
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
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.toplevels.remove(&handle.id());
            }
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

delegate_compositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_layer!(App);
delegate_registry!(App);

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
