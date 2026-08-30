//! Nested-session input-method acceptance probe.
//!
//! Connect this process to an explicitly selected nested Nickel compositor. It
//! waits for a real `zwp_text_input_v3` client to activate, publishes a visible
//! preedit marker, commits a second marker, and exits. The probe never connects
//! to or changes the host input-method service.

use std::{
    cell::Cell,
    error::Error,
    thread,
    time::{Duration, Instant},
};

use smithay_client_toolkit::{
    delegate_input_method, delegate_registry, delegate_seat,
    reexports::client::{Connection, QueueHandle, globals::registry_queue_init, protocol::wl_seat},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        input_method::{
            Active, CursorPosition, InputMethod, InputMethodEventState, InputMethodHandler,
            InputMethodManager, ZwpInputMethodV2,
        },
    },
};

const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(5);
const PREEDIT_OBSERVATION: Duration = Duration::from_secs(2);

struct Probe {
    registry_state: RegistryState,
    seat_state: SeatState,
    input_method: InputMethod,
    active: Cell<bool>,
    unavailable: Cell<bool>,
}

impl InputMethodHandler for Probe {
    fn handle_done(
        &self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _input_method: &ZwpInputMethodV2,
        state: &InputMethodEventState,
    ) {
        self.active
            .set(matches!(state.active, Active::Active { .. }));
    }

    fn handle_unavailable(
        &self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _input_method: &ZwpInputMethodV2,
    ) {
        self.unavailable.set(true);
    }
}

impl SeatHandler for Probe {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: Capability,
    ) {
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        _: Capability,
    ) {
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl ProvidesRegistryState for Probe {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers!(SeatState);
}

delegate_input_method!(Probe);
delegate_seat!(Probe);
delegate_registry!(Probe);

fn main() -> Result<(), Box<dyn Error>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init(&connection)?;
    let qh = event_queue.handle();
    let seat_state = SeatState::new(&globals, &qh);
    let seat = seat_state
        .seats()
        .next()
        .ok_or("nested compositor exposes no seat")?;
    let manager = InputMethodManager::bind(&globals, &qh)?;
    let input_method = manager.get_input_method(&qh, &seat);
    let mut probe = Probe {
        registry_state: RegistryState::new(&globals),
        seat_state,
        input_method,
        active: Cell::new(false),
        unavailable: Cell::new(false),
    };

    let deadline = Instant::now() + ACTIVATION_TIMEOUT;
    while !probe.active.get() && !probe.unavailable.get() && Instant::now() < deadline {
        event_queue.roundtrip(&mut probe)?;
        thread::sleep(Duration::from_millis(10));
    }
    if probe.unavailable.get() {
        return Err("another input method already owns the nested seat".into());
    }
    if !probe.active.get() {
        return Err("no focused text-input client activated before the deadline".into());
    }

    probe.input_method.set_preedit_string(
        "ime-preedit".into(),
        CursorPosition::Visible { start: 11, end: 11 },
    );
    probe.input_method.commit();
    connection.flush()?;
    println!("PREEDIT_SENT");
    thread::sleep(PREEDIT_OBSERVATION);

    probe
        .input_method
        .set_preedit_string(String::new(), CursorPosition::Hidden);
    probe.input_method.commit_string("ime-commit".into());
    probe.input_method.commit();
    connection.flush()?;
    println!("COMMIT_SENT");
    thread::sleep(Duration::from_millis(250));
    Ok(())
}
