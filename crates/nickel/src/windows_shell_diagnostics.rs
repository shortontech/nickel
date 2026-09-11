use crate::winit_shell::SurfaceRole;
use nickel_remote_control::diagnostics::{
    MAX_DIAGNOSTIC_SHELL_SURFACES, ShellDiagnosticRole, ShellSurfaceDiagnostic,
};

#[derive(Clone, Debug)]
pub(crate) struct SurfaceObservation {
    pub(crate) role: SurfaceRole,
    pub(crate) generation: u64,
    pub(crate) native_visible: bool,
    pub(crate) canonical_visible: bool,
    pub(crate) protected: bool,
    pub(crate) geometry: Option<[i64; 4]>,
    pub(crate) output: Option<String>,
    pub(crate) scene_generation: Option<u64>,
    pub(crate) scale_factor: f32,
    pub(crate) redraw_pending: bool,
    pub(crate) keyboard_focused: bool,
}

fn diagnostic_role(role: SurfaceRole) -> Option<ShellDiagnosticRole> {
    Some(match role {
        SurfaceRole::Desktop => ShellDiagnosticRole::Desktop,
        SurfaceRole::Panel => ShellDiagnosticRole::Panel,
        SurfaceRole::Launcher => ShellDiagnosticRole::Launcher,
        SurfaceRole::ControlCenter => ShellDiagnosticRole::ControlCenter,
        SurfaceRole::Notification => ShellDiagnosticRole::Notification,
        SurfaceRole::VolumeOsd => ShellDiagnosticRole::VolumeOsd,
        SurfaceRole::WindowPreview => ShellDiagnosticRole::WindowPreview,
        SurfaceRole::WindowContextMenu => ShellDiagnosticRole::WindowContextMenu,
        SurfaceRole::Screenshot => ShellDiagnosticRole::Screenshot,
        SurfaceRole::OnScreenKeyboard => ShellDiagnosticRole::OnScreenKeyboard,
        SurfaceRole::CodexProjectMenu | SurfaceRole::Lock | SurfaceRole::CodexChat => return None,
        #[cfg(target_os = "windows")]
        SurfaceRole::TrustedControl => return None,
    })
}

pub(crate) fn project(
    protected_desktop: bool,
    observations: impl IntoIterator<Item = SurfaceObservation>,
) -> (Vec<ShellSurfaceDiagnostic>, bool) {
    if protected_desktop {
        return (Vec::new(), false);
    }
    let mut eligible = observations.into_iter().filter_map(|observation| {
        let role = diagnostic_role(observation.role)?;
        if observation.protected
            || !observation.native_visible
            || !observation.canonical_visible
            || observation.generation == 0
            || !observation.scale_factor.is_finite()
            || observation.scale_factor <= 0.0
        {
            return None;
        }
        let geometry = observation.geometry?;
        if geometry[2] <= 0 || geometry[3] <= 0 {
            return None;
        }
        Some(ShellSurfaceDiagnostic {
            id: format!("windows-shell:{}", observation.generation),
            generation: observation.generation,
            role,
            geometry,
            output: observation
                .output
                .map(|output| output.chars().take(128).collect()),
            scene_generation: observation.scene_generation?,
            scale_factor: observation.scale_factor,
            redraw_pending: observation.redraw_pending,
            keyboard_focused: observation.keyboard_focused,
        })
    });
    let records = eligible
        .by_ref()
        .take(MAX_DIAGNOSTIC_SHELL_SURFACES)
        .collect();
    let truncated = eligible.next().is_some();
    (records, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(role: SurfaceRole, generation: u64) -> SurfaceObservation {
        SurfaceObservation {
            role,
            generation,
            native_visible: true,
            canonical_visible: true,
            protected: false,
            geometry: Some([10, 20, 300, 200]),
            output: Some("display-a".into()),
            scene_generation: Some(41),
            scale_factor: 1.25,
            redraw_pending: true,
            keyboard_focused: true,
        }
    }

    #[test]
    fn projects_only_visible_unprotected_ordinary_surfaces() {
        let mut protected = observation(SurfaceRole::Panel, 2);
        protected.protected = true;
        let mut hidden = observation(SurfaceRole::ControlCenter, 3);
        hidden.canonical_visible = false;
        let (records, truncated) = project(
            false,
            [
                observation(SurfaceRole::Launcher, 1),
                protected,
                hidden,
                observation(SurfaceRole::Lock, 4),
                observation(SurfaceRole::CodexProjectMenu, 5),
                observation(SurfaceRole::CodexChat, 6),
            ],
        );
        assert!(!truncated);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.id, "windows-shell:1");
        assert_eq!(record.generation, 1);
        assert_eq!(record.geometry, [10, 20, 300, 200]);
        assert_eq!(record.output.as_deref(), Some("display-a"));
        assert_eq!(record.scene_generation, 41);
        assert_eq!(record.scale_factor, 1.25);
        assert!(record.redraw_pending);
        assert!(record.keyboard_focused);
    }

    #[test]
    fn protected_desktop_removes_every_surface() {
        let (records, truncated) = project(true, [observation(SurfaceRole::Launcher, 1)]);
        assert!(records.is_empty());
        assert!(!truncated);
    }

    #[test]
    fn omits_records_without_valid_native_or_scene_evidence() {
        let mut missing_geometry = observation(SurfaceRole::Launcher, 1);
        missing_geometry.geometry = None;
        let mut missing_scene = observation(SurfaceRole::Panel, 2);
        missing_scene.scene_generation = None;
        let mut disconnected = observation(SurfaceRole::Desktop, 3);
        disconnected.native_visible = false;
        let (records, truncated) = project(false, [missing_geometry, missing_scene, disconnected]);
        assert!(records.is_empty());
        assert!(!truncated);
    }

    #[test]
    fn bounds_records_and_reports_truncation() {
        let observations = (1..=(MAX_DIAGNOSTIC_SHELL_SURFACES as u64 + 1))
            .map(|generation| observation(SurfaceRole::Panel, generation));
        let (records, truncated) = project(false, observations);
        assert_eq!(records.len(), MAX_DIAGNOSTIC_SHELL_SURFACES);
        assert!(truncated);
        assert_eq!(records[0].generation, 1);
        assert_eq!(
            records.last().unwrap().generation,
            MAX_DIAGNOSTIC_SHELL_SURFACES as u64
        );
    }
}
