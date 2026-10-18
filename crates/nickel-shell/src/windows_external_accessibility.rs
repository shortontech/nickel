//! Bounded external UI Automation observation. COM objects never leave the
//! worker and provider strings are deliberately never requested.
#![cfg(target_os = "windows")]

use nickel_remote_control::native_semantics::{
    NativeSemanticAction, NativeSemanticActionOutcome, NativeSemanticNode, NativeSemanticScope,
    NativeSemanticSnapshot,
};
use std::{
    sync::atomic::{AtomicU8, Ordering},
    time::{Duration, Instant},
};

const MAX_NODES: usize = nickel_remote_control::native_semantics::MAX_NATIVE_SEMANTIC_NODES;
const MAX_DEPTH: usize = 16;
const MAX_RUNTIME_ID_PARTS: usize = 32;
pub(crate) const MAX_WINDOWS: usize = 16;
pub(crate) const MAX_ACTION_OBSERVATIONS: usize = 8;
pub(crate) const ACTION_PENDING: u8 = 0;
pub(crate) const ACTION_ATTEMPTED: u8 = 1;
pub(crate) const ACTION_CANCELLED: u8 = 2;

pub(crate) fn commit_action_dispatch(
    dispatch: &AtomicU8,
    expected_local_input_epoch: u64,
    current_local_input_epoch: u64,
    now: Instant,
    deadline: Instant,
) -> Result<(), String> {
    if now >= deadline {
        return Err("Windows UI Automation action timed out before dispatch".into());
    }
    if current_local_input_epoch != expected_local_input_epoch {
        return Err("physical input cancelled the native semantic action".into());
    }
    dispatch
        .compare_exchange(
            ACTION_PENDING,
            ACTION_ATTEMPTED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map(|_| ())
        .map_err(|_| "Windows UI Automation action was cancelled before dispatch".into())
}

pub(crate) fn cancel_action_dispatch(dispatch: &AtomicU8) -> bool {
    dispatch
        .compare_exchange(
            ACTION_PENDING,
            ACTION_CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

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
    pub actions: Vec<ActionProof>,
    pub started: Instant,
    pub completed: Instant,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActionProof {
    pub node: u32,
    pub root: RootProof,
    pub runtime_id: Vec<i32>,
    pub action: NativeSemanticAction,
}
#[derive(Clone)]
pub(crate) struct ActionObservation {
    pub observation_generation: u64,
    pub proof: Proof,
    pub actions: Vec<ActionProof>,
}
#[derive(Clone)]
pub(crate) struct ActionPlan {
    pub observation_generation: u64,
    pub proof: Proof,
    pub target: ActionProof,
}
pub(crate) struct ActionAuthorization {
    /// Keeps shared input reserved through the provider call when authority did
    /// not end immediately after the already committed transition.
    pub input: Option<nickel_remote_control::HeldInput>,
}
impl ActionObservation {
    pub(crate) fn plan(
        &self,
        request: &nickel_remote_control::native_semantics::NativeSemanticActionRequest,
    ) -> Option<ActionPlan> {
        let scope = if self.proof.application_wide {
            NativeSemanticScope::ApplicationConnection
        } else {
            NativeSemanticScope::Window
        };
        if self.observation_generation != request.observation_generation
            || self.proof.id != request.window_id
            || self.proof.generation != request.window_generation
            || scope != request.scope
        {
            return None;
        }
        let target = self
            .actions
            .iter()
            .find(|action| action.node == request.node && action.action == request.action)?
            .clone();
        Some(ActionPlan {
            observation_generation: self.observation_generation,
            proof: self.proof.clone(),
            target,
        })
    }
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
    let mut actions = Vec::new();
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
            &mut actions,
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
                "non_invoke_actions".into(),
            ],
            nodes,
            truncated,
        },
        actions,
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
    actions: &mut Vec<ActionProof>,
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
    let enabled = unsafe { element.CurrentIsEnabled() }.is_ok_and(|value| value.as_bool());
    let runtime_id = enabled.then(|| runtime_id(element)).flatten();
    let supports_invoke = runtime_id.is_some()
        && unsafe {
            element.GetCurrentPatternAs::<
                windows::Win32::UI::Accessibility::IUIAutomationInvokePattern,
            >(windows::Win32::UI::Accessibility::UIA_InvokePatternId)
        }
        .is_ok();
    let advertised = if supports_invoke {
        vec![NativeSemanticAction::Invoke]
    } else {
        Vec::new()
    };
    nodes.push(NativeSemanticNode {
        id,
        parent,
        role: unsafe { element.CurrentControlType() }
            .ok()
            .and_then(|role| u32::try_from(role.0).ok())
            .unwrap_or_default(),
        bounds: rect.and_then(|rect| clipped_bounds(rect, proof.bounds, origin)),
        enabled,
        focused: unsafe { element.CurrentHasKeyboardFocus() }.is_ok_and(|value| value.as_bool()),
        actions: advertised,
    });
    if let Some(runtime_id) = runtime_id
        && supports_invoke
    {
        actions.push(ActionProof {
            node: id,
            root: proof.clone(),
            runtime_id,
            action: NativeSemanticAction::Invoke,
        });
    }
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
            actions,
            truncated,
        )?;
        if *truncated {
            break;
        }
        child = unsafe { walker.GetNextSiblingElement(&current) }.ok();
    }
    Ok(())
}

