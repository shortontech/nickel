//! Narrow Windows display service for the JSX plugin data/effect boundary.
//! Only whole-layout position and primary changes are supported. Native
//! DisplayConfig validation, temporary application, and recovery live in
//! `windows_remote_display_topology`.

use crate::windows_remote_display_topology::{self as topology, Observation, RecoveryPlan};
use nickel_remote_control::diagnostics::{OutputDiagnostic, OutputInventory};
use nickel_session_protocol::{
    Geometry, OutputLayout, OutputMode, OutputSnapshot, OutputTransform,
};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Gdi::{DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW};
use windows::core::PCWSTR;

const RECOVERY_WINDOW: Duration = Duration::from_secs(15);

pub(crate) struct DisplayRead {
    pub available: bool,
    pub reason: Option<String>,
    pub outputs: Vec<OutputSnapshot>,
    pub pending_confirmation: bool,
}

struct Pending {
    owner: String,
    plan: RecoveryPlan,
    deadline: Instant,
}

#[derive(Default)]
struct State {
    pending: Option<Pending>,
    recovery_error: Option<String>,
}

fn state() -> &'static Arc<Mutex<State>> {
    static STATE: OnceLock<Arc<Mutex<State>>> = OnceLock::new();
    STATE.get_or_init(|| Arc::new(Mutex::new(State::default())))
}

fn current_mode(name: &str) -> Result<(OutputMode, OutputTransform), String> {
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let mut mode = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    // SAFETY: The NUL-terminated name and writable mode outlive this call.
    if !unsafe { EnumDisplaySettingsW(PCWSTR(wide.as_ptr()), ENUM_CURRENT_SETTINGS, &raw mut mode) }
        .as_bool()
    {
        return Err(format!("Windows display mode is unavailable for {name}"));
    }
    // SAFETY: EnumDisplaySettingsW initialized the display union.
    let orientation = unsafe { mode.Anonymous1.Anonymous2.dmDisplayOrientation };
    let transform = match orientation.0 {
        0 => OutputTransform::Normal,
        1 => OutputTransform::Rotate90,
        2 => OutputTransform::Rotate180,
        3 => OutputTransform::Rotate270,
        _ => return Err("Windows display orientation is unknown".into()),
    };
    Ok((
        OutputMode {
            width: i32::try_from(mode.dmPelsWidth)
                .map_err(|_| "Windows display width is invalid")?,
            height: i32::try_from(mode.dmPelsHeight)
                .map_err(|_| "Windows display height is invalid")?,
            refresh_millihz: i32::try_from(mode.dmDisplayFrequency.saturating_mul(1000))
                .map_err(|_| "Windows display refresh rate is invalid")?,
        },
        transform,
    ))
}

fn inventory() -> Result<(OutputInventory, Vec<OutputSnapshot>), String> {
    let observed = crate::platform::remote_observation::outputs()?;
    if observed.len() > nickel_remote_control::diagnostics::MAX_DIAGNOSTIC_OUTPUTS {
        return Err("Windows display inventory exceeds the supported output limit".into());
    }
    let mut diagnostics = Vec::with_capacity(observed.len());
    let mut snapshots = Vec::with_capacity(observed.len());
    for (index, output) in observed.into_iter().enumerate() {
        let width =
            i32::try_from(output.bounds.width).map_err(|_| "Windows display width is invalid")?;
        let height =
            i32::try_from(output.bounds.height).map_err(|_| "Windows display height is invalid")?;
        let work_width = i32::try_from(output.work_area.width)
            .map_err(|_| "Windows work area width is invalid")?;
        let work_height = i32::try_from(output.work_area.height)
            .map_err(|_| "Windows work area height is invalid")?;
        let (mode, transform) = current_mode(&output.name)?;
        diagnostics.push(OutputDiagnostic {
            name: output.name.clone(),
            generation: index as u64 + 1,
            geometry: [output.bounds.x, output.bounds.y, width, height],
            work_area: [
                output.work_area.x,
                output.work_area.y,
                work_width,
                work_height,
            ],
            scale_120: output.scale_120,
            primary: output.primary,
            enabled: true,
        });
        snapshots.push(OutputSnapshot {
            name: output.name.clone(),
            model: output.name,
            geometry: Geometry {
                x: output.bounds.x,
                y: output.bounds.y,
                width,
                height,
            },
            work_area: Geometry {
                x: output.work_area.x,
                y: output.work_area.y,
                width: work_width,
                height: work_height,
            },
            scale_120: output.scale_120,
            transform,
            physical_width_mm: 0,
            physical_height_mm: 0,
            primary: output.primary,
            enabled: true,
            modes: vec![mode],
            current_mode: Some(mode),
        });
    }
    Ok((
        OutputInventory {
            observation_generation: 1,
            observed_at_us: 0,
            topology_generation: 1,
            outputs: diagnostics,
            truncated: false,
        },
        snapshots,
    ))
}

pub(crate) fn read() -> DisplayRead {
    let (pending_confirmation, recovery_error) = state()
        .lock()
        .map(|state| (state.pending.is_some(), state.recovery_error.clone()))
        .unwrap_or((
            true,
            Some("Windows display recovery owner is unavailable".into()),
        ));
    match inventory() {
        Ok((inventory, outputs)) => match topology::observe(&inventory) {
            Ok(observed) => DisplayRead {
                available: observed.transaction_supported && recovery_error.is_none(),
                reason: recovery_error.or(observed.transaction_unavailable_reason),
                outputs,
                pending_confirmation,
            },
            Err(reason) => DisplayRead {
                available: false,
                reason: Some(reason),
                outputs,
                pending_confirmation,
            },
        },
        Err(reason) => DisplayRead {
            available: false,
            reason: Some(reason),
            outputs: Vec::new(),
            pending_confirmation,
        },
    }
}

