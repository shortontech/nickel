use crate::winit_shell::SurfaceRole;
use nickel_remote_control::desktop_events::ShellEventRole;
use nickel_remote_control::diagnostics::{
    InternalRendererDiagnostic, MAX_DIAGNOSTIC_SHELL_SURFACES, ProjectedResourceDiagnostic,
    RendererFallbackReason, RendererPolicy, SharedPresenterCacheDiagnostic, ShellDiagnosticRole,
    ShellImageCacheDiagnostic, ShellSurfaceDiagnostic,
};

#[derive(Clone, Debug)]
pub(crate) struct SurfaceObservation {
    pub(crate) native: usize,
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
    pub(crate) presentation_generation: u64,
    pub(crate) presentation_failures: u64,
    pub(crate) presented_frame_bytes: u64,
}

/// Project only presenter state owned by the exact ordinary surface. The
/// software rasterizer and its caches are shared, so this deliberately excludes
/// their allocation and cache accounting.
pub(crate) fn project_presenters(
    protected_desktop: bool,
    observed_at_us: u64,
    observations: impl IntoIterator<Item = SurfaceObservation>,
) -> Vec<InternalRendererDiagnostic> {
    if protected_desktop {
        return Vec::new();
    }
    observations
        .into_iter()
        .filter_map(|observation| {
            diagnostic_role(observation.role)?;
            if observation.protected
                || !observation.native_visible
                || !observation.canonical_visible
                || observation.generation == 0
                || observation.scene_generation.is_none()
            {
                return None;
            }
            Some(InternalRendererDiagnostic {
                surface: format!("windows-shell:{}", observation.generation),
                surface_generation: observation.generation,
                observed_at_us,
                mode: if observation.presentation_generation == 0 {
                    "pending"
                } else {
                    "software_shared_memory"
                }
                .into(),
                configured_mode: RendererPolicy::Software,
                fallback_reason: Some(RendererFallbackReason::RequestedSoftware),
                gpu_frames: 0,
                // Native present completion is reported separately below;
                // the shared renderer does not expose per-surface render work.
                fallback_frames: 0,
                software_frame_bytes: observation.presented_frame_bytes,
                fallback_raster_bytes: 0,
                fallback_buffer_creations: 0,
                fallback_buffer_reuses: 0,
                fallback_upload_damage_bytes: 0,
                fallback_full_repaints: 0,
                fallback_partial_repaints: 0,
                texture_import_failures: 0,
                fallback_import_failures: 0,
                presentation_generation: Some(observation.presentation_generation),
                presentation_failures: Some(observation.presentation_failures),
            })
        })
        .take(MAX_DIAGNOSTIC_SHELL_SURFACES)
        .collect()
}

pub(crate) fn input_surface(
    protected_desktop: bool,
    native: usize,
    observations: &[SurfaceObservation],
) -> Option<&SurfaceObservation> {
    if protected_desktop || native == 0 {
        return None;
    }
    observations.iter().find(|observation| {
        observation.native == native
            && observation.generation != 0
            && observation.native_visible
            && observation.canonical_visible
            && !observation.protected
            && observation
                .geometry
                .is_some_and(|geometry| geometry[2] > 0 && geometry[3] > 0)
            && observation.scene_generation.is_some()
            && observation.scale_factor.is_finite()
            && observation.scale_factor > 0.0
            && diagnostic_role(observation.role).is_some()
    })
}

pub(crate) fn semantic_node_at(
    scale_factor: f32,
    client_point: [i32; 2],
    bounds: impl IntoIterator<Item = [f32; 4]>,
) -> Option<u64> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 || client_point[1] < 0 {
        return None;
    }
    let x = client_point[0] as f64 / f64::from(scale_factor);
    let y = client_point[1] as f64 / f64::from(scale_factor);
    bounds
        .into_iter()
        .enumerate()
        .filter(|(_, bounds)| {
            let [left, top, width, height] = *bounds;
            width.is_finite()
                && height.is_finite()
                && width > 0.0
                && height > 0.0
                && x >= f64::from(left)
                && y >= f64::from(top)
                && x < f64::from(left + width)
                && y < f64::from(top + height)
        })
        .min_by(|(_, left), (_, right)| {
            let left_area = left[2] * left[3];
            let right_area = right[2] * right[3];
            left_area.total_cmp(&right_area)
        })
        .map(|(ordinal, _)| ordinal as u64)
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