fn runtime_id(
    element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
) -> Option<Vec<i32>> {
    use windows::Win32::System::{
        Com::SAFEARRAY,
        Ole::{
            SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetElement, SafeArrayGetElemsize,
            SafeArrayGetLBound, SafeArrayGetUBound,
        },
    };
    struct OwnedArray(*mut SAFEARRAY);
    impl Drop for OwnedArray {
        fn drop(&mut self) {
            let _ = unsafe { SafeArrayDestroy(self.0) };
        }
    }
    let array = OwnedArray(unsafe { element.GetRuntimeId() }.ok()?);
    if array.0.is_null()
        || unsafe { SafeArrayGetDim(array.0) } != 1
        || unsafe { SafeArrayGetElemsize(array.0) } != std::mem::size_of::<i32>() as u32
    {
        return None;
    }
    let lower = unsafe { SafeArrayGetLBound(array.0, 1) }.ok()?;
    let upper = unsafe { SafeArrayGetUBound(array.0, 1) }.ok()?;
    let count = upper.checked_sub(lower)?.checked_add(1)? as usize;
    if !(1..=MAX_RUNTIME_ID_PARTS).contains(&count) {
        return None;
    }
    let mut result = Vec::with_capacity(count);
    for index in lower..=upper {
        let mut value = 0i32;
        unsafe {
            SafeArrayGetElement(array.0, &index, (&mut value as *mut i32).cast()).ok()?;
        }
        result.push(value);
    }
    Some(result)
}

pub(crate) fn execute_action(
    plan: &ActionPlan,
    permit: &nickel_remote_control::DesktopPermit,
    deadline: Instant,
    authorize: impl FnOnce() -> Result<ActionAuthorization, String>,
    dispatch: &AtomicU8,
) -> Result<NativeSemanticActionOutcome, String> {
    use windows::Win32::{
        Foundation::HWND,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationInvokePattern},
    };
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
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
    let root = unsafe { automation.ElementFromHandle(HWND(plan.target.root.native as *mut _)) }
        .map_err(|_| "Windows UI Automation window is unavailable".to_owned())?;
    if unsafe { root.CurrentProcessId() }
        .ok()
        .and_then(|pid| u32::try_from(pid).ok())
        != Some(plan.target.root.pid)
    {
        return Err("Windows UI Automation process identity changed".into());
    }
    let mut visited = 0usize;
    let mut matches = Vec::<IUIAutomationInvokePattern>::new();
    find_action(
        &root,
        0,
        &walker,
        &plan.target,
        permit,
        deadline,
        &mut visited,
        &mut matches,
    )?;
    if matches.len() != 1 {
        return Err("Windows UI Automation action identity changed".into());
    }
    let authorization = match authorize() {
        Ok(authorization) => authorization,
        Err(_) if dispatch.load(Ordering::Acquire) == ACTION_ATTEMPTED => {
            ActionAuthorization { input: None }
        }
        Err(error) => return Err(error),
    };
    let _input = authorization.input;
    let result = unsafe { matches[0].Invoke() };
    Ok(NativeSemanticActionOutcome {
        requested: true,
        confirmed: false,
        uncertain: result.is_err(),
    })
}