fn requested_layout(
    requested: &OutputLayout,
    observed: &Observation,
    snapshots: &[OutputSnapshot],
) -> Result<nickel_remote_control::display_layout::Layout, String> {
    let mut layout = observed.layout.clone();
    if requested.placements.len() != layout.outputs.len() {
        return Err("Windows display request must include every active output".into());
    }
    let primary = observed
        .native_names
        .iter()
        .find(|(_, name)| *name == &requested.primary)
        .map(|(id, _)| id.clone())
        .ok_or("Windows primary display is unknown")?;
    let primary_placement = requested
        .placements
        .iter()
        .find(|placement| placement.name == requested.primary)
        .ok_or("Windows primary display is missing from the layout")?;
    let mut seen = BTreeSet::new();
    for placement in &requested.placements {
        if !seen.insert(&placement.name) {
            return Err("Windows display request contains duplicate outputs".into());
        }
        let snapshot = snapshots
            .iter()
            .find(|output| output.name == placement.name)
            .ok_or("Windows display request contains an unknown output")?;
        if !placement.enabled || placement.scale_120 != snapshot.scale_120 {
            return Err("Windows display enable and scale changes are unavailable".into());
        }
        if placement
            .mode
            .is_some_and(|mode| Some(mode) != snapshot.current_mode)
        {
            return Err("Windows display mode changes are unavailable".into());
        }
        let id = observed
            .native_names
            .iter()
            .find(|(_, name)| *name == &placement.name)
            .map(|(id, _)| id)
            .ok_or("Windows display identity is unavailable")?;
        let output = layout
            .outputs
            .iter_mut()
            .find(|output| &output.output.id == id)
            .ok_or("Windows display output retired")?;
        output.x = placement
            .x
            .checked_sub(primary_placement.x)
            .ok_or("Windows display X position exceeds native bounds")?;
        output.y = placement
            .y
            .checked_sub(primary_placement.y)
            .ok_or("Windows display Y position exceeds native bounds")?;
    }
    layout.primary = layout
        .outputs
        .iter()
        .find(|output| output.output.id == primary)
        .ok_or("Windows primary display retired")?
        .output
        .clone();
    Ok(layout)
}

pub(crate) fn set_layout(owner: &str, requested: &OutputLayout) -> Result<(), String> {
    let mut state_guard = state()
        .lock()
        .map_err(|_| "Windows display recovery owner is unavailable")?;
    if state_guard.pending.is_some() {
        return Err("Windows display recovery is already pending".into());
    }
    if let Some(error) = &state_guard.recovery_error {
        return Err(format!("Windows display recovery needs attention: {error}"));
    }
    let (before_inventory, snapshots) = inventory()?;
    let observed = topology::observe(&before_inventory)?;
    if !observed.transaction_supported {
        return Err(observed
            .transaction_unavailable_reason
            .unwrap_or_else(|| "Windows display topology is unavailable".into()));
    }
    let layout = requested_layout(requested, &observed, &snapshots)?;
    let plan = topology::apply_position_change(
        &observed,
        &observed.layout,
        &layout,
        observed.topology_generation,
        || Ok(()),
    )
    .map_err(|failure| {
        if let Some(plan) = failure.recovery {
            if let Err(error) = plan.restore() {
                state_guard.recovery_error = Some(error);
            }
        }
        failure.reason
    })?;
    let verified = inventory()
        .and_then(|(inventory, _)| topology::observe(&inventory))
        .is_ok_and(|actual| topology::matches_physical_layout(&actual.layout, &layout));
    if !verified {
        if let Err(error) = plan.restore() {
            state_guard.recovery_error = Some(error.clone());
            return Err(format!(
                "Windows display readback did not match; recovery failed: {error}"
            ));
        }
        return Err("Windows display readback did not match the requested layout".into());
    }
    let deadline = Instant::now() + RECOVERY_WINDOW;
    state_guard.pending = Some(Pending {
        owner: owner.into(),
        plan,
        deadline,
    });
    drop(state_guard);
    let shared = Arc::clone(state());
    std::thread::spawn(move || {
        std::thread::sleep(RECOVERY_WINDOW);
        if let Ok(mut guard) = shared.lock()
            && guard
                .pending
                .as_ref()
                .is_some_and(|pending| Instant::now() >= pending.deadline)
            && let Some(pending) = guard.pending.take()
            && let Err(error) = pending.plan.restore()
        {
            guard.recovery_error = Some(error);
        }
    });
    Ok(())
}

pub(crate) fn confirm(owner: &str) -> Result<(), String> {
    let mut guard = state()
        .lock()
        .map_err(|_| "Windows display recovery owner is unavailable")?;
    let pending = guard
        .pending
        .as_ref()
        .ok_or("No Windows display change awaits confirmation")?;
    if pending.owner != owner {
        return Err("Another plugin owns the pending display change".into());
    }
    if Instant::now() >= pending.deadline {
        return Err("Windows display confirmation window expired".into());
    }
    pending.plan.persist()?;
    guard.pending = None;
    Ok(())
}

pub(crate) fn revert(owner: &str) -> Result<(), String> {
    let mut guard = state()
        .lock()
        .map_err(|_| "Windows display recovery owner is unavailable")?;
    let pending = guard
        .pending
        .as_ref()
        .ok_or("No Windows display change awaits recovery")?;
    if pending.owner != owner {
        return Err("Another plugin owns the pending display change".into());
    }
    pending.plan.restore()?;
    guard.pending = None;
    Ok(())
}
