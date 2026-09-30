//! Host-owned shell preview. Retained prior mounts are never exposed to candidate ingress.
use super::*;
use std::collections::BTreeMap;

type SurfaceHosts = BTreeMap<
    nickel_core::plugins::PluginSurfaceKey,
    (
        nickel_core::plugins::PluginSurface,
        nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>,
    ),
>;
pub(super) struct ShellPreview {
    pub token: u64,
    pub owner: String,
    pub previous: String,
    pub selected: String,
    pub deadline: Instant,
    pub deadline_unix_milliseconds: u64,
    previous_hosts: SurfaceHosts,
    previous_placements: BTreeMap<
        nickel_core::plugins::PluginSurfaceKey,
        (nickel_core::plugins::PluginSurfaceAnchor, i32, i32),
    >,
}
impl LiveShell {
    pub(super) fn shell_source_current(&self, owner: &str) -> bool {
        self.native_ui_service_granted(
            owner,
            nickel_core::plugins::PluginCapability::PluginsControl,
        ) && self.plugin_registry.get(owner).is_some_and(|entry| {
            entry
                .manifest
                .capabilities
                .contains(&nickel_core::plugins::PluginCapability::PluginsRead)
        })
    }
    pub(super) fn shell_runtime_current(&self, id: &str) -> bool {
        self.package_runtimes.contains_key(id)
            && self.plugin_registry.get(id).is_some_and(|entry| {
                entry.desired_enabled && entry.health == nickel_core::plugins::PluginHealth::Running
            })
    }
    pub(super) fn shell_preview_snapshot(&self, owner: &str) -> serde_json::Value {
        self.shell_selection_preview.as_ref().map_or(serde_json::Value::Null,|preview| {
            let current = self.shell_source_current(owner) && preview.owner == owner && preview.selected == self.active_shell_package_id;
            serde_json::json!({"token":preview.token.to_string(),"previousShell":preview.previous,"selectedShell":preview.selected,"deadlineUnixMilliseconds":preview.deadline_unix_milliseconds,
                "canConfirm":current && Instant::now()<preview.deadline && self.shell_runtime_current(&preview.selected),"canRevert":current})
        })
    }
    pub(super) fn take_shell_hosts(&mut self, id: &str) -> SurfaceHosts {
        let keys = self
            .plugin_surface_hosts
            .keys()
            .filter(|key| key.plugin_id == id)
            .cloned()
            .collect::<Vec<_>>();
        keys.into_iter()
            .filter_map(|key| {
                self.plugin_surface_hosts
                    .remove(&key)
                    .map(|value| (key, value))
            })
            .collect()
    }
    pub(super) fn retire_shell_hosts(hosts: SurfaceHosts) {
        for (_, (_, host)) in hosts {
            let _ = host.application().retire_surface();
        }
    }
    pub(super) fn begin_shell_preview(&mut self, owner: &str, id: &str) -> Result<bool, String> {
        if self.shell_selection_preview.is_some() {
            return Err("a shell preview is already pending".into());
        }
        if self.display_preview.is_some()
            || self.projection_rollback_deadline.is_some()
            || self.projection_chooser.pending().is_some()
        {
            return Err("a display preview is already pending".into());
        }
        #[cfg(target_os = "windows")]
        if crate::windows_plugin_display::read(owner).pending_confirmation {
            return Err("a display preview is already pending".into());
        }
        if !self.shell_source_current(owner) {
            return Err("shell preview source is unavailable".into());
        }
        if self.active_shell_package_id == id {
            return Ok(false);
        }
        if !self.is_shell_package(id) {
            return Err("package does not export a shell".into());
        }
        let token = self
            .shell_preview_sequence
            .checked_add(1)
            .ok_or("shell preview identity exhausted")?;
        // Candidate construction happens before detaching any known-good presentation.
        self.set_plugin_enabled(id, true)?;
        if !self.shell_runtime_current(id) {
            return Err("shell runtime is unavailable".into());
        }
        let previous = self.active_shell_package_id.clone();
        let previous_hosts = self.take_shell_hosts(&previous);
        let previous_placements = self
            .plugin_window_placement_overrides
            .iter()
            .filter(|(key, _)| key.plugin_id == previous)
            .map(|(key, value)| (key.clone(), *value))
            .collect();
        if let Err(error) = self.select_shell_package(id) {
            self.plugin_surface_hosts.extend(previous_hosts);
            self.plugin_window_placement_overrides
                .extend(previous_placements);
            return Err(error);
        }
        self.shell_preview_sequence = token;
        let deadline = Instant::now() + Duration::from_secs(15);
        let deadline_unix_milliseconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .saturating_add(15_000)
            .min(u64::MAX as u128) as u64;
        // Preserve open windows where the selected shell also declares them, without assuming Settings.
        let common = previous_hosts
            .keys()
            .map(|key| key.surface_id.clone())
            .filter(|surface| self.active_shell_declares(surface))
            .collect::<Vec<_>>();
        self.shell_selection_preview = Some(ShellPreview {
            token,
            owner: owner.into(),
            previous,
            selected: id.into(),
            deadline,
            deadline_unix_milliseconds,
            previous_hosts,
            previous_placements,
        });
        self.control_host
            .application_mut()
            .show_shell_preview(token);
        self.control_visible = true;
        for surface in common {
            let _ = self.show_plugin_window(id, &surface);
        }
        #[cfg(target_os = "linux")]
        let _ = self.send_session_command(
            "shell-preview-recovery-focus",
            ShellCommand::FocusControlCenter,
        );
        Ok(true)
    }
    pub(super) fn persist_confirmed_shell(&mut self, id: &str) -> Result<(), String> {
        #[cfg(not(test))]
        nickel_core::plugins::PluginActivationSettings::select_shell_default(id)
            .map_err(|error| error.to_string())?;
        self.confirmed_shell_package_id = id.into();
        Ok(())
    }
    pub(super) fn confirm_shell_preview(
        &mut self,
        owner: Option<&str>,
        token: u64,
    ) -> Result<bool, String> {
        let preview = self
            .shell_selection_preview
            .as_ref()
            .ok_or("no shell preview is pending")?;
        if preview.token != token || owner.is_some_and(|owner| preview.owner != owner) {
            return Err("shell preview token is stale".into());
        }
        if !self.shell_source_current(&preview.owner)
            || Instant::now() >= preview.deadline
            || !self.shell_runtime_current(&preview.selected)
            || self.active_shell_package_id != preview.selected
        {
            return Err("shell preview confirmation is unavailable".into());
        }
        let selected = preview.selected.clone();
        self.persist_confirmed_shell(&selected)?;
        let preview = self
            .shell_selection_preview
            .take()
            .expect("validated preview");
        Self::retire_shell_hosts(preview.previous_hosts);
        self.control_host.application_mut().dismiss_shell_preview();
        self.control_visible = self
            .control_host
            .application()
            .view_state()
            .trusted_visible();
        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        self.plugins_results.insert(
            preview.owner,
            serde_json::json!({"status":"confirmed","selectedShell":selected}),
        );
        Ok(true)
    }
    pub(super) fn revert_shell_preview(
        &mut self,
        owner: Option<&str>,
        token: u64,
        reason: &str,
    ) -> Result<bool, String> {
        let preview = self
            .shell_selection_preview
            .as_ref()
            .ok_or("no shell preview is pending")?;
        if preview.token != token
            || owner
                .is_some_and(|owner| preview.owner != owner || !self.shell_source_current(owner))
        {
            return Err("shell preview token or source is stale".into());
        }
        let mut preview = self
            .shell_selection_preview
            .take()
            .expect("validated preview");
        self.retire_preview_plugin_state();
        let candidate_hosts = self.take_shell_hosts(&preview.selected);
        Self::retire_shell_hosts(candidate_hosts);
        if self.shell_runtime_current(&preview.previous) {
            self.active_shell_package_id = preview.previous.clone();
            self.plugin_surface_hosts
                .extend(std::mem::take(&mut preview.previous_hosts));
            self.plugin_window_placement_overrides
                .extend(std::mem::take(&mut preview.previous_placements));
        } else if let Err(error) = self.select_shell_package("nickel-default") {
            // Keep native recovery and a retry deadline even if a previous provider was disabled.
            preview.deadline = Instant::now() + Duration::from_secs(1);
            self.shell_selection_preview = Some(preview);
            return Err(error);
        } else {
            Self::retire_shell_hosts(std::mem::take(&mut preview.previous_hosts));
        }
        self.control_host.application_mut().dismiss_shell_preview();
        self.control_visible = self
            .control_host
            .application()
            .view_state()
            .trusted_visible();
        self.cancel_keyboard_gestures();
        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        self.plugins_results.insert(preview.owner,serde_json::json!({"status":"reverted","selectedShell":self.active_shell_package_id,"detail":reason}));
        Ok(true)
    }
    pub(super) fn recover_pending_shell(&mut self, reason: &str) -> bool {
        let Some(token) = self
            .shell_selection_preview
            .as_ref()
            .map(|preview| preview.token)
        else {
            return false;
        };
        match self.revert_shell_preview(None, token, reason) {
            Ok(changed) => changed,
            Err(error) => {
                tracing::warn!(%error,"shell preview recovery failed");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_core::plugins::{PluginPackage, PluginPackageSource, PluginSurfaceKey};
    fn with_stack(test: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(test)
            .unwrap()
            .join()
            .unwrap();
    }
    fn install(shell: &mut LiveShell, id: &str, empty: bool, broken: bool) {
        let surfaces = if empty {
            serde_json::json!([])
        } else {
            serde_json::json!([{"id":"taskbar","kind":"panel","width":800,"height":42,"initially_open":true}])
        };
        let manifest=serde_json::json!({"api_version":1,"id":id,"name":id,"version":"1.0.0","entry":"main.js","composition":{"api_version":1,"id":id,"version":"1.0.0","exports":{"shell":"./main.js#Shell"}},"surfaces":surfaces}).to_string();
        let source = if broken {
            "export function Shell(){throw Error('preview failed');}"
        } else if empty {
            "export function Shell(){return null;}"
        } else {
            "export function Shell(){return h(FixedWindow,{id:'taskbar',width:800,height:42},h(Text,{},'Candidate'));}"
        };
        let package = PluginPackage::from_embedded(&[
            ("plugin.json", manifest.as_bytes()),
            ("main.js", source.as_bytes()),
        ])
        .unwrap();
        shell
            .plugin_registry
            .register(package.manifest.clone())
            .unwrap();
        shell
            .external_plugin_packages
            .insert(id.into(), PluginPackageSource::embedded(package));
    }
    fn start(shell: &mut LiveShell, id: &str) {
        let snapshot = shell.plugin_management("nickel-default").unwrap();
        let effect = crate::plugins_capabilities::ShellSelectionEffect::parse(
            &serde_json::json!({"id":id,"revision":snapshot["revision"]}),
        )
        .unwrap();
        assert!(shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::ShellSelection {
                plugin_id: "nickel-default".into(),
                effect
            }
        ]));
    }
    fn decide(shell: &mut LiveShell, token: u64, confirm: bool) {
        let snapshot = shell.plugin_management("nickel-default").unwrap();
        let effect=crate::plugins_capabilities::ShellPreviewDecision::parse(&serde_json::json!({"type":if confirm {"plugins.confirmShell"} else {"plugins.revertShell"},"token":token.to_string(),"revision":snapshot["revision"]})).unwrap();
        shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::ShellPreviewDecision {
                plugin_id: "nickel-default".into(),
                effect,
            },
        ]);
    }
    #[test]
    fn shell_preview_preserves_recovery_on_focus_loss_and_reverts_on_escape() {
        with_stack(|| {
            let mut shell = LiveShell::new().unwrap();
            install(&mut shell, "preview-focus", true, false);
            start(&mut shell, "preview-focus");
            assert!(!shell.dismiss_ephemeral_on_focus_loss(SurfaceRole::ControlCenter));
            assert!(shell.control_visible);
            assert!(
                shell
                    .control_host
                    .application()
                    .view_state()
                    .trusted_visible()
            );
            assert!(
                !shell.preview_projection(nickel_core::display_projection::ProjectionMode::Extend)
            );
            assert!(shell.control_key(Some(KeyCode::Escape), 800, 600));
            assert!(shell.shell_selection_preview.is_none());
            assert_eq!(shell.active_shell_package_id, "nickel-default");
        });
    }

    #[test]
    fn shell_preview_rejects_concurrent_display_transaction() {
        with_stack(|| {
            let mut shell = LiveShell::new().unwrap();
            install(&mut shell, "preview-concurrent", false, false);
            shell.projection_rollback_deadline = Some(Instant::now() + Duration::from_secs(15));
            assert!(
                shell
                    .begin_shell_preview("nickel-default", "preview-concurrent")
                    .is_err()
            );
            assert_eq!(shell.active_shell_package_id, "nickel-default");
            assert!(shell.shell_selection_preview.is_none());
            assert!(shell.projection_rollback_deadline.is_some());
        });
    }

    #[test]
    fn shell_preview_expiry_restores_retained_previous_host_without_persisting() {
        with_stack(|| {
            let mut shell = LiveShell::new().unwrap();
            install(&mut shell, "preview-expiry", false, false);
            let key = PluginSurfaceKey {
                plugin_id: "nickel-default".into(),
                surface_id: "taskbar".into(),
            };
            let runtime = shell
                .plugin_panel_host_ref(&key)
                .unwrap()
                .application()
                .shared_composition_runtime()
                .unwrap();
            start(&mut shell, "preview-expiry");
            assert_eq!(shell.active_shell_package_id, "preview-expiry");
            assert_eq!(shell.confirmed_shell_package_id, "nickel-default");
            assert!(
                shell
                    .control_host
                    .application()
                    .view_state()
                    .trusted_visible()
            );
            let preview = shell.shell_selection_preview.as_ref().unwrap();
            let deadline = preview.deadline;
            assert!(
                shell
                    .host_deadline_sources()
                    .iter()
                    .any(|(name, time)| *name == "shell-selection-preview" && *time == deadline)
            );
            assert!(
                shell
                    .poll_deadlines(deadline + Duration::from_millis(1))
                    .visibility_changed
            );
            assert_eq!(shell.active_shell_package_id, "nickel-default");
            assert_eq!(shell.confirmed_shell_package_id, "nickel-default");
            assert!(shell.shell_selection_preview.is_none());
            assert!(std::rc::Rc::ptr_eq(
                &runtime,
                &shell
                    .plugin_panel_host_ref(&key)
                    .unwrap()
                    .application()
                    .shared_composition_runtime()
                    .unwrap()
            ));
        });
    }
    #[test]
    fn shell_preview_requires_current_revision_token_and_confirmation_to_persist() {
        with_stack(|| {
            let mut shell = LiveShell::new().unwrap();
            install(&mut shell, "preview-confirm", false, false);
            start(&mut shell, "preview-confirm");
            let token = shell.shell_selection_preview.as_ref().unwrap().token;
            decide(&mut shell, token + 1, true);
            assert!(shell.shell_selection_preview.is_some());
            assert_eq!(
                shell.plugins_results["nickel-default"]["status"],
                "rejected"
            );
            let effect = crate::plugins_capabilities::ShellPreviewDecision {
                token,
                revision: 0,
                confirm: true,
            };
            shell.apply_plugin_effects(vec![
                crate::plugin_panel::PluginEffect::ShellPreviewDecision {
                    plugin_id: "nickel-default".into(),
                    effect,
                },
            ]);
            assert!(shell.shell_selection_preview.is_some());
            decide(&mut shell, token, true);
            assert_eq!(shell.confirmed_shell_package_id, "preview-confirm");
            assert!(shell.shell_selection_preview.is_none());
            assert_eq!(
                shell.plugins_results["nickel-default"]["status"],
                "confirmed"
            );
            decide(&mut shell, token, false);
            assert_eq!(shell.active_shell_package_id, "preview-confirm");
        });
    }
    #[test]
    fn shell_preview_runtime_failure_restores_previous_presentation() {
        with_stack(|| {
            let mut shell = LiveShell::new().unwrap();
            install(&mut shell, "preview-failure", false, false);
            start(&mut shell, "preview-failure");
            assert!(
                shell.fail_installed_plugin_runtime(
                    "preview-failure",
                    "late callback failed".into()
                )
            );
            assert_eq!(shell.active_shell_package_id, "nickel-default");
            assert!(shell.shell_selection_preview.is_none());
            assert!(shell.plugin_surface_matches(&shell.active_shell_surface_key("taskbar")));
            assert_eq!(
                shell.plugins_results["nickel-default"]["status"],
                "reverted"
            );
        });
    }
    #[test]
    fn shell_preview_native_recovery_works_without_settings_or_any_candidate_surface() {
        with_stack(|| {
            let mut shell = LiveShell::new().unwrap();
            install(&mut shell, "preview-empty", true, false);
            start(&mut shell, "preview-empty");
            assert_eq!(shell.active_shell_package_id, "preview-empty");
            assert!(shell.shell_selection_preview.is_some());
            assert!(!shell.active_shell_declares("settings"));
            assert!(
                !shell
                    .plugin_surface_hosts
                    .keys()
                    .any(|key| key.plugin_id == "preview-empty")
            );
            for width in [280, 380, 420] {
                shell.control_host.step(HostBatch {
                    surface_size: Some((width, 600)),
                    events: vec![HostEvent::Poll],
                    ..Default::default()
                });
                for label in ["Restore previous shell", "Keep this shell"] {
                    let target = shell
                        .control_host
                        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                            role: SemanticRole::Button,
                            name: label.into(),
                        })
                        .unwrap();
                    assert!(
                        target.bounds.size.width >= 150.0,
                        "{label} must remain readable"
                    );
                    assert!(target.bounds.origin.x >= 0.0);
                    assert!(target.bounds.origin.x + target.bounds.size.width <= width as f32);
                }
            }
            let button = shell
                .control_host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Restore previous shell".into(),
                })
                .unwrap();
            shell
                .control_host
                .handle_event(UiEvent::AccessibilityActivate(button.id));
            shell.apply_control_effects();
            assert_eq!(shell.active_shell_package_id, "nickel-default");
            assert!(shell.shell_selection_preview.is_none());
        });
    }
    #[test]
    fn shell_preview_disabled_requester_recovers_to_trusted_default_and_rejects_old_token() {
        with_stack(|| {
            let mut shell = LiveShell::new().unwrap();
            install(&mut shell, "preview-disable", false, false);
            start(&mut shell, "preview-disable");
            let token = shell.shell_selection_preview.as_ref().unwrap().token;
            shell.set_plugin_enabled("nickel-default", false).unwrap();
            assert_eq!(shell.active_shell_package_id, "nickel-default");
            assert!(shell.shell_selection_preview.is_none());
            assert!(shell.shell_runtime_current("nickel-default"));
            decide(&mut shell, token, true);
            assert_eq!(shell.active_shell_package_id, "nickel-default");
        });
    }
    #[test]
    fn shell_preview_constructor_failure_keeps_previous_host_and_no_pending_transaction() {
        with_stack(|| {
            let mut shell = LiveShell::new().unwrap();
            install(&mut shell, "preview-broken", false, true);
            start(&mut shell, "preview-broken");
            assert_eq!(shell.active_shell_package_id, "nickel-default");
            assert!(shell.shell_selection_preview.is_none());
            assert_eq!(shell.confirmed_shell_package_id, "nickel-default");
            assert_eq!(
                shell.plugins_results["nickel-default"]["status"],
                "rejected"
            );
            assert!(shell.plugin_surface_matches(&shell.active_shell_surface_key("taskbar")));
        });
    }
}