fn event_role(role: SurfaceRole) -> Option<ShellEventRole> {
    Some(match role {
        SurfaceRole::Desktop => ShellEventRole::Desktop,
        SurfaceRole::Panel => ShellEventRole::Panel,
        SurfaceRole::Launcher => ShellEventRole::Launcher,
        SurfaceRole::ControlCenter => ShellEventRole::ControlCenter,
        SurfaceRole::Notification => ShellEventRole::Notification,
        SurfaceRole::VolumeOsd => ShellEventRole::VolumeOsd,
        SurfaceRole::WindowPreview => ShellEventRole::WindowPreview,
        SurfaceRole::WindowContextMenu => ShellEventRole::WindowContextMenu,
        SurfaceRole::Screenshot => ShellEventRole::Screenshot,
        SurfaceRole::OnScreenKeyboard => ShellEventRole::OnScreenKeyboard,
        SurfaceRole::CodexProjectMenu | SurfaceRole::Lock | SurfaceRole::CodexChat => return None,
        #[cfg(target_os = "windows")]
        SurfaceRole::TrustedControl => return None,
    })
}

/// Project one exact native/canonical visibility transition. The event stream
/// never retains protected shell identities, and a stale queued winit event is
/// ignored when current native and production visibility no longer agree with it.
pub(crate) fn project_visibility_event(
    protected_desktop: bool,
    observation: &SurfaceObservation,
    visible: bool,
) -> Option<(u64, ShellEventRole)> {
    if protected_desktop
        || observation.protected
        || observation.generation == 0
        || observation.native_visible != visible
        || observation.canonical_visible != visible
    {
        return None;
    }
    Some((observation.generation, event_role(observation.role)?))
}

