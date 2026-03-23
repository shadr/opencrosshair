use smithay_client_toolkit::{
    delegate_output, delegate_registry,
    output::{OutputHandler, OutputInfo, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
};
use wayland_client::{
    Connection, QueueHandle,
    protocol::wl_output::{self, WlOutput},
};
use wayland_client::globals::registry_queue_init;

pub fn get_outputs_with_info(conn: &Connection) -> Vec<(WlOutput, Option<OutputInfo>)> {
    let (globals, mut event_queue) = registry_queue_init(conn).unwrap();
    let qh = event_queue.handle();

    let registry_state = RegistryState::new(&globals);
    let output_delegate = OutputState::new(&globals, &qh);

    let mut list_outputs = ListOutputs {
        registry_state,
        output_state: output_delegate,
    };

    event_queue.roundtrip(&mut list_outputs).unwrap();

    let mut outputs = Vec::new();
    for output in list_outputs.output_state.outputs() {
        let info = list_outputs.output_state.info(&output);
        outputs.push((output, info));
    }
    outputs
}

pub struct ListOutputs {
    pub registry_state: RegistryState,
    pub output_state: OutputState,
}

impl OutputHandler for ListOutputs {
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

delegate_output!(ListOutputs);
delegate_registry!(ListOutputs);

impl ProvidesRegistryState for ListOutputs {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers! {
        OutputState,
    }
}
