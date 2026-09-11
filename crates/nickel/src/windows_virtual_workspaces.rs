//! Bounded projection of the public Windows virtual-desktop authority.
//!
//! The supported `IVirtualDesktopManager` contract identifies a window's
//! desktop and moves a window to a known desktop. Windows does not publish a
//! supported API for creating, switching, removing, or enumerating empty
//! desktops, so this module never emulates those operations with global keys.

use nickel_remote_control::diagnostics::{MAX_DIAGNOSTIC_WORKSPACES, WorkspaceDiagnostic};
use std::collections::{BTreeMap, BTreeSet};

const MAX_RETAINED_DESKTOPS: usize = MAX_DIAGNOSTIC_WORKSPACES;

#[derive(Clone, Debug, Default)]
pub(crate) struct Observation {
    pub(crate) desktops: Vec<u128>,
    pub(crate) current: Option<u128>,
    pub(crate) window_desktops: BTreeMap<usize, u128>,
    pub(crate) truncated: bool,
}

#[derive(Default)]
pub(crate) struct Owner {
    next_id: u64,
    identities: BTreeMap<u128, u64>,
    current: Option<u128>,
    window_desktops: BTreeMap<usize, u128>,
    truncated: bool,
}

impl Owner {
    pub(crate) fn reconcile(&mut self, observation: Observation) -> Result<(), String> {
        let mut desktops = observation.desktops.into_iter().collect::<BTreeSet<_>>();
        desktops.extend(observation.window_desktops.values().copied());
        if let Some(current) = observation.current {
            desktops.insert(current);
        }
        if desktops.len() > MAX_RETAINED_DESKTOPS {
            return Err("Windows virtual desktop inventory exceeds its bound".into());
        }
        self.identities
            .retain(|desktop, _| desktops.contains(desktop));
        for desktop in desktops {
            if !self.identities.contains_key(&desktop) {
                self.next_id = self
                    .next_id
                    .checked_add(1)
                    .ok_or("Windows workspace identities exhausted")?;
                self.identities.insert(desktop, self.next_id);
            }
        }
        self.current = observation.current;
        self.window_desktops = observation.window_desktops;
        self.truncated = observation.truncated;
        Ok(())
    }

    pub(crate) fn diagnostics(
        &self,
        windows: &[(usize, String)],
    ) -> (Vec<WorkspaceDiagnostic>, bool) {
        let projected = windows.iter().cloned().collect::<BTreeMap<_, _>>();
        let mut workspaces = self
            .identities
            .iter()
            .map(|(native_desktop, id)| {
                let members = self
                    .window_desktops
                    .iter()
                    .filter_map(|(native, desktop)| {
                        (desktop == native_desktop)
                            .then(|| projected.get(native).cloned())
                            .flatten()
                    })
                    .collect::<Vec<_>>();
                let last_focused_window = (self.current == Some(*native_desktop))
                    .then(|| members.first().cloned())
                    .flatten();
                WorkspaceDiagnostic {
                    id: *id,
                    active: self.current == Some(*native_desktop),
                    windows: members,
                    last_focused_window,
                }
            })
            .collect::<Vec<_>>();
        workspaces.sort_by_key(|workspace| workspace.id);
        (workspaces, self.truncated)
    }

    pub(crate) fn native_id(&self, workspace: u64) -> Option<u128> {
        self.identities
            .iter()
            .find_map(|(native, id)| (*id == workspace).then_some(*native))
    }

    pub(crate) fn workspace_for_window(&self, native: usize) -> Option<u64> {
        let desktop = self.window_desktops.get(&native)?;
        self.identities.get(desktop).copied()
    }
}

#[cfg(target_os = "windows")]
pub(crate) mod native {
    use super::Observation;
    use std::{collections::BTreeMap, ffi::c_void, mem::size_of};
    use windows::{
        Win32::{
            Foundation::{ERROR_SUCCESS, HWND},
            System::{
                Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
                Registry::{HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RegGetValueW},
            },
            UI::Shell::{IVirtualDesktopManager, VirtualDesktopManager},
        },
        core::{GUID, PCWSTR},
    };

