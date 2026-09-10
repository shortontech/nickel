//! Owner generations and scope projection for freshly observed native resources.
//! Native HWND validity is rechecked by the adapter; these are owner identities,
//! not a claim that a raw HWND can be retained against native destruction/reuse.
use nickel_remote_control::{
    WindowSummary,
    diagnostics::{MAX_DIAGNOSTIC_OUTPUTS, MAX_DIAGNOSTIC_WINDOWS, OutputDiagnostic},
    leases::{ResourceEvidence, ResourceId, ResourceScope, ResourceScopeAuthority},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
    pub(crate) fn valid(self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= i32::MAX as u32
            && self.height <= i32::MAX as u32
    }
    fn contains(self, other: Self) -> bool {
        self.valid()
            && other.valid()
            && other.x >= self.x
            && other.y >= self.y
            && i64::from(other.x) + i64::from(other.width)
                <= i64::from(self.x) + i64::from(self.width)
            && i64::from(other.y) + i64::from(other.height)
                <= i64::from(self.y) + i64::from(self.height)
    }
    fn array(self) -> [i32; 4] {
        [self.x, self.y, self.width as i32, self.height as i32]
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Output {
    pub native: usize,
    pub name: String,
    pub bounds: Rect,
    pub work_area: Rect,
    pub scale_120: u32,
    pub primary: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Window {
    pub native: usize,
    pub pid: u32,
    pub created: u64,
    pub thread: u32,
    pub title: String,
    pub label: String,
    pub bounds: Rect,
    pub active: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub protected: bool,
    pub application: Option<String>,
}
struct Record<T> {
    identity: ResourceId,
    value: T,
}
#[derive(Default)]
pub(crate) struct Owner {
    generation: u64,
    windows: BTreeMap<usize, Record<Window>>,
    outputs: BTreeMap<String, Record<Output>>,
}
impl Owner {
    fn identity(&mut self, id: Option<&str>) -> Result<ResourceId, String> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("Windows resource generations exhausted")?;
        Ok(ResourceId {
            id: id
                .map(str::to_owned)
                .unwrap_or_else(|| self.generation.to_string()),
            generation: self.generation,
        })
    }
    pub(crate) fn retire_window(&mut self, native: usize, mut revoke: impl FnMut(&ResourceId)) {
        if let Some(window) = self.windows.get(&native) {
            revoke(&window.identity);
        }
        self.windows.remove(&native);
    }
    pub(crate) fn clear(&mut self, mut revoke: impl FnMut(&ResourceId)) {
        for record in self.windows.values() {
            revoke(&record.identity);
        }
        for record in self.outputs.values() {
            revoke(&record.identity);
        }
        self.windows.clear();
        self.outputs.clear();
    }
    pub(crate) fn reconcile(
        &mut self,
        windows: Vec<Window>,
        outputs: Vec<Output>,
        mut revoke: impl FnMut(&ResourceId),
    ) -> Result<(), String> {
        if windows.len() > MAX_DIAGNOSTIC_WINDOWS || outputs.len() > MAX_DIAGNOSTIC_OUTPUTS {
            self.clear(revoke);
            return Err("Windows inventory exceeds its bound".into());
        }
        let mut next_outputs = BTreeMap::new();
        for output in outputs {
            if output.native == 0
                || output.name.is_empty()
                || output.name.len() > 512
                || !output.bounds.valid()
                || !output.work_area.valid()
                || output.scale_120 == 0
            {
                self.clear(revoke);
                return Err("Invalid output evidence".into());
            }
            if next_outputs.insert(output.name.clone(), output).is_some() {
                self.clear(revoke);
                return Err("Ambiguous output evidence".into());
            }
        }
        let retired: Vec<_> = self
            .outputs
            .iter()
            .filter_map(|(name, record)| {
                next_outputs
                    .get(name)
                    .is_none_or(|next| next.native != record.value.native)
                    .then_some(name.clone())
            })
            .collect();
        for name in retired {
            revoke(&self.outputs[&name].identity);
            self.outputs.remove(&name);
        }
        for (name, output) in next_outputs {
            if let Some(record) = self.outputs.get_mut(&name) {
                record.value = output;
            } else {
                let identity = self.identity(Some(&name))?;
                self.outputs.insert(
                    name,
                    Record {
                        identity,
                        value: output,
                    },
                );
            }
        }
        let mut next_windows = BTreeMap::new();
        for window in windows {
            if window.protected
                || window.native == 0
                || window.pid == 0
                || window.created == 0
                || window.thread == 0
                || !window.bounds.valid()
            {
                continue;
            }
            if next_windows.insert(window.native, window).is_some() {
                self.clear(revoke);
                return Err("Ambiguous window evidence".into());
            }
        }
        let retired: Vec<_> = self
            .windows
            .iter()
            .filter_map(|(native, record)| {
                next_windows
                    .get(native)
                    .is_none_or(|next| {
                        (next.pid, next.created, next.thread)
                            != (record.value.pid, record.value.created, record.value.thread)
                    })
                    .then_some(*native)
            })
            .collect();
        for native in retired {
            self.retire_window(native, &mut revoke);
        }
        for (native, window) in next_windows {
            if let Some(record) = self.windows.get_mut(&native) {
                record.value = window;
            } else {
                let identity = self.identity(None)?;
                self.windows.insert(
                    native,
                    Record {
                        identity,
                        value: window,
                    },
                );
            }
        }
        Ok(())
    }
    fn output_for(&self, bounds: Rect) -> Option<&ResourceId> {
        let mut matches = self
            .outputs
            .values()
            .filter(|output| output.value.bounds.contains(bounds));
        let identity = &matches.next()?.identity;
        matches.next().is_none().then_some(identity)
    }
    pub(crate) fn window(&self, id: &str, generation: u64) -> Option<&Window> {
        self.windows
            .values()
            .find(|record| record.identity.id == id && record.identity.generation == generation)
            .map(|record| &record.value)
    }
    pub(crate) fn windows<'a>(
        &'a self,
        scope: &'a ResourceScope,
    ) -> impl Iterator<Item = (WindowSummary, ResourceEvidence<'a>)> + 'a {
        self.windows.values().filter_map(move |record| {
            let window = &record.value;
            let evidence = ResourceEvidence {
                surface: None,
                window: Some(&record.identity),
                verified_application: window.application.as_deref(),
                output: self.output_for(window.bounds),
                authorized_surface_ancestors: &[],
                protected: window.protected,
            };
            if !scope.covers(&evidence) {
                return None;
            }
            Some((
                WindowSummary {
                    id: record.identity.id.clone(),
                    generation: record.identity.generation,
                    application_id: window.label.clone(),
                    title: window.title.clone(),
                    active: window.active,
                    minimized: window.minimized,
                    maximized: window.maximized,
                    fullscreen: window.fullscreen,
                    x: window.bounds.x,
                    y: window.bounds.y,
                    width: window.bounds.width,
                    height: window.bounds.height,
                    workspace: 0,
                    verified_application: window.application.clone(),
                },
                evidence,
            ))
        })
    }
    pub(crate) fn outputs<'a>(
        &'a self,
        scope: &'a ResourceScope,
    ) -> impl Iterator<Item = (OutputDiagnostic, ResourceEvidence<'a>)> + 'a {
        self.outputs.values().filter_map(move |record| {
            let output = &record.value;
            let evidence = ResourceEvidence {
                surface: None,
                window: None,
                verified_application: None,
                output: Some(&record.identity),
                authorized_surface_ancestors: &[],
                protected: false,
            };
            if !scope.covers(&evidence) {
                return None;
            }
            Some((
                OutputDiagnostic {
                    name: output.name.clone(),
                    generation: record.identity.generation,
                    geometry: output.bounds.array(),
                    work_area: output.work_area.array(),
                    scale_120: output.scale_120,
                    primary: output.primary,
                    enabled: true,
                },
                evidence,
            ))
        })
    }
}

pub(crate) fn protected_executable(name: Option<&str>) -> bool {
    let Some(name) = name else {
        return true;
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "nickel.exe"
            | "nickel-settings.exe"
            | "nickel-login.exe"
            | "nickel-screenshot.exe"
            | "consent.exe"
            | "credentialuibroker.exe"
            | "logonui.exe"
            | "lockapp.exe"
            | "winlogon.exe"
            | "systemsettings.exe"
            | "sechealthui.exe"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output(native: usize, name: &str, x: i32, width: u32) -> Output {
        Output {
            native,
            name: name.into(),
            bounds: Rect {
                x,
                y: 0,
                width,
                height: 1000,
            },
            work_area: Rect {
                x,
                y: 0,
                width,
                height: 960,
            },
            scale_120: 120,
            primary: x == 0,
        }
    }
    fn window(native: usize, x: i32) -> Window {
        Window {
            native,
            pid: 10,
            created: 20,
            thread: 30,
            title: "App".into(),
            label: "Unverified label".into(),
            bounds: Rect {
                x,
                y: 10,
                width: 100,
                height: 100,
            },
            active: true,
            minimized: false,
            maximized: false,
            fullscreen: false,
            protected: false,
            application: None,
        }
    }
    #[test]
    fn negative_monitor_geometry_and_whole_window_membership_are_exact() {
        let mut owner = Owner::default();
        owner
            .reconcile(
                vec![window(1, -500), window(2, -50), window(3, 50)],
                vec![output(1, "left", -1000, 1000), output(2, "right", 0, 1500)],
                |_| {},
            )
            .unwrap();
        let full = ResourceScope::FullSession;
        let outputs: Vec<_> = owner.outputs(&full).collect();
        assert_eq!(outputs[0].0.geometry, [-1000, 0, 1000, 1000]);
        assert_eq!(outputs[0].0.work_area, [-1000, 0, 1000, 960]);
        let left = ResourceScope::Output(ResourceId {
            id: "left".into(),
            generation: outputs[0].0.generation,
        });
        let visible: Vec<_> = owner.windows(&left).collect();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].0.x, -500);
        assert_eq!(owner.windows(&full).count(), 3);
    }
    #[test]
    fn destroy_reuse_and_process_incarnation_changes_retire_before_new_identity() {
        let mut owner = Owner::default();
        owner
            .reconcile(
                vec![window(1, 10)],
                vec![output(1, "main", 0, 1000)],
                |_| {},
            )
            .unwrap();
        let old = owner.windows(&ResourceScope::FullSession).next().unwrap().0;
        let mut revoked = Vec::new();
        owner.retire_window(1, |id| revoked.push(id.clone()));
        owner
            .reconcile(
                vec![window(1, 10)],
                vec![output(1, "main", 0, 1000)],
                |_| {},
            )
            .unwrap();
        assert!(owner.window(&old.id, old.generation).is_none());
        assert_eq!(revoked[0].generation, old.generation);
        let mut changed = window(1, 10);
        changed.created += 1;
        owner
            .reconcile(vec![changed], vec![output(1, "main", 0, 1000)], |id| {
                revoked.push(id.clone())
            })
            .unwrap();
        assert_eq!(revoked.len(), 2);
    }
    #[test]
    fn protected_or_unverified_shared_runtime_never_gains_application_membership() {
        let mut protected = window(2, 10);
        protected.protected = true;
        let mut owner = Owner::default();
        owner
            .reconcile(
                vec![window(1, 10), protected],
                vec![output(1, "main", 0, 1000)],
                |_| {},
            )
            .unwrap();
        assert_eq!(owner.windows(&ResourceScope::FullSession).count(), 1);
        assert_eq!(
            owner
                .windows(&ResourceScope::Application("Unverified label".into()))
                .count(),
            0
        );
        assert!(protected_executable(None));
        assert!(protected_executable(Some("CONSENT.EXE")));
        assert!(protected_executable(Some("nickel-settings.exe")));
        assert!(!protected_executable(Some("ordinary.exe")));
    }
    #[test]
    fn output_replacement_and_ambiguous_overlap_fail_closed() {
        let mut owner = Owner::default();
        owner
            .reconcile(
                vec![window(1, 10)],
                vec![output(1, "main", 0, 1000)],
                |_| {},
            )
            .unwrap();
        let old = owner.outputs(&ResourceScope::FullSession).next().unwrap().0;
        let mut revoked = Vec::new();
        owner
            .reconcile(
                vec![window(1, 10)],
                vec![output(2, "main", 0, 1000), output(3, "clone", 0, 1000)],
                |id| revoked.push(id.clone()),
            )
            .unwrap();
        assert_eq!(revoked[0].generation, old.generation);
        let current = owner
            .outputs(&ResourceScope::FullSession)
            .find(|(o, _)| o.name == "main")
            .unwrap()
            .0;
        assert_eq!(
            owner
                .windows(&ResourceScope::Output(ResourceId {
                    id: "main".into(),
                    generation: current.generation
                }))
                .count(),
            0
        );
    }
}
