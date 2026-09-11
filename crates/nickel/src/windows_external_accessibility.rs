//! Bounded external UI Automation observation. COM objects never leave the
//! worker and provider strings are deliberately never requested.
#![cfg(target_os = "windows")]

use nickel_remote_control::native_semantics::{
    NativeSemanticNode, NativeSemanticScope, NativeSemanticSnapshot,
};
use std::time::{Duration, Instant};

const MAX_NODES: usize = 128;
const MAX_DEPTH: usize = 16;

fn clipped_bounds(
    rect: windows::Win32::Foundation::RECT,
    scope: crate::windows_resource_owner::Rect,
) -> Option<[i32; 4]> {
    let scope_right = i64::from(scope.x) + i64::from(scope.width);
    let scope_bottom = i64::from(scope.y) + i64::from(scope.height);
    let left = i64::from(rect.left).max(i64::from(scope.x));
    let top = i64::from(rect.top).max(i64::from(scope.y));
    let right = i64::from(rect.right).min(scope_right);
    let bottom = i64::from(rect.bottom).min(scope_bottom);
    if right <= left || bottom <= top {
        return None;
    }
    Some([
        i32::try_from(left - i64::from(scope.x)).ok()?,
        i32::try_from(top - i64::from(scope.y)).ok()?,
        i32::try_from(right - left).ok()?,
        i32::try_from(bottom - top).ok()?,
    ])
}

#[derive(Clone)]
pub(crate) struct Proof {
    pub id: String,
    pub generation: u64,
    pub native: usize,
    pub pid: u32,
    pub created: u64,
    pub thread: u32,
    pub bounds: crate::windows_resource_owner::Rect,
}
pub(crate) struct Observation {
    pub snapshot: NativeSemanticSnapshot,
    pub started: Instant,
    pub completed: Instant,
}
pub(crate) struct Admission;
static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
impl Admission {
    pub(crate) fn acquire() -> Result<Self, String> {
        BUSY.compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .map_err(|_| "Windows UI Automation observation is busy".to_owned())?;
        Ok(Self)
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        BUSY.store(false, std::sync::atomic::Ordering::Release);
    }
}

fn live(permit: &nickel_remote_control::DesktopPermit, deadline: Instant) -> Result<(), String> {
    permit.check_live()?;
    (Instant::now() < deadline)
        .then_some(())
        .ok_or_else(|| "Windows UI Automation observation timed out".into())
}

pub(crate) fn observe(
    proof: &Proof,
    permit: &nickel_remote_control::DesktopPermit,
    deadline: Instant,
) -> Result<Observation, String> {
    use windows::Win32::{
        Foundation::HWND,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::Accessibility::{CUIAutomation, IUIAutomation},
    };
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    let started = Instant::now();
    live(permit, deadline)?;
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|_| "Windows UI Automation COM initialization failed".to_owned())?;
    let _com = Com;
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| "Windows UI Automation is unavailable".to_owned())?;
    let root = unsafe { automation.ElementFromHandle(HWND(proof.native as *mut _)) }
        .map_err(|_| "Windows UI Automation window is unavailable".to_owned())?;
    if unsafe { root.CurrentProcessId() }
        .ok()
        .and_then(|pid| u32::try_from(pid).ok())
        != Some(proof.pid)
    {
        return Err("Windows UI Automation process identity changed".into());
    }
    let walker = unsafe { automation.ControlViewWalker() }
        .map_err(|_| "Windows UI Automation control view is unavailable".to_owned())?;
    let mut nodes = Vec::with_capacity(MAX_NODES);
    let mut truncated = false;
    walk(
        &root,
        None,
        0,
        &walker,
        proof,
        permit,
        deadline,
        &mut nodes,
        &mut truncated,
    )?;
    let completed = Instant::now();
    live(permit, deadline)?;
    Ok(Observation {
        snapshot: NativeSemanticSnapshot {
            scope: NativeSemanticScope::Window,
            window: proof.id.clone(),
            window_generation: proof.generation,
            association_generation: None,
            observation_generation: 0,
            observed_at_us: 0,
            observation_started_at_us: 0,
            owner_validated_at_us: 0,
            atomic: false,
            unavailable_fields: vec![
                "names".into(),
                "descriptions".into(),
                "text".into(),
                "values".into(),
                "actions".into(),
            ],
            nodes,
            truncated,
        },
        started,
        completed,
    })
}

#[allow(clippy::too_many_arguments)]
fn walk(
    element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    parent: Option<u32>,
    depth: usize,
    walker: &windows::Win32::UI::Accessibility::IUIAutomationTreeWalker,
    proof: &Proof,
    permit: &nickel_remote_control::DesktopPermit,
    deadline: Instant,
    nodes: &mut Vec<NativeSemanticNode>,
    truncated: &mut bool,
) -> Result<(), String> {
    live(permit, deadline)?;
    if nodes.len() == MAX_NODES || depth == MAX_DEPTH {
        *truncated = true;
        return Ok(());
    }
    if unsafe { element.CurrentProcessId() }
        .ok()
        .and_then(|pid| u32::try_from(pid).ok())
        != Some(proof.pid)
    {
        return Ok(());
    }
    let id = nodes.len() as u32;
    let rect = unsafe { element.CurrentBoundingRectangle() }.ok();
    nodes.push(NativeSemanticNode {
        id,
        parent,
        role: unsafe { element.CurrentControlType() }
            .ok()
            .and_then(|role| u32::try_from(role.0).ok())
            .unwrap_or_default(),
        bounds: rect.and_then(|rect| clipped_bounds(rect, proof.bounds)),
        enabled: unsafe { element.CurrentIsEnabled() }.is_ok_and(|value| value.as_bool()),
        focused: unsafe { element.CurrentHasKeyboardFocus() }.is_ok_and(|value| value.as_bool()),
    });
    let mut child = unsafe { walker.GetFirstChildElement(element) }.ok();
    while let Some(current) = child {
        walk(
            &current,
            Some(id),
            depth + 1,
            walker,
            proof,
            permit,
            deadline,
            nodes,
            truncated,
        )?;
        if *truncated {
            break;
        }
        child = unsafe { walker.GetNextSiblingElement(&current) }.ok();
    }
    Ok(())
}

pub(crate) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::RECT;

    #[test]
    fn provider_geometry_is_clipped_and_made_window_relative() {
        let scope = crate::windows_resource_owner::Rect {
            x: 100,
            y: 50,
            width: 200,
            height: 100,
        };
        assert_eq!(
            clipped_bounds(
                RECT {
                    left: 80,
                    top: 60,
                    right: 140,
                    bottom: 200
                },
                scope
            ),
            Some([0, 10, 40, 90])
        );
        assert_eq!(
            clipped_bounds(
                RECT {
                    left: -20,
                    top: -20,
                    right: 0,
                    bottom: 0
                },
                scope
            ),
            None
        );
    }
}
