use crate::winit_shell::SurfaceRole;
use nickel_remote_control::diagnostics::{
    MAX_DIAGNOSTIC_SHELL_SURFACES, ProjectedResourceDiagnostic, ShellDiagnosticRole,
    ShellImageCacheDiagnostic, ShellSurfaceDiagnostic,
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

fn bounded_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Project count and retained-byte accounting only. No cache keys, image
/// dimensions, source paths, pixels, titles, or application identities cross
/// this boundary. Preview accounting must already be filtered to the ordinary
/// windows present in the containing diagnostic snapshot.
pub(crate) fn project_image_cache(
    observation_generation: u64,
    observed_at_us: u64,
    cache: crate::live_shell::ShellImageCacheDiagnostics,
) -> (ShellImageCacheDiagnostic, ProjectedResourceDiagnostic) {
    let cache = ShellImageCacheDiagnostic {
        observation_generation,
        observed_at_us,
        launcher_icon_entries: bounded_u64(cache.launcher_icon_entries),
        launcher_icon_bytes: bounded_u64(cache.launcher_icon_bytes),
        wallpaper_entries: bounded_u64(cache.wallpaper_entries),
        wallpaper_bytes: bounded_u64(cache.wallpaper_bytes),
        tray_entries: bounded_u64(cache.tray_entries),
        tray_bytes: bounded_u64(cache.tray_bytes),
        preview_entries: bounded_u64(cache.preview_entries),
        preview_bytes: bounded_u64(cache.preview_bytes),
    };
    let projected = ProjectedResourceDiagnostic {
        observation_generation,
        observed_at_us,
        renderer_surfaces: 0,
        software_frame_bytes: 0,
        fallback_raster_bytes: 0,
        shell_image_entries: cache
            .launcher_icon_entries
            .saturating_add(cache.wallpaper_entries)
            .saturating_add(cache.tray_entries)
            .saturating_add(cache.preview_entries),
        shell_image_bytes: cache
            .launcher_icon_bytes
            .saturating_add(cache.wallpaper_bytes)
            .saturating_add(cache.tray_bytes)
            .saturating_add(cache.preview_bytes),
    };
    (cache, projected)
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

    #[test]
    fn image_cache_projection_is_generation_correlated_and_aggregate_only() {
        let (cache, projected) = project_image_cache(
            17,
            23,
            crate::live_shell::ShellImageCacheDiagnostics {
                launcher_icon_entries: 2,
                launcher_icon_bytes: 20,
                wallpaper_entries: 1,
                wallpaper_bytes: 30,
                tray_entries: 4,
                tray_bytes: 40,
                preview_entries: 3,
                preview_bytes: 50,
            },
        );
        assert_eq!(cache.observation_generation, 17);
        assert_eq!(cache.observed_at_us, 23);
        assert_eq!(cache.preview_entries, 3);
        assert_eq!(cache.preview_bytes, 50);
        assert_eq!(projected.observation_generation, 17);
        assert_eq!(projected.observed_at_us, 23);
        assert_eq!(projected.shell_image_entries, 10);
        assert_eq!(projected.shell_image_bytes, 140);
        assert_eq!(projected.renderer_surfaces, 0);
        assert_eq!(projected.software_frame_bytes, 0);
        assert_eq!(projected.fallback_raster_bytes, 0);
    }
}