    const REGISTRY_PATH: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops";
    const MAX_REGISTRY_BYTES: usize = super::MAX_RETAINED_DESKTOPS * size_of::<GUID>();

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    fn registry_binary(value: &str) -> Result<Vec<u8>, String> {
        let path = wide(REGISTRY_PATH);
        let value = wide(value);
        let mut bytes = [0_u8; MAX_REGISTRY_BYTES];
        let mut length = bytes.len() as u32;
        // SAFETY: both strings are terminated and the fixed output buffer is
        // writable for `length`; only REG_BINARY values are accepted.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                PCWSTR(value.as_ptr()),
                RRF_RT_REG_BINARY,
                None,
                Some(bytes.as_mut_ptr().cast::<c_void>()),
                Some(&mut length),
            )
        };
        if status != ERROR_SUCCESS {
            return Err("Windows virtual desktop inventory is unavailable".into());
        }
        let length = usize::try_from(length).map_err(|_| "Invalid Windows desktop inventory")?;
        if length == 0 || length > bytes.len() || length % size_of::<GUID>() != 0 {
            return Err("Invalid Windows virtual desktop inventory".into());
        }
        Ok(bytes[..length].to_vec())
    }

    fn parse_guids(bytes: &[u8]) -> Result<Vec<u128>, String> {
        if bytes.is_empty() || !bytes.len().is_multiple_of(16) {
            return Err("Invalid Windows virtual desktop inventory".into());
        }
        Ok(bytes
            .chunks_exact(16)
            .map(|bytes| {
                // Registry desktop IDs use the in-memory GUID layout, whose
                // first fields are little-endian rather than RFC byte order.
                let guid = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<GUID>()) };
                guid.to_u128()
            })
            .collect())
    }

    fn manager() -> Result<IVirtualDesktopManager, String> {
        // The winit owner initializes COM. CoCreateInstance uses that existing
        // apartment and returns only the supported public desktop interface.
        unsafe { CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| "Windows virtual desktop manager is unavailable".into())
    }

    pub(crate) fn observe(windows: &[usize]) -> Result<Observation, String> {
        let mut desktops = parse_guids(&registry_binary("VirtualDesktopIDs")?)?;
        let truncated = desktops.len() > super::MAX_RETAINED_DESKTOPS;
        desktops.truncate(super::MAX_RETAINED_DESKTOPS);
        let manager = manager()?;
        let mut window_desktops = BTreeMap::new();
        let mut current = registry_binary("CurrentVirtualDesktop")
            .ok()
            .and_then(|bytes| parse_guids(&bytes).ok())
            .and_then(|ids| ids.into_iter().next())
            .filter(|id| desktops.contains(id));
        for native in windows.iter().copied() {
            let hwnd = HWND(native as *mut c_void);
            // SAFETY: handles came from a fresh bounded EnumWindows inventory;
            // these calls only query the supported desktop manager.
            if let Ok(id) = unsafe { manager.GetWindowDesktopId(hwnd) } {
                let id = id.to_u128();
                if desktops.contains(&id) {
                    window_desktops.insert(native, id);
                    if current.is_none()
                        && unsafe { manager.IsWindowOnCurrentVirtualDesktop(hwnd) }
                            .is_ok_and(|on_current| on_current.as_bool())
                    {
                        current = Some(id);
                    }
                }
            }
        }
        if current.is_none() {
            return Err("Windows current virtual desktop is unavailable".into());
        }
        Ok(Observation {
            desktops,
            current,
            window_desktops,
            truncated,
        })
    }

    pub(crate) fn move_window(native: usize, desktop: u128) -> Result<(), String> {
        let manager = manager()?;
        let desktop = GUID::from_u128(desktop);
        // SAFETY: the owner freshly validates the HWND and desktop identity at
        // the authorization boundary. The public manager performs the move.
        unsafe { manager.MoveWindowToDesktop(HWND(native as *mut c_void), &desktop) }
            .map_err(|_| "Windows rejected the virtual desktop move".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_uses_opaque_non_reused_workspace_ids_and_bounded_membership() {
        let mut owner = Owner::default();
        owner
            .reconcile(Observation {
                desktops: vec![11, 22],
                current: Some(11),
                window_desktops: BTreeMap::from([(7, 11), (8, 22)]),
                truncated: false,
            })
            .unwrap();
        let (first, truncated) =
            owner.diagnostics(&[(7, "ordinary-a".into()), (8, "ordinary-b".into())]);
        assert!(!truncated);
        assert_eq!(first.len(), 2);
        assert!(first[0].active);
        assert_eq!(first[0].windows, ["ordinary-a"]);
        assert_eq!(owner.workspace_for_window(8), Some(first[1].id));

        let retired = first[0].id;
        owner
            .reconcile(Observation {
                desktops: vec![22, 33],
                current: Some(33),
                window_desktops: BTreeMap::new(),
                truncated: false,
            })
            .unwrap();
        let (second, _) = owner.diagnostics(&[]);
        assert!(!second.iter().any(|workspace| workspace.id == retired));
        assert!(second.iter().map(|workspace| workspace.id).max().unwrap() > retired);
    }

    #[test]
    fn oversized_native_inventory_fails_closed() {
        let mut owner = Owner::default();
        let error = owner
            .reconcile(Observation {
                desktops: (0..=MAX_RETAINED_DESKTOPS as u128).collect(),
                ..Observation::default()
            })
            .unwrap_err();
        assert!(error.contains("exceeds"));
    }
}