/// Project current ordinary keyboard focus. Absence deliberately represents
/// protected, hidden, stale and unsupported recipients alike.
pub(crate) fn project_focus_event(
    protected_desktop: bool,
    observation: &SurfaceObservation,
) -> Option<(u64, ShellEventRole)> {
    if protected_desktop
        || observation.protected
        || observation.generation == 0
        || !observation.native_visible
        || !observation.canonical_visible
        || !observation.keyboard_focused
    {
        return None;
    }
    Some((observation.generation, event_role(observation.role)?))
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

/// Project only process-level cache-owner aggregates. The shared rasterizer
/// cannot truthfully assign an entry or byte to one surface, so this record is
/// deliberately separate from protected-filtered renderer records.
pub(crate) fn project_presenter_cache(
    observation_generation: u64,
    observed_at_us: u64,
    memory: crate::winit_shell::ShellMemoryDiagnostics,
) -> SharedPresenterCacheDiagnostic {
    let cache = memory.presenter_caches;
    SharedPresenterCacheDiagnostic {
        observation_generation,
        observed_at_us,
        cache_generation: memory.presenter_cache_generation,
        cache_owners: bounded_u64(cache.presenters),
        live_entries: bounded_u64(cache.live_entries),
        live_bytes: bounded_u64(cache.live_bytes),
        peak_cache_bytes: bounded_u64(cache.peak_cache_bytes),
        hits: cache.hits,
        misses: cache.misses,
        insertions: cache.insertions,
        evictions: cache.evictions,
        invalidations: cache.invalidations,
        recomputation_nanos: cache.recomputation_nanos,
    }
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
            native: generation as usize + 100,
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
            presentation_generation: 3,
            presentation_failures: 1,
            presented_frame_bytes: 240_000,
        }
    }

    #[test]
    fn presenter_projection_uses_only_per_surface_production_state() {
        let mut protected = observation(SurfaceRole::Panel, 2);
        protected.protected = true;
        let records = project_presenters(
            false,
            91,
            [observation(SurfaceRole::Launcher, 1), protected],
        );
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.surface, "windows-shell:1");
        assert_eq!(record.observed_at_us, 91);
        assert_eq!(record.presentation_generation, Some(3));
        assert_eq!(record.presentation_failures, Some(1));
        assert_eq!(record.fallback_frames, 0);
        assert_eq!(record.software_frame_bytes, 240_000);
        assert_eq!(record.fallback_raster_bytes, 0);
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
    fn input_correlation_requires_exact_current_ordinary_surface() {
        let visible = observation(SurfaceRole::Launcher, 7);
        assert_eq!(
            input_surface(false, 107, std::slice::from_ref(&visible))
                .unwrap()
                .generation,
            7
        );
        assert!(input_surface(false, 108, std::slice::from_ref(&visible)).is_none());
        assert!(input_surface(true, 107, std::slice::from_ref(&visible)).is_none());

        let mut hidden = visible.clone();
        hidden.native_visible = false;
        assert!(input_surface(false, 107, &[hidden]).is_none());
        let mut protected = visible.clone();
        protected.protected = true;
        assert!(input_surface(false, 107, &[protected]).is_none());
        let mut replaced = visible;
        replaced.generation = 0;
        assert!(input_surface(false, 107, &[replaced]).is_none());
    }

    #[test]
    fn input_correlation_omits_protected_fixed_roles() {
        assert!(input_surface(false, 105, &[observation(SurfaceRole::Lock, 5)]).is_none());
        assert!(input_surface(false, 106, &[observation(SurfaceRole::CodexChat, 6)]).is_none());
    }

    #[test]
    fn semantic_hit_uses_client_scale_and_prefers_smallest_node() {
        assert_eq!(
            semantic_node_at(
                2.0,
                [80, 60],
                [[0.0, 0.0, 100.0, 100.0], [30.0, 20.0, 20.0, 20.0]]
            ),
            Some(1)
        );
        assert_eq!(
            semantic_node_at(2.0, [220, 60], [[0.0, 0.0, 100.0, 100.0]]),
            None
        );
        assert_eq!(
            semantic_node_at(2.0, [80, -1], [[0.0, 0.0, 100.0, 100.0]]),
            None
        );
        assert_eq!(
            semantic_node_at(f32::NAN, [80, 60], [[0.0, 0.0, 100.0, 100.0]]),
            None
        );
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

    #[test]
    fn shared_presenter_projection_is_generation_correlated_and_aggregate_only() {
        let record = project_presenter_cache(
            17,
            23,
            crate::winit_shell::ShellMemoryDiagnostics {
                presenter_cache_generation: 11,
                presenter_caches: nickel_ui::AggregatePresenterCacheDiagnostics {
                    presenters: 1,
                    live_entries: 7,
                    live_bytes: 80,
                    peak_cache_bytes: 120,
                    hits: 9,
                    misses: 2,
                    insertions: 3,
                    evictions: 1,
                    invalidations: 4,
                    recomputation_nanos: 50,
                },
                process_rss_bytes: Some(9_999),
            },
        );
        assert_eq!(record.observation_generation, 17);
        assert_eq!(record.observed_at_us, 23);
        assert_eq!(record.cache_generation, 11);
        assert_eq!(record.cache_owners, 1);
        assert_eq!(record.live_entries, 7);
        assert_eq!(record.live_bytes, 80);
        assert_eq!(record.peak_cache_bytes, 120);
    }

    #[test]
    fn shell_event_projection_requires_current_ordinary_unprotected_evidence() {
        let visible = observation(SurfaceRole::Launcher, 17);
        assert_eq!(
            project_visibility_event(false, &visible, true),
            Some((17, ShellEventRole::Launcher))
        );
        assert_eq!(
            project_focus_event(false, &visible),
            Some((17, ShellEventRole::Launcher))
        );

        let mut hidden = visible.clone();
        hidden.native_visible = false;
        hidden.canonical_visible = false;
        hidden.keyboard_focused = false;
        assert_eq!(
            project_visibility_event(false, &hidden, false),
            Some((17, ShellEventRole::Launcher))
        );
        assert!(project_visibility_event(false, &hidden, true).is_none());
        assert!(project_focus_event(false, &hidden).is_none());

        let mut protected = visible.clone();
        protected.protected = true;
        assert!(project_visibility_event(false, &protected, true).is_none());
        assert!(project_focus_event(false, &protected).is_none());
        assert!(project_visibility_event(true, &visible, true).is_none());
        assert!(project_focus_event(true, &visible).is_none());

        let codex = observation(SurfaceRole::CodexChat, 18);
        assert!(project_visibility_event(false, &codex, true).is_none());
        assert!(project_focus_event(false, &codex).is_none());
        #[cfg(target_os = "windows")]
        let trusted = observation(SurfaceRole::TrustedControl, 19);
        #[cfg(target_os = "windows")]
        assert!(project_visibility_event(false, &trusted, true).is_none());
        #[cfg(target_os = "windows")]
        assert!(project_focus_event(false, &trusted).is_none());
    }

    #[test]
    fn stale_visibility_and_focus_events_fail_closed() {
        let mut stale = observation(SurfaceRole::Screenshot, 23);
        stale.canonical_visible = false;
        assert!(project_visibility_event(false, &stale, true).is_none());
        assert!(project_visibility_event(false, &stale, false).is_none());
        assert!(project_focus_event(false, &stale).is_none());

        stale.generation = 0;
        stale.native_visible = false;
        stale.keyboard_focused = false;
        assert!(project_visibility_event(false, &stale, false).is_none());
    }
}
