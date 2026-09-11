//! Owner generations and scope projection for freshly observed native resources.
//! Native HWND validity is rechecked by the adapter; these are owner identities,
//! not a claim that a raw HWND can be retained against native destruction/reuse.
use nickel_remote_control::{
    WindowSummary,
    diagnostics::{MAX_DIAGNOSTIC_OUTPUTS, MAX_DIAGNOSTIC_WINDOWS, OutputDiagnostic},
    leases::{ResourceEvidence, ResourceId, ResourceScope, ResourceScopeAuthority},
    pointer::PointerTarget,
    window_actions::WindowAction,
};
use std::collections::BTreeMap;

pub(crate) fn capture_dimensions(width: i32, height: i32) -> Result<(u16, u16, usize), String> {
    let width = u16::try_from(width).map_err(|_| "Windows capture dimensions exceed limits")?;
    let height = u16::try_from(height).map_err(|_| "Windows capture dimensions exceed limits")?;
    if width > 8192 || height > 8192 {
        return Err("Windows capture dimensions exceed limits".into());
    }
    let pixels = usize::from(width)
        .checked_mul(usize::from(height))
        .filter(|pixels| *pixels > 0 && *pixels <= 16_777_216)
        .ok_or("Windows capture dimensions exceed limits")?;
    Ok((width, height, pixels))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutputCaptureChromeEvidence {
    pub on_requested_output: bool,
    pub visible: bool,
    pub protected: bool,
    /// True only after the production window owner has verified the retained
    /// trusted HWND's native capture-exclusion affinity.
    pub natively_excluded: bool,
}

/// A composed desktop readback is safe only when every visible protected
/// surface on the requested output has a production-verified native exclusion.
/// Surfaces on other outputs cannot widen the requested output's pixels.
pub(crate) fn output_capture_chrome_is_safe(
    evidence: impl IntoIterator<Item = OutputCaptureChromeEvidence>,
) -> bool {
    evidence.into_iter().all(|surface| {
        !surface.on_requested_output
            || !surface.visible
            || !surface.protected
            || surface.natively_excluded
    })
}

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
    pub(crate) fn contains(self, other: Self) -> bool {
        self.valid()
            && other.valid()
            && other.x >= self.x
            && other.y >= self.y
            && i64::from(other.x) + i64::from(other.width)
                <= i64::from(self.x) + i64::from(self.width)
            && i64::from(other.y) + i64::from(other.height)
                <= i64::from(self.y) + i64::from(self.height)
    }
    pub(crate) fn intersects(self, other: Self) -> bool {
        self.valid()
            && other.valid()
            && i64::from(self.x) < i64::from(other.x) + i64::from(other.width)
            && i64::from(other.x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(self.y) < i64::from(other.y) + i64::from(other.height)
            && i64::from(other.y) < i64::from(self.y) + i64::from(self.height)
    }
    fn contains_point(self, x: i32, y: i32) -> bool {
        self.valid()
            && i64::from(x) >= i64::from(self.x)
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) >= i64::from(self.y)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
    fn array(self) -> [i32; 4] {
        [self.x, self.y, self.width as i32, self.height as i32]
    }
}

/// Convert one physical virtual-desktop axis to the normalized coordinate
/// accepted by an absolute `SendInput` mouse event. Kept in owner policy so
/// negative-origin and edge behavior remains portable-testable.
pub(crate) fn absolute_pointer_axis(value: i32, origin: i32, length: i32) -> Result<i32, String> {
    if length <= 1 || value < origin || i64::from(value) >= i64::from(origin) + i64::from(length) {
        return Err("Windows pointer coordinate is outside the virtual desktop".into());
    }
    Ok((((i64::from(value) - i64::from(origin)) * 65_535) / i64::from(length - 1)) as i32)
}

/// Validate a client-local point before native client-to-screen conversion.
/// The caller supplies `GetClientRect` dimensions, which deliberately exclude
/// title bars, borders, shadows, and other non-client decoration.
pub(crate) fn client_pointer_coordinate(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<(), String> {
    if width <= 0 || height <= 0 || x < 0 || y < 0 || x >= width || y >= height {
        return Err("Windows pointer coordinate is outside the client area".into());
    }
    Ok(())
}

pub(crate) enum PointerTargetResource<'a> {
    Window {
        native: usize,
        evidence: ResourceEvidence<'a>,
    },
    Global {
        x: i32,
        y: i32,
        confined_output: Option<&'a ResourceId>,
        evidence: ResourceEvidence<'a>,
    },
}

impl PointerTargetResource<'_> {
    pub(crate) fn evidence(&self) -> &ResourceEvidence<'_> {
        match self {
            Self::Window { evidence, .. } | Self::Global { evidence, .. } => evidence,
        }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WindowMutationInputState {
    pub keyboard_held: bool,
    pub pointer_held: bool,
    pub physical_input_idle: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WindowMutationPlan {
    output_topology_generation: u64,
    destination: Option<(ResourceId, Rect)>,
}
struct Record<T> {
    identity: ResourceId,
    value: T,
}
#[derive(Default)]
pub(crate) struct Owner {
    generation: u64,
    output_topology_generation: u64,
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
        let output_topology_changed = self.outputs.len() != next_outputs.len()
            || next_outputs.iter().any(|(name, output)| {
                self.outputs
                    .get(name)
                    .is_none_or(|current| current.value != *output)
            });
        if output_topology_changed {
            self.output_topology_generation = self
                .output_topology_generation
                .checked_add(1)
                .ok_or("Windows output topology generations exhausted")?;
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
    pub(crate) fn output_topology_generation(&self) -> u64 {
        self.output_topology_generation
    }
    fn output_for(&self, bounds: Rect) -> Option<&ResourceId> {
        let mut matches = self
            .outputs
            .values()
            .filter(|output| output.value.bounds.contains(bounds));
        let identity = &matches.next()?.identity;
        matches.next().is_none().then_some(identity)
    }
    /// Resolve one exact owner-generated output incarnation. Callers must still
    /// compare it with a fresh native topology before performing an effect.
    pub(crate) fn output_resource(&self, identity: &ResourceId) -> Option<&Output> {
        self.outputs
            .values()
            .find(|record| record.identity == *identity)
            .map(|record| &record.value)
    }

    /// Resolve one exact output incarnation through the requesting lease's
    /// scope projection. The returned evidence is checked by DesktopPermit at
    /// the owner boundary immediately before native readback.
    pub(crate) fn output_capture_resource<'a>(
        &'a self,
        scope: &'a ResourceScope,
        id: &str,
        generation: u64,
    ) -> Option<(&'a Output, ResourceEvidence<'a>)> {
        let record = self
            .outputs
            .values()
            .find(|record| record.identity.id == id && record.identity.generation == generation)?;
        let evidence = ResourceEvidence {
            surface: None,
            window: None,
            verified_application: None,
            output: Some(&record.identity),
            authorized_surface_ancestors: &[],
            protected: false,
        };
        scope.covers(&evidence).then_some((&record.value, evidence))
    }

    /// Validate the owner-side policy for one focus or window mutation before
    /// reserving shared input. Output-scoped moves and resizes must keep the
    /// entire requested rectangle on exactly one live output incarnation.
    pub(crate) fn prepare_window_mutation(
        &self,
        scope: &ResourceScope,
        action: WindowAction,
        input: WindowMutationInputState,
    ) -> Result<WindowMutationPlan, String> {
        action.validate()?;
        if input.keyboard_held || input.pointer_held || !input.physical_input_idle {
            return Err("local or remote input is already active".into());
        }
        let destination = match (scope, action) {
            (
                ResourceScope::Output(output),
                WindowAction::SetBounds {
                    x,
                    y,
                    width,
                    height,
                },
            ) => {
                let bounds = Rect {
                    x,
                    y,
                    width,
                    height,
                };
                if self.output_for(bounds) != Some(output) {
                    return Err("requested Windows bounds leave the authorized output".into());
                }
                Some((output.clone(), bounds))
            }
            _ => None,
        };
        Ok(WindowMutationPlan {
            output_topology_generation: self.output_topology_generation,
            destination,
        })
    }

    /// Recheck owner generations immediately before native dispatch. The
    /// request-local native preparation performs the corresponding live
    /// monitor revalidation at the same commit boundary.
    pub(crate) fn revalidate_window_mutation(
        &self,
        plan: &WindowMutationPlan,
    ) -> Result<(), String> {
        if self.output_topology_generation != plan.output_topology_generation {
            return Err("Windows output topology changed before window mutation".into());
        }
        if let Some((output, bounds)) = &plan.destination
            && self.output_for(*bounds) != Some(output)
        {
            return Err("requested Windows bounds leave the authorized output".into());
        }
        Ok(())
    }
    pub(crate) fn window(&self, id: &str, generation: u64) -> Option<&Window> {
        self.windows
            .values()
            .find(|record| record.identity.id == id && record.identity.generation == generation)
            .map(|record| &record.value)
    }
    pub(crate) fn scope_is_live(&self, scope: &ResourceScope) -> bool {
        match scope {
            ResourceScope::Window(identity) => self
                .windows
                .values()
                .any(|record| record.identity == *identity && !record.value.protected),
            ResourceScope::Output(identity) => self
                .outputs
                .values()
                .any(|record| record.identity == *identity),
            ResourceScope::Application(identity) => self.windows.values().any(|record| {
                !record.value.protected
                    && record.value.application.as_deref() == Some(identity.as_str())
            }),
            // Native external surfaces do not have a separately owned surface
            // incarnation on Windows yet. Never reinterpret one as a HWND.
            ResourceScope::Surface(_) => false,
            ResourceScope::FullSession => true,
        }
    }
    pub(crate) fn window_resource<'a>(
        &'a self,
        scope: &'a ResourceScope,
        id: &str,
        generation: u64,
    ) -> Option<(&'a Window, ResourceEvidence<'a>)> {
        let record = self
            .windows
            .values()
            .find(|record| record.identity.id == id && record.identity.generation == generation)?;
        let window = &record.value;
        let evidence = ResourceEvidence {
            surface: None,
            window: Some(&record.identity),
            verified_application: window.application.as_deref(),
            output: self.output_for(window.bounds),
            authorized_surface_ancestors: &[],
            protected: window.protected,
        };
        scope.covers(&evidence).then_some((window, evidence))
    }

    /// Resolve the caller's coordinate space through exact owner identities.
    /// Native hit testing remains a separate final check because stacking can
    /// change after this pure policy decision.
    pub(crate) fn pointer_target_resource<'a>(
        &'a self,
        scope: &'a ResourceScope,
        target: &PointerTarget,
        x: i32,
        y: i32,
    ) -> Option<PointerTargetResource<'a>> {
        match target {
            PointerTarget::Window {
                window_id,
                generation,
            } => {
                let (window, evidence) = self.window_resource(scope, window_id, *generation)?;
                Some(PointerTargetResource::Window {
                    native: window.native,
                    evidence,
                })
            }
            PointerTarget::Surface { .. } => None,
            PointerTarget::Output {
                output_id,
                generation,
            } => {
                let record = self.outputs.values().find(|record| {
                    record.identity.id == *output_id
                        && record.identity.generation == *generation
                        && record.value.bounds.contains_point(x, y)
                })?;
                let evidence = ResourceEvidence {
                    surface: None,
                    window: None,
                    verified_application: None,
                    output: Some(&record.identity),
                    authorized_surface_ancestors: &[],
                    protected: false,
                };
                scope
                    .covers(&evidence)
                    .then_some(PointerTargetResource::Global {
                        x,
                        y,
                        confined_output: Some(&record.identity),
                        evidence,
                    })
            }
            PointerTarget::Desktop => {
                if scope != &ResourceScope::FullSession {
                    return None;
                }
                let mut outputs = self
                    .outputs
                    .values()
                    .filter(|record| record.value.bounds.contains_point(x, y));
                let output = outputs.next()?;
                if outputs.next().is_some() {
                    return None;
                }
                Some(PointerTargetResource::Global {
                    x,
                    y,
                    confined_output: None,
                    evidence: ResourceEvidence {
                        surface: None,
                        window: None,
                        verified_application: None,
                        output: Some(&output.identity),
                        authorized_surface_ancestors: &[],
                        protected: false,
                    },
                })
            }
        }
    }

    pub(crate) fn pointer_target_identity_is_live(&self, target: &PointerTarget) -> bool {
        match target {
            PointerTarget::Window {
                window_id,
                generation,
            } => self.window(window_id, *generation).is_some(),
            PointerTarget::Surface { .. } => false,
            PointerTarget::Output {
                output_id,
                generation,
            } => self.outputs.values().any(|record| {
                record.identity.id == *output_id && record.identity.generation == *generation
            }),
            PointerTarget::Desktop => true,
        }
    }

    /// Accept a final native hit only when it is desktop background or an
    /// exact, ordinary, unprotected window covered by the live scope. Unknown
    /// HWNDs include Nickel trusted chrome and protected/excluded processes.
    pub(crate) fn pointer_hit_allowed(
        &self,
        scope: &ResourceScope,
        confined_output: Option<&ResourceId>,
        hit_window: Option<usize>,
    ) -> bool {
        let Some(native) = hit_window else {
            return true;
        };
        let Some(record) = self.windows.get(&native) else {
            return false;
        };
        let output = self.output_for(record.value.bounds);
        if confined_output.is_some() && output != confined_output {
            return false;
        }
        scope.covers(&ResourceEvidence {
            surface: None,
            window: Some(&record.identity),
            verified_application: record.value.application.as_deref(),
            output,
            authorized_surface_ancestors: &[],
            protected: record.value.protected,
        })
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
    /// Native identities for the same protected-filtered projection returned by
    /// `windows`. These are consumed only on the owner thread to filter retained
    /// preview accounting; they are never serialized or granted as authority.
    #[cfg(any(test, target_os = "windows"))]
    pub(crate) fn native_windows<'a>(
        &'a self,
        scope: &'a ResourceScope,
    ) -> impl Iterator<Item = usize> + 'a {
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
            scope.covers(&evidence).then_some(window.native)
        })
    }

    pub(crate) fn projected_native_windows<'a>(
        &'a self,
        scope: &'a ResourceScope,
    ) -> impl Iterator<Item = (usize, String)> + 'a {
        self.windows(scope).filter_map(|(summary, _)| {
            self.window(&summary.id, summary.generation)
                .map(|window| (window.native, summary.id))
        })
    }

    /// Map a freshly sampled native recipient through the same exact scoped,
    /// protected-filtered projection used by the containing snapshot.
    pub(crate) fn diagnostic_window_id(
        &self,
        scope: &ResourceScope,
        native: usize,
    ) -> Option<String> {
        self.windows(scope).find_map(|(summary, _)| {
            self.window(&summary.id, summary.generation)
                .is_some_and(|window| window.native == native)
                .then_some(summary.id)
        })
    }

    pub(crate) fn shell_surface_authorized(
        &self,
        scope: &ResourceScope,
        surface: &ResourceId,
        output_name: Option<&str>,
        authorized_surface_ancestors: &[ResourceId],
    ) -> bool {
        self.shell_surface_resource(scope, surface, output_name, authorized_surface_ancestors)
            .is_some()
    }

    fn shell_surface_resource<'a>(
        &'a self,
        scope: &ResourceScope,
        surface: &'a ResourceId,
        output_name: Option<&str>,
        authorized_surface_ancestors: &'a [ResourceId],
    ) -> Option<ResourceEvidence<'a>> {
        let output =
            output_name.and_then(|name| self.outputs.get(name).map(|record| &record.identity))?;
        let evidence = ResourceEvidence {
            surface: Some(surface),
            window: None,
            verified_application: None,
            output: Some(output),
            authorized_surface_ancestors,
            protected: false,
        };
        scope.covers(&evidence).then_some(evidence)
    }

    pub(crate) fn output_generation(&self, name: &str) -> Option<u64> {
        self.outputs
            .get(name)
            .map(|record| record.identity.generation)
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
        assert_eq!(owner.native_windows(&left).collect::<Vec<_>>(), vec![1]);
        assert_eq!(
            owner.native_windows(&full).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(owner.windows(&full).count(), 3);
        assert!(
            Rect {
                x: -10,
                y: 0,
                width: 20,
                height: 20,
            }
            .intersects(Rect {
                x: 0,
                y: 10,
                width: 20,
                height: 20,
            })
        );
        assert!(
            !Rect {
                x: -20,
                y: 0,
                width: 20,
                height: 20,
            }
            .intersects(Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 20,
            })
        );
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
    fn input_recipient_mapping_uses_exact_resource_scope() {
        let mut owner = Owner::default();
        owner
            .reconcile(
                vec![window(11, -500), window(22, 50)],
                vec![output(1, "left", -1000, 1000), output(2, "right", 0, 1500)],
                |_| {},
            )
            .unwrap();
        let outputs: Vec<_> = owner.outputs(&ResourceScope::FullSession).collect();
        let left = ResourceScope::Output(ResourceId {
            id: "left".into(),
            generation: outputs[0].0.generation,
        });
        assert!(owner.diagnostic_window_id(&left, 11).is_some());
        assert!(owner.diagnostic_window_id(&left, 22).is_none());
        assert!(
            owner
                .diagnostic_window_id(&ResourceScope::FullSession, 22)
                .is_some()
        );
        assert!(
            owner
                .diagnostic_window_id(&ResourceScope::FullSession, 99)
                .is_none()
        );
    }

    #[test]
    fn shell_input_scope_requires_exact_surface_or_output_incarnation() {
        let mut owner = Owner::default();
        owner
            .reconcile(Vec::new(), vec![output(1, "main", 0, 1000)], |_| {})
            .unwrap();
        let output = owner.outputs(&ResourceScope::FullSession).next().unwrap().0;
        let surface = ResourceId {
            id: "windows-shell:40".into(),
            generation: 40,
        };
        assert!(owner.shell_surface_authorized(
            &ResourceScope::FullSession,
            &surface,
            Some("main"),
            &[],
        ));
        assert!(owner.shell_surface_authorized(
            &ResourceScope::Surface(surface.clone()),
            &surface,
            Some("main"),
            &[],
        ));
        assert!(owner.shell_surface_authorized(
            &ResourceScope::Output(ResourceId {
                id: output.name,
                generation: output.generation,
            }),
            &surface,
            Some("main"),
            &[],
        ));
        assert!(!owner.shell_surface_authorized(
            &ResourceScope::Surface(ResourceId {
                id: surface.id.clone(),
                generation: surface.generation + 1,
            }),
            &surface,
            Some("main"),
            &[],
        ));
        assert!(!owner.shell_surface_authorized(
            &ResourceScope::Window(surface.clone()),
            &surface,
            Some("main"),
            &[],
        ));
        assert!(!owner.shell_surface_authorized(
            &ResourceScope::Output(ResourceId {
                id: "main".into(),
                generation: output.generation + 1,
            }),
            &surface,
            Some("main"),
            &[],
        ));

        let parent = ResourceId {
            id: "windows-shell:39".into(),
            generation: 39,
        };
        assert!(owner.shell_surface_authorized(
            &ResourceScope::Surface(parent.clone()),
            &surface,
            Some("main"),
            std::slice::from_ref(&parent),
        ));
        assert!(!owner.shell_surface_authorized(
            &ResourceScope::Surface(ResourceId {
                id: parent.id.clone(),
                generation: parent.generation + 1,
            }),
            &surface,
            Some("main"),
            std::slice::from_ref(&parent),
        ));
    }

    #[test]
    fn production_owner_lease_scope_matrix_survives_actions_without_reprompt_or_widening() {
        use nickel_remote_control::{ControlPlane, DesktopPermit, lease_requests};
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};

        #[derive(Clone, Copy)]
        enum Action {
            Observe,
            Input,
        }

        #[derive(Clone, Copy)]
        enum Target<'a> {
            Window(usize),
            Surface {
                identity: &'a ResourceId,
                output: &'a str,
                ancestors: &'a [ResourceId],
            },
        }

        #[derive(Clone, Copy)]
        enum ScopeKind {
            Surface,
            Window,
            Application,
            Output,
            FullSession,
        }

        fn identified_window(native: usize, x: i32, application: &str, active: bool) -> Window {
            let mut value = window(native, x);
            value.pid = 100 + native as u32;
            value.created = 200 + native as u64;
            value.thread = 300 + native as u32;
            value.application = Some(application.into());
            value.active = active;
            value
        }

        fn reconcile(
            owner: &mut Owner,
            editor_x: i32,
            terminal_focused: bool,
            include_new_windows: bool,
        ) {
            let mut windows = vec![
                identified_window(
                    1,
                    editor_x,
                    "windows:catalog-launch:editor",
                    !terminal_focused,
                ),
                identified_window(2, 300, "windows:catalog-launch:terminal", terminal_focused),
            ];
            if include_new_windows {
                windows.extend([
                    identified_window(3, 500, "windows:catalog-launch:editor", false),
                    identified_window(4, 1300, "windows:catalog-launch:viewer", false),
                ]);
            }
            owner
                .reconcile(
                    windows,
                    vec![output(1, "left", 0, 1000), output(2, "right", 1000, 1000)],
                    |_| {},
                )
                .unwrap();
        }

        fn dispatch(
            owner: &Owner,
            permit: &DesktopPermit,
            target: Target<'_>,
            action: Action,
            effects: &mut usize,
        ) -> Result<(), String> {
            let scope = permit.resource_scope()?;
            let evidence = match target {
                Target::Window(native) => {
                    let record = owner
                        .windows
                        .get(&native)
                        .ok_or("production owner has no such window")?;
                    owner
                        .window_resource(&scope, &record.identity.id, record.identity.generation)
                        .map(|(_, evidence)| evidence)
                        .ok_or("production owner rejected window scope")?
                }
                Target::Surface {
                    identity,
                    output,
                    ancestors,
                } => owner
                    .shell_surface_resource(&scope, identity, Some(output), ancestors)
                    .ok_or("production owner rejected shell surface scope")?,
            };
            match action {
                Action::Observe => permit.with_resource(&evidence, || {
                    *effects += 1;
                    Ok(())
                }),
                Action::Input => permit.with_input(&evidence, || {
                    *effects += 1;
                    Ok(())
                }),
            }
        }

        let cases = [
            ("surface", ScopeKind::Surface),
            ("window", ScopeKind::Window),
            ("application", ScopeKind::Application),
            ("output", ScopeKind::Output),
            ("full session", ScopeKind::FullSession),
        ];

        for (case_index, (case_name, kind)) in cases.into_iter().enumerate() {
            let mut owner = Owner::default();
            reconcile(&mut owner, 100, false, false);
            let leased_surface = ResourceId {
                id: "windows-shell:editor".into(),
                generation: 41,
            };
            let child_surface = ResourceId {
                id: "windows-shell:editor-dialog".into(),
                generation: 42,
            };
            let unrelated_surface = ResourceId {
                id: "windows-shell:unrelated-dialog".into(),
                generation: 43,
            };
            let leased_window = owner.windows[&1].identity.clone();
            let leased_output = owner.outputs["left"].identity.clone();
            let scope = match kind {
                ScopeKind::Surface => ResourceScope::Surface(leased_surface.clone()),
                ScopeKind::Window => ResourceScope::Window(leased_window),
                ScopeKind::Application => {
                    ResourceScope::Application("windows:catalog-launch:editor".into())
                }
                ScopeKind::Output => ResourceScope::Output(leased_output),
                ScopeKind::FullSession => ResourceScope::FullSession,
            };

            let now = Instant::now();
            let control = Arc::new(Mutex::new(ControlPlane::default()));
            let (identity, lease) = {
                let mut control = control.lock().unwrap();
                control.set_enabled(true);
                let identity = control.connect_identity(case_name).unwrap();
                let watch = control
                    .reserve_connection_watch(&identity.client_id, &identity.token, now)
                    .unwrap();
                control
                    .activate_connection_watch(
                        &identity.client_id,
                        &identity.token,
                        watch,
                        false,
                        now,
                    )
                    .unwrap();
                let request = lease_requests::LeaseRequest {
                    renewal: None,
                    scope: scope.clone(),
                    duration: Some(Duration::from_secs(1200)),
                    allow_resumption: false,
                    full_debug: false,
                };
                assert!(
                    control
                        .request_lease(&identity.client_id, &identity.token, request.clone(), now,)
                        .unwrap(),
                    "{case_name} did not create its one permission request"
                );
                let generation = control
                    .lease_requests()
                    .pending_generation(&identity.client_id)
                    .unwrap();
                let lease = control
                    .approve_lease_local(&identity.client_id, &request, generation, now)
                    .unwrap();
                (identity, lease)
            };
            let permission_events = vec![
                lease_requests::Outcome::Submitted,
                lease_requests::Outcome::Approved,
            ];
            assert_eq!(
                control
                    .lock()
                    .unwrap()
                    .lease_requests()
                    .audit()
                    .map(|event| event.outcome)
                    .collect::<Vec<_>>(),
                permission_events
            );
            let permit = || {
                DesktopPermit::from_active_lease(
                    control.clone(),
                    identity.client_id.clone(),
                    identity.token.clone(),
                    lease,
                )
                .unwrap()
            };
            let mut effects = 0;
            let primary = match kind {
                ScopeKind::Surface => Target::Surface {
                    identity: &leased_surface,
                    output: "left",
                    ancestors: &[],
                },
                _ => Target::Window(1),
            };

            macro_rules! check {
                ($scenario:expr, $target:expr, $action:expr, $expected:expr $(,)?) => {{
                    let expected: [bool; 5] = $expected;
                    let before = effects;
                    let result = dispatch(&owner, &permit(), $target, $action, &mut effects);
                    assert_eq!(
                        result.is_ok(),
                        expected[case_index],
                        "{} scope disagreed with production-owner scenario {:?}: {:?}",
                        case_name,
                        $scenario,
                        result
                    );
                    assert_eq!(
                        effects,
                        before + usize::from(expected[case_index]),
                        "{} scope dispatched a rejected effect for {}",
                        case_name,
                        $scenario
                    );
                    let control = control.lock().unwrap();
                    assert_eq!(
                        control.lease_requests().pending().count(),
                        0,
                        "{} scope reprompted after {}",
                        case_name,
                        $scenario
                    );
                    assert_eq!(
                        control
                            .lease_requests()
                            .audit()
                            .map(|event| event.outcome)
                            .collect::<Vec<_>>(),
                        permission_events,
                        "{} scope recorded another permission decision after {}",
                        case_name,
                        $scenario
                    );
                }};
            }

            check!("first observation", primary, Action::Observe, [true; 5]);
            check!("second input", primary, Action::Input, [true; 5]);

            reconcile(&mut owner, 100, true, false);
            check!(
                "original resource after focus changed",
                primary,
                Action::Observe,
                [true; 5],
            );
            check!(
                "newly focused different application",
                Target::Window(2),
                Action::Input,
                [false, false, false, true, true],
            );

            reconcile(&mut owner, 1100, true, false);
            let moved_primary = match kind {
                ScopeKind::Surface => Target::Surface {
                    identity: &leased_surface,
                    output: "right",
                    ancestors: &[],
                },
                _ => Target::Window(1),
            };
            check!(
                "leased resource moved to another output",
                moved_primary,
                Action::Input,
                [true, true, true, false, true],
            );

            reconcile(&mut owner, 1100, true, true);
            check!(
                "new window from the leased application on the leased output",
                Target::Window(3),
                Action::Observe,
                [false, false, true, true, true],
            );
            check!(
                "verified child transient on the leased output",
                Target::Surface {
                    identity: &child_surface,
                    output: "left",
                    ancestors: std::slice::from_ref(&leased_surface),
                },
                Action::Input,
                [true, false, false, true, true],
            );
            check!(
                "unrelated transient on the leased output",
                Target::Surface {
                    identity: &child_surface,
                    output: "left",
                    ancestors: std::slice::from_ref(&unrelated_surface),
                },
                Action::Observe,
                [false, false, false, true, true],
            );
            check!(
                "different application on the leased output",
                Target::Window(2),
                Action::Input,
                [false, false, false, true, true],
            );
            check!(
                "different application outside the leased output",
                Target::Window(4),
                Action::Observe,
                [false, false, false, false, true],
            );
            assert!(effects >= 3, "{case_name} did not sustain multiple actions");
            assert_eq!(control.lock().unwrap().leases().iter().count(), 1);
        }
    }

    #[test]
    fn approval_scope_requires_the_exact_live_owner_incarnation() {
        let mut app = window(1, 10);
        app.application = Some("windows:catalog-launch:trusted".into());
        let mut owner = Owner::default();
        owner
            .reconcile(vec![app], vec![output(1, "main", 0, 1000)], |_| {})
            .unwrap();
        let live_window = owner.windows(&ResourceScope::FullSession).next().unwrap().0;
        let live_output = owner.outputs(&ResourceScope::FullSession).next().unwrap().0;
        assert!(owner.scope_is_live(&ResourceScope::Window(ResourceId {
            id: live_window.id.clone(),
            generation: live_window.generation,
        })));
        assert!(!owner.scope_is_live(&ResourceScope::Window(ResourceId {
            id: live_window.id,
            generation: live_window.generation + 1,
        })));
        assert!(owner.scope_is_live(&ResourceScope::Output(ResourceId {
            id: live_output.name.clone(),
            generation: live_output.generation,
        })));
        assert!(!owner.scope_is_live(&ResourceScope::Output(ResourceId {
            id: live_output.name,
            generation: live_output.generation + 1,
        })));
        assert!(owner.scope_is_live(&ResourceScope::Application(
            "windows:catalog-launch:trusted".into(),
        )));
        assert!(!owner.scope_is_live(&ResourceScope::Application("Unverified label".into(),)));
        assert!(!owner.scope_is_live(&ResourceScope::Surface(ResourceId {
            id: "1".into(),
            generation: 1,
        })));
        assert!(owner.scope_is_live(&ResourceScope::FullSession));
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
    #[test]
    fn topology_generation_tracks_outputs_but_not_windows() {
        let mut owner = Owner::default();
        assert_eq!(owner.output_topology_generation(), 0);
        owner
            .reconcile(
                vec![window(1, 10)],
                vec![output(1, "main", 0, 1000)],
                |_| {},
            )
            .unwrap();
        assert_eq!(owner.output_topology_generation(), 1);

        let mut changed_window = window(1, 10);
        changed_window.bounds.x = 20;
        owner
            .reconcile(
                vec![changed_window],
                vec![output(1, "main", 0, 1000)],
                |_| {},
            )
            .unwrap();
        assert_eq!(owner.output_topology_generation(), 1);

        let mut changed_output = output(1, "main", 0, 1000);
        changed_output.scale_120 = 180;
        owner
            .reconcile(vec![], vec![changed_output.clone()], |_| {})
            .unwrap();
        assert_eq!(owner.output_topology_generation(), 2);
        owner
            .reconcile(vec![], vec![changed_output], |_| {})
            .unwrap();
        assert_eq!(owner.output_topology_generation(), 2);

        owner.reconcile(vec![], vec![], |_| {}).unwrap();
        assert_eq!(owner.output_topology_generation(), 3);
    }
    #[test]
    fn output_scoped_bounds_require_unique_whole_destination_membership() {
        let mut owner = Owner::default();
        owner
            .reconcile(
                vec![window(1, -500)],
                vec![output(1, "left", -1000, 1000), output(2, "right", 0, 1500)],
                |_| {},
            )
            .unwrap();
        let left = owner
            .outputs(&ResourceScope::FullSession)
            .find(|(output, _)| output.name == "left")
            .unwrap()
            .0;
        let scope = ResourceScope::Output(ResourceId {
            id: left.name,
            generation: left.generation,
        });
        let idle = WindowMutationInputState {
            keyboard_held: false,
            pointer_held: false,
            physical_input_idle: true,
        };
        let contained = WindowAction::SetBounds {
            x: -1000,
            y: 0,
            width: 1000,
            height: 1000,
        };
        let plan = owner
            .prepare_window_mutation(&scope, contained, idle)
            .unwrap();
        owner.revalidate_window_mutation(&plan).unwrap();

        for action in [
            WindowAction::SetBounds {
                x: -1001,
                y: 0,
                width: 100,
                height: 100,
            },
            WindowAction::SetBounds {
                x: -50,
                y: 0,
                width: 100,
                height: 100,
            },
            WindowAction::SetBounds {
                x: 50,
                y: 0,
                width: 100,
                height: 100,
            },
        ] {
            assert!(owner.prepare_window_mutation(&scope, action, idle).is_err());
        }

        owner
            .reconcile(
                vec![window(1, -500)],
                vec![
                    output(1, "left", -1000, 1000),
                    output(2, "left-clone", -1000, 1000),
                    output(3, "right", 0, 1500),
                ],
                |_| {},
            )
            .unwrap();
        assert!(
            owner
                .prepare_window_mutation(&scope, contained, idle)
                .is_err()
        );
    }

    #[test]
    fn output_scoped_bounds_plan_rejects_a_changed_topology() {
        let mut owner = Owner::default();
        owner
            .reconcile(
                vec![window(1, -500)],
                vec![output(1, "left", -1000, 1000)],
                |_| {},
            )
            .unwrap();
        let live_output = owner.outputs(&ResourceScope::FullSession).next().unwrap().0;
        let scope = ResourceScope::Output(ResourceId {
            id: live_output.name,
            generation: live_output.generation,
        });
        let plan = owner
            .prepare_window_mutation(
                &scope,
                WindowAction::SetBounds {
                    x: -900,
                    y: 10,
                    width: 200,
                    height: 200,
                },
                WindowMutationInputState {
                    keyboard_held: false,
                    pointer_held: false,
                    physical_input_idle: true,
                },
            )
            .unwrap();
        let mut changed = output(1, "left", -1000, 1000);
        changed.scale_120 = 180;
        owner
            .reconcile(vec![window(1, -500)], vec![changed], |_| {})
            .unwrap();
        assert!(owner.revalidate_window_mutation(&plan).is_err());
    }

    #[test]
    fn non_output_scopes_keep_their_existing_bounds_authority() {
        let mut owned = window(1, -500);
        owned.application = Some("verified.app".into());
        let mut owner = Owner::default();
        owner
            .reconcile(
                vec![owned],
                vec![output(1, "left", -1000, 1000), output(2, "right", 0, 1500)],
                |_| {},
            )
            .unwrap();
        let summary = owner.windows(&ResourceScope::FullSession).next().unwrap().0;
        let action = WindowAction::SetBounds {
            x: 100,
            y: 10,
            width: 200,
            height: 200,
        };
        let idle = WindowMutationInputState {
            keyboard_held: false,
            pointer_held: false,
            physical_input_idle: true,
        };
        for scope in [
            ResourceScope::FullSession,
            ResourceScope::Window(ResourceId {
                id: summary.id,
                generation: summary.generation,
            }),
            ResourceScope::Application("verified.app".into()),
        ] {
            let plan = owner.prepare_window_mutation(&scope, action, idle).unwrap();
            owner.revalidate_window_mutation(&plan).unwrap();
        }
    }

    #[test]
    fn every_window_mutation_rejects_held_or_physical_input() {
        let mut owner = Owner::default();
        owner
            .reconcile(
                vec![window(1, 10)],
                vec![output(1, "main", 0, 1000)],
                |_| {},
            )
            .unwrap();
        let actions = [
            WindowAction::Activate,
            WindowAction::Minimize,
            WindowAction::Maximize,
            WindowAction::Restore,
            WindowAction::Fullscreen,
            WindowAction::ExitFullscreen,
            WindowAction::Close,
            WindowAction::MoveToWorkspace { workspace: 2 },
            WindowAction::SetBounds {
                x: 20,
                y: 20,
                width: 200,
                height: 200,
            },
        ];
        let busy = [
            WindowMutationInputState {
                keyboard_held: true,
                pointer_held: false,
                physical_input_idle: true,
            },
            WindowMutationInputState {
                keyboard_held: false,
                pointer_held: true,
                physical_input_idle: true,
            },
            WindowMutationInputState {
                keyboard_held: false,
                pointer_held: false,
                physical_input_idle: false,
            },
        ];
        for action in actions {
            owner
                .prepare_window_mutation(
                    &ResourceScope::FullSession,
                    action,
                    WindowMutationInputState {
                        keyboard_held: false,
                        pointer_held: false,
                        physical_input_idle: true,
                    },
                )
                .unwrap();
            for input in busy {
                assert!(
                    owner
                        .prepare_window_mutation(&ResourceScope::FullSession, action, input)
                        .is_err(),
                    "{action:?} accepted {input:?}"
                );
            }
        }
    }
    #[test]
    fn mutation_lookup_requires_exact_generation_and_applicable_scope() {
        let mut owned = window(1, 20);
        owned.application = Some("verified.app".into());
        let mut owner = Owner::default();
        owner
            .reconcile(vec![owned], vec![output(1, "main", 0, 1000)], |_| {})
            .unwrap();
        let summary = owner.windows(&ResourceScope::FullSession).next().unwrap().0;
        let window_scope = ResourceScope::Window(ResourceId {
            id: summary.id.clone(),
            generation: summary.generation,
        });
        assert!(
            owner
                .window_resource(&window_scope, &summary.id, summary.generation)
                .is_some()
        );
        assert!(
            owner
                .window_resource(
                    &ResourceScope::Application("verified.app".into()),
                    &summary.id,
                    summary.generation,
                )
                .is_some()
        );
        assert!(
            owner
                .window_resource(
                    &ResourceScope::Application("other.app".into()),
                    &summary.id,
                    summary.generation,
                )
                .is_none()
        );
        assert!(
            owner
                .window_resource(&window_scope, &summary.id, summary.generation + 1)
                .is_none()
        );
    }
    #[test]
    fn capture_dimensions_bound_pixels_and_wire_dimensions() {
        assert_eq!(capture_dimensions(640, 480), Ok((640, 480, 307_200)));
        for dimensions in [
            (0, 480),
            (640, 0),
            (-1, 480),
            (8193, 1),
            (1, 8193),
            (i32::from(u16::MAX) + 1, 1),
            (5000, 5000),
        ] {
            assert!(capture_dimensions(dimensions.0, dimensions.1).is_err());
        }
    }

    #[test]
    fn output_capture_lookup_requires_exact_generation_and_applicable_scope() {
        let mut owner = Owner::default();
        owner
            .reconcile(
                Vec::new(),
                vec![output(1, "left", -1000, 1000), output(2, "right", 0, 1500)],
                |_| {},
            )
            .unwrap();
        let left = owner
            .outputs(&ResourceScope::FullSession)
            .find(|(output, _)| output.name == "left")
            .unwrap()
            .0;
        let exact = ResourceScope::Output(ResourceId {
            id: left.name.clone(),
            generation: left.generation,
        });
        assert!(
            owner
                .output_capture_resource(&exact, &left.name, left.generation)
                .is_some()
        );
        assert!(
            owner
                .output_capture_resource(&ResourceScope::FullSession, &left.name, left.generation)
                .is_some()
        );
        assert!(
            owner
                .output_capture_resource(&exact, &left.name, left.generation + 1)
                .is_none()
        );
        assert!(
            owner
                .output_capture_resource(&exact, "right", left.generation)
                .is_none()
        );
        assert!(
            owner
                .output_capture_resource(
                    &ResourceScope::Window(ResourceId {
                        id: "window".into(),
                        generation: 1,
                    }),
                    &left.name,
                    left.generation,
                )
                .is_none()
        );
    }

    #[test]
    fn output_capture_chrome_requires_native_exclusion_for_visible_protection() {
        let ordinary = OutputCaptureChromeEvidence {
            on_requested_output: true,
            visible: true,
            protected: false,
            natively_excluded: false,
        };
        let hidden_protected = OutputCaptureChromeEvidence {
            protected: true,
            visible: false,
            ..ordinary
        };
        let protected_elsewhere = OutputCaptureChromeEvidence {
            protected: true,
            on_requested_output: false,
            ..ordinary
        };
        let trusted_excluded = OutputCaptureChromeEvidence {
            protected: true,
            natively_excluded: true,
            ..ordinary
        };
        assert!(output_capture_chrome_is_safe([
            ordinary,
            hidden_protected,
            protected_elsewhere,
            trusted_excluded,
        ]));
        assert!(!output_capture_chrome_is_safe([
            ordinary,
            OutputCaptureChromeEvidence {
                protected: true,
                ..ordinary
            },
        ]));
    }

    #[test]
    fn absolute_pointer_axis_handles_negative_virtual_desktops_and_edges() {
        assert_eq!(absolute_pointer_axis(-1920, -1920, 3840).unwrap(), 0);
        assert_eq!(absolute_pointer_axis(1919, -1920, 3840).unwrap(), 65_535);
        assert_eq!(absolute_pointer_axis(0, -1920, 3840).unwrap(), 32_776);
        assert_eq!(
            absolute_pointer_axis(-2, i32::MIN, i32::MAX).unwrap(),
            65_535
        );
        assert!(absolute_pointer_axis(-1921, -1920, 3840).is_err());
        assert!(absolute_pointer_axis(1920, -1920, 3840).is_err());
        assert!(absolute_pointer_axis(0, 0, 1).is_err());
    }

    #[test]
    fn window_pointer_coordinates_exclude_non_client_decorations_and_edges() {
        assert!(client_pointer_coordinate(0, 0, 800, 600).is_ok());
        assert!(client_pointer_coordinate(799, 599, 800, 600).is_ok());
        for point in [(-1, 0), (0, -1), (800, 0), (0, 600)] {
            assert!(client_pointer_coordinate(point.0, point.1, 800, 600).is_err());
        }
        assert!(client_pointer_coordinate(0, 0, 0, 600).is_err());
    }

    #[test]
    fn output_pointer_target_requires_exact_generation_and_global_membership() {
        let mut owner = Owner::default();
        owner
            .reconcile(
                vec![window(11, -500), window(22, 100)],
                vec![output(1, "left", -1000, 1000), output(2, "right", 0, 1500)],
                |_| {},
            )
            .unwrap();
        let left = owner
            .outputs(&ResourceScope::FullSession)
            .find(|(output, _)| output.name == "left")
            .unwrap()
            .0;
        let scope = ResourceScope::Output(ResourceId {
            id: left.name.clone(),
            generation: left.generation,
        });
        let exact = PointerTarget::Output {
            output_id: left.name.clone(),
            generation: left.generation,
        };

        assert!(matches!(
            owner.pointer_target_resource(&scope, &exact, -1000, 0),
            Some(PointerTargetResource::Global { x: -1000, y: 0, .. })
        ));
        assert!(
            owner
                .pointer_target_resource(&scope, &exact, -1, 999)
                .is_some()
        );
        assert!(
            owner
                .pointer_target_resource(&scope, &exact, 0, 10)
                .is_none()
        );
        assert!(
            owner
                .pointer_target_resource(
                    &scope,
                    &PointerTarget::Output {
                        output_id: left.name,
                        generation: left.generation + 1,
                    },
                    -500,
                    10,
                )
                .is_none()
        );
        assert!(owner.pointer_target_identity_is_live(&exact));
        owner
            .reconcile(
                vec![window(11, -500), window(22, 100)],
                vec![output(3, "left", -1000, 1000), output(2, "right", 0, 1500)],
                |_| {},
            )
            .unwrap();
        assert!(!owner.pointer_target_identity_is_live(&exact));
    }

    #[test]
    fn global_pointer_hits_fail_closed_for_unknown_protected_or_cross_output_windows() {
        let mut owner = Owner::default();
        owner
            .reconcile(
                vec![window(11, -500), window(22, 100)],
                vec![output(1, "left", -1000, 1000), output(2, "right", 0, 1500)],
                |_| {},
            )
            .unwrap();
        let outputs: Vec<_> = owner
            .outputs(&ResourceScope::FullSession)
            .map(|(output, _)| output)
            .collect();
        let left = outputs.iter().find(|output| output.name == "left").unwrap();
        let left_id = ResourceId {
            id: left.name.clone(),
            generation: left.generation,
        };
        let left_scope = ResourceScope::Output(left_id.clone());

        assert!(owner.pointer_hit_allowed(&left_scope, Some(&left_id), None));
        assert!(owner.pointer_hit_allowed(&left_scope, Some(&left_id), Some(11)));
        assert!(!owner.pointer_hit_allowed(&left_scope, Some(&left_id), Some(22)));
        assert!(!owner.pointer_hit_allowed(&left_scope, Some(&left_id), Some(999)));
        assert!(owner.pointer_hit_allowed(&ResourceScope::FullSession, None, Some(22)));

        let desktop = PointerTarget::Desktop;
        assert!(
            owner
                .pointer_target_resource(&ResourceScope::FullSession, &desktop, 100, 10)
                .is_some()
        );
        assert!(
            owner
                .pointer_target_resource(&left_scope, &desktop, -500, 10)
                .is_none()
        );
        assert!(
            owner
                .pointer_target_resource(&ResourceScope::FullSession, &desktop, 2000, 10)
                .is_none()
        );
    }
}
