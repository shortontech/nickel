//! Bounded external UI Automation observation. COM objects never leave the
//! worker and provider strings are deliberately never requested.
#![cfg(target_os = "windows")]

use nickel_remote_control::native_semantics::{
    NativeSemanticNode, NativeSemanticScope, NativeSemanticSnapshot,
};
use std::time::{Duration, Instant};

const MAX_NODES: usize = 128;
const MAX_DEPTH: usize = 16;
pub(crate) const MAX_WINDOWS: usize = 16;

fn clipped_bounds(
    rect: windows::Win32::Foundation::RECT,
    owner: crate::windows_resource_owner::Rect,
    origin: crate::windows_resource_owner::Rect,
) -> Option<[i32; 4]> {
    let scope_right = i64::from(owner.x) + i64::from(owner.width);
    let scope_bottom = i64::from(owner.y) + i64::from(owner.height);
    let left = i64::from(rect.left).max(i64::from(owner.x));
    let top = i64::from(rect.top).max(i64::from(owner.y));
    let right = i64::from(rect.right).min(scope_right);
    let bottom = i64::from(rect.bottom).min(scope_bottom);
    if right <= left || bottom <= top {
        return None;
    }
    Some([
        i32::try_from(left - i64::from(origin.x)).ok()?,
        i32::try_from(top - i64::from(origin.y)).ok()?,
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
    pub application: Option<String>,
    pub roots: Vec<RootProof>,
    pub application_wide: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RootProof {
    pub id: String,
    pub generation: u64,
    pub native: usize,
    pub pid: u32,
    pub created: u64,
    pub thread: u32,
    pub bounds: crate::windows_resource_owner::Rect,
}

pub(crate) fn same_roots(expected: &[RootProof], current: &[RootProof]) -> bool {
    expected == current
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
    let walker = unsafe { automation.ControlViewWalker() }
        .map_err(|_| "Windows UI Automation control view is unavailable".to_owned())?;
    let mut nodes = Vec::with_capacity(MAX_NODES);
    let mut truncated = false;
    for root_proof in &proof.roots {
        live(permit, deadline)?;
        let root = unsafe { automation.ElementFromHandle(HWND(root_proof.native as *mut _)) }
            .map_err(|_| "Windows UI Automation window is unavailable".to_owned())?;
        if unsafe { root.CurrentProcessId() }
            .ok()
            .and_then(|pid| u32::try_from(pid).ok())
            != Some(root_proof.pid)
        {
            return Err("Windows UI Automation process identity changed".into());
        }
        walk(
            &root,
            None,
            0,
            &walker,
            root_proof,
            proof.bounds,
            permit,
            deadline,
            &mut nodes,
            &mut truncated,
        )?;
        if truncated {
            break;
        }
    }
    let completed = Instant::now();
    live(permit, deadline)?;
    Ok(Observation {
        snapshot: NativeSemanticSnapshot {
            scope: if proof.application_wide {
                NativeSemanticScope::ApplicationConnection
            } else {
                NativeSemanticScope::Window
            },
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
    proof: &RootProof,
    origin: crate::windows_resource_owner::Rect,
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
        bounds: rect.and_then(|rect| clipped_bounds(rect, proof.bounds, origin)),
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
            origin,
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
                scope,
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
                scope,
                scope
            ),
            None
        );
    }

    #[test]
    fn application_geometry_uses_one_anchor_coordinate_space() {
        let owner = crate::windows_resource_owner::Rect {
            x: -300,
            y: 100,
            width: 200,
            height: 100,
        };
        let anchor = crate::windows_resource_owner::Rect {
            x: 100,
            y: 50,
            width: 200,
            height: 100,
        };
        assert_eq!(
            clipped_bounds(
                RECT {
                    left: -350,
                    top: 90,
                    right: -200,
                    bottom: 150,
                },
                owner,
                anchor,
            ),
            Some([-400, 50, 100, 50])
        );
    }

    #[test]
    fn application_root_set_changes_fail_exact_comparison() {
        let root = RootProof {
            id: "7".into(),
            generation: 9,
            native: 11,
            pid: 13,
            created: 15,
            thread: 17,
            bounds: crate::windows_resource_owner::Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 20,
            },
        };
        assert!(same_roots(
            std::slice::from_ref(&root),
            std::slice::from_ref(&root)
        ));
        let mut changed = root.clone();
        changed.created += 1;
        assert!(!same_roots(
            std::slice::from_ref(&root),
            std::slice::from_ref(&changed)
        ));
        assert!(!same_roots(std::slice::from_ref(&root), &[]));
        assert!(!same_roots(&[], std::slice::from_ref(&root)));
    }
}