#[allow(clippy::too_many_arguments)]
fn find_action(
    element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    depth: usize,
    walker: &windows::Win32::UI::Accessibility::IUIAutomationTreeWalker,
    target: &ActionProof,
    permit: &nickel_remote_control::DesktopPermit,
    deadline: Instant,
    visited: &mut usize,
    matches: &mut Vec<windows::Win32::UI::Accessibility::IUIAutomationInvokePattern>,
) -> Result<(), String> {
    use windows::Win32::UI::Accessibility::{IUIAutomationInvokePattern, UIA_InvokePatternId};
    live(permit, deadline)?;
    if *visited == MAX_NODES || depth == MAX_DEPTH {
        return Ok(());
    }
    *visited += 1;
    if unsafe { element.CurrentProcessId() }
        .ok()
        .and_then(|pid| u32::try_from(pid).ok())
        != Some(target.root.pid)
    {
        return Ok(());
    }
    if unsafe { element.CurrentIsEnabled() }.is_ok_and(|value| value.as_bool())
        && runtime_id(element).as_deref() == Some(target.runtime_id.as_slice())
        && target.action == NativeSemanticAction::Invoke
        && let Ok(pattern) = unsafe {
            element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
        }
    {
        matches.push(pattern);
        if matches.len() > 1 {
            return Err("Windows UI Automation action identity is ambiguous".into());
        }
    }
    let mut child = unsafe { walker.GetFirstChildElement(element) }.ok();
    while let Some(current) = child {
        find_action(
            &current,
            depth + 1,
            walker,
            target,
            permit,
            deadline,
            visited,
            matches,
        )?;
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

    #[test]
    fn advertised_action_requires_exact_snapshot_and_node_identity() {
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
        let observation = ActionObservation {
            observation_generation: 23,
            proof: Proof {
                id: "7".into(),
                generation: 9,
                native: 11,
                pid: 13,
                created: 15,
                thread: 17,
                bounds: root.bounds,
                application: None,
                roots: vec![root.clone()],
                application_wide: false,
            },
            actions: vec![ActionProof {
                node: 3,
                root,
                runtime_id: vec![42, 11, 3],
                action: NativeSemanticAction::Invoke,
            }],
        };
        let request = |observation_generation, node, scope| {
            nickel_remote_control::native_semantics::NativeSemanticActionRequest {
                lease_id: 1,
                scope,
                window_id: "7".into(),
                window_generation: 9,
                observation_generation,
                node,
                action: NativeSemanticAction::Invoke,
            }
        };
        let plan = observation
            .plan(&request(23, 3, NativeSemanticScope::Window))
            .unwrap();
        assert_eq!(plan.target.runtime_id, [42, 11, 3]);
        assert!(
            observation
                .plan(&request(22, 3, NativeSemanticScope::Window))
                .is_none()
        );
        assert!(
            observation
                .plan(&request(23, 2, NativeSemanticScope::Window))
                .is_none()
        );
        assert!(
            observation
                .plan(&request(23, 3, NativeSemanticScope::ApplicationConnection))
                .is_none()
        );
    }

    #[test]
    fn cancellation_before_locked_commit_cannot_invoke_later() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(1);
        let cancelled = AtomicU8::new(ACTION_PENDING);
        assert!(cancel_action_dispatch(&cancelled));
        let mut invoked = false;
        if commit_action_dispatch(&cancelled, 7, 7, now, deadline).is_ok() {
            invoked = true;
        }
        assert!(!invoked);
        assert_eq!(cancelled.load(Ordering::Acquire), ACTION_CANCELLED);

        let attempted = AtomicU8::new(ACTION_PENDING);
        assert!(commit_action_dispatch(&attempted, 7, 7, now, deadline).is_ok());
        assert!(!cancel_action_dispatch(&attempted));
        assert_eq!(attempted.load(Ordering::Acquire), ACTION_ATTEMPTED);
    }

    #[test]
    fn locked_commit_rejects_physical_input_and_expiry_without_transition() {
        let now = Instant::now();
        let physical_input = AtomicU8::new(ACTION_PENDING);
        assert!(
            commit_action_dispatch(&physical_input, 7, 8, now, now + Duration::from_secs(1))
                .is_err()
        );
        assert_eq!(physical_input.load(Ordering::Acquire), ACTION_PENDING);

        let expired = AtomicU8::new(ACTION_PENDING);
        assert!(commit_action_dispatch(&expired, 7, 7, now, now).is_err());
        assert_eq!(expired.load(Ordering::Acquire), ACTION_PENDING);
    }
}
