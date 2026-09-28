//! Read the production shell hosts without switching their active viewport.
use super::*;
use nickel_remote_control::semantics::{MAX_PAYLOAD_BYTES, MAX_RESOLVED_NODES};

type Projection = (u64, Vec<nickel_ui::SemanticNodeSnapshot>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoteActionDisposition {
    Guarded,
    Unavailable,
}

fn action_disposition(
    action: nickel_ui::ActionKind,
    callback: RemoteActionDisposition,
) -> RemoteActionDisposition {
    use nickel_ui::ActionKind;
    match action {
        ActionKind::Activate | ActionKind::ContextMenu => callback,
        ActionKind::Increment
        | ActionKind::Decrement
        | ActionKind::SetValue
        | ActionKind::Dismiss
        | ActionKind::Cancel
        | ActionKind::Scroll => RemoteActionDisposition::Guarded,
        ActionKind::Expand
        | ActionKind::Collapse
        | ActionKind::Select
        | ActionKind::EnterNavigation
        | ActionKind::ExitNavigation => RemoteActionDisposition::Unavailable,
    }
}

fn project<A: UiApplication>(
    host: &nickel_ui::UiHost<A>,
    activate: impl Fn(&A::Message) -> RemoteActionDisposition,
) -> Result<Projection, String> {
    let mut nodes = host
        .bounded_semantic_nodes(MAX_RESOLVED_NODES, MAX_PAYLOAD_BYTES)
        .map_err(|_| "shell semantics are protected or exceed budget")?;
    for node in &mut nodes {
        node.actions.retain(|action| {
            let callback = host
                .message_for_semantic_action(&node.id, *action)
                .map(&activate)
                .unwrap_or(RemoteActionDisposition::Unavailable);
            action_disposition(*action, callback) == RemoteActionDisposition::Guarded
        });
        node.enabled = !node.actions.is_empty();
    }
    Ok((host.resolved_frame_generation(), nodes))
}

fn observe_only(mut projection: Projection) -> Projection {
    for node in &mut projection.1 {
        node.actions.clear();
        node.enabled = false;
    }
    projection
}

fn plugin_projection(
    host: &nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>,
    allowed: impl Fn(&str, nickel_ui::ActionKind) -> bool,
) -> Result<Projection, String> {
    let mut nodes = host
        .bounded_semantic_nodes(MAX_RESOLVED_NODES, MAX_PAYLOAD_BYTES)
        .map_err(|_| "shell semantics are protected or exceed budget")?;
    for node in &mut nodes {
        let leaf = node.id.as_str().rsplit('/').next().unwrap_or_default();
        node.actions.retain(|action| {
            allowed(leaf, *action)
                && (*action == nickel_ui::ActionKind::SetValue
                    || host
                        .message_for_semantic_action(&node.id, *action)
                        .is_some())
        });
        node.enabled = !node.actions.is_empty();
    }
    Ok((host.resolved_frame_generation(), nodes))
}

fn control_activate(action: &ControlAction) -> RemoteActionDisposition {
    match action {
        ControlAction::ToggleWifiSection
        | ControlAction::SetWifiEnabled(_)
        | ControlAction::ActivateWifi { .. }
        | ControlAction::ToggleBluetoothSection
        | ControlAction::SetBluetoothPowered(_)
        | ControlAction::SetBluetoothDiscovery(_)
        | ControlAction::ToggleBluetoothDevice { .. }
        | ControlAction::ToggleAudioSection
        | ControlAction::SetAudioVolume(_)
        | ControlAction::SetAudioMuted(_)
        | ControlAction::SelectAudioDevice { .. }
        | ControlAction::RequestSessionAction(_)
        | ControlAction::CancelSessionAction => RemoteActionDisposition::Guarded,
        ControlAction::SwitchWorkspace(_)
        | ControlAction::WifiScroll
        | ControlAction::BluetoothScroll
        | ControlAction::AudioScroll
        | ControlAction::CreateWorkspace
        | ControlAction::ToggleShowDesktop
        | ControlAction::ShowNotifications
        | ControlAction::RemoveWorkspace(_)
        | ControlAction::PreviewProjection(_)
        | ControlAction::ConfirmProjection
        | ControlAction::CancelProjection
        | ControlAction::ConfirmSessionAction
        | ControlAction::SessionAction(_) => RemoteActionDisposition::Unavailable,
    }
}

fn panel_activate(action: &TaskbarAction) -> RemoteActionDisposition {
    match action {
        TaskbarAction::Launcher
        | TaskbarAction::ToggleTaskPin(_)
        | TaskbarAction::MoveTaskPinLeft(_)
        | TaskbarAction::MoveTaskPinRight(_)
        | TaskbarAction::Control => RemoteActionDisposition::Guarded,
        TaskbarAction::OnScreenKeyboard
        | TaskbarAction::Task(_)
        | TaskbarAction::TaskContext(_)
        | TaskbarAction::TaskDrag(_, _)
        | TaskbarAction::Codex
        | TaskbarAction::Tray(_)
        | TaskbarAction::TrayContext(_) => RemoteActionDisposition::Unavailable,
    }
}

impl LiveShell {
    pub(crate) fn bounded_shell_semantics(
        &self,
        role: SurfaceRole,
        output: Option<&str>,
    ) -> Result<Projection, String> {
        if self.locked || !self.surface_visible(role) || self.surface_remote_access_protected(role)
        {
            return Err("shell surface is hidden or protected".into());
        }
        match role {
            SurfaceRole::Desktop => {
                let output = output.ok_or("desktop output is unavailable")?;
                if output == self.desktop_active_viewport {
                    Ok(observe_only(project(&self.desktop_host, |_| {
                        RemoteActionDisposition::Unavailable
                    })?))
                } else {
                    self.desktop_viewports
                        .get(output)
                        .ok_or("desktop viewport is unavailable")?
                        .host
                        .bounded_semantics(MAX_RESOLVED_NODES, MAX_PAYLOAD_BYTES)
                        .map(observe_only)
                        .map_err(|_| "shell semantics are protected or exceed budget".into())
                }
            }
            SurfaceRole::Taskbar => {
                if self.plugin_taskbar_host.is_some() {
                    let host = if output == self.panel_output.as_deref() {
                        self.plugin_taskbar_host.as_ref()
                    } else {
                        self.plugin_taskbar_hosts.get(&output.map(str::to_owned))
                    }
                    .ok_or("panel plugin viewport is unavailable")?;
                    return plugin_projection(host, |leaf, action| {
                        matches!(action, nickel_ui::ActionKind::Activate)
                            && matches!(leaf, "taskbar-launcher" | "taskbar-control")
                    });
                }
                if output == self.panel_output.as_deref() {
                    project(&self.panel_host, panel_activate)
                } else {
                    let key = output.map(str::to_owned);
                    project(
                        self.panel_hosts
                            .get(&key)
                            .ok_or("panel viewport is unavailable")?,
                        panel_activate,
                    )
                }
            }
            SurfaceRole::Launcher if self.run_visible => {
                if let Some(host) = self.plugin_run_host.as_ref() {
                    plugin_projection(host, |leaf, action| {
                        leaf == "run-command" && action == nickel_ui::ActionKind::SetValue
                    })
                } else {
                    Err("Run plugin is unavailable".into())
                }
            }
            SurfaceRole::Launcher => {
                if let Some(host) = self.plugin_launcher_host.as_ref() {
                    plugin_projection(host, |leaf, action| {
                        leaf == "launcher-query" && action == nickel_ui::ActionKind::SetValue
                    })
                } else {
                    Err("Launcher plugin is unavailable".into())
                }
            }
            SurfaceRole::ControlCenter => {
                if self.control_plugin_active() {
                    Ok(observe_only(plugin_projection(
                        self.plugin_control_host.as_ref().unwrap(),
                        |_, _| false,
                    )?))
                } else if self.control_host.application().view_state().projection_only {
                    project(&self.control_host, control_activate)
                } else {
                    Err("Control Center plugin is unavailable".into())
                }
            }
            SurfaceRole::Notification => {
                if let Some(host) = self.plugin_notification_host.as_ref() {
                    Ok(observe_only(plugin_projection(host, |_, _| false)?))
                } else if self.trusted_notification_visible() {
                    Ok(observe_only(project(&self.notification_host, |_| {
                        RemoteActionDisposition::Unavailable
                    })?))
                } else {
                    Err("Notification plugin is unavailable".into())
                }
            }
            SurfaceRole::VolumeOsd => {
                if let Some(host) = self.plugin_volume_osd_host.as_ref() {
                    Ok(observe_only(plugin_projection(host, |_, _| false)?))
                } else {
                    Err("Volume overlay plugin is unavailable".into())
                }
            }
            SurfaceRole::WindowPreview => {
                if self.preview_plugin_active() {
                    Ok(observe_only(plugin_projection(
                        self.plugin_preview_host.as_ref().unwrap(),
                        |_, _| false,
                    )?))
                } else {
                    Err("Window preview plugin is unavailable".into())
                }
            }
            SurfaceRole::WindowContextMenu => {
                if let Some(host) = self.window_menu_plugin_host.as_ref() {
                    Ok(observe_only(plugin_projection(host, |_, _| false)?))
                } else if let Some(host) = self.window_menu_host.as_ref() {
                    Ok(observe_only(project(host, |_| {
                        RemoteActionDisposition::Unavailable
                    })?))
                } else if let Some(host) = self.application_menu_plugin_host.as_ref() {
                    Ok(observe_only(plugin_projection(host, |_, _| false)?))
                } else if let Some(host) = self.application_menu_host.as_ref() {
                    Ok(observe_only(project(host, |_| {
                        RemoteActionDisposition::Unavailable
                    })?))
                } else {
                    Err("window menu is unavailable".into())
                }
            }
            SurfaceRole::Screenshot => Ok(observe_only((
                self.screenshot.change_token().semantic_generation,
                self.screenshot
                    .bounded_semantics(MAX_RESOLVED_NODES, MAX_PAYLOAD_BYTES)
                    .map_err(|_| "shell semantics are protected or exceed budget")?,
            ))),
            SurfaceRole::OnScreenKeyboard => {
                Ok(observe_only(project(&self.keyboard_host, |_| {
                    RemoteActionDisposition::Unavailable
                })?))
            }
            _ => Err("shell semantics are unavailable for this role".into()),
        }
    }
}

// Windows projects semantic effects but does not consume their payloads;
// the Linux compositor owns their guarded application.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub(crate) enum RemoteShellEffect {
    Panel(TaskbarAction, Option<String>),
    Control(ControlAction),
}

pub(crate) struct RemoteShellOutcome {
    pub(crate) host: nickel_ui::HostEventOutcome,
    pub(crate) effects: Vec<RemoteShellEffect>,
}

fn mutate<A: UiApplication>(
    host: &mut nickel_ui::UiHost<A>,
    generation: u64,
    node: usize,
    action: nickel_ui::SemanticAction,
    clipboard_limit: usize,
) -> Result<nickel_ui::HostEventOutcome, String> {
    host.perform_bounded_semantic_action(
        generation,
        node,
        action,
        MAX_RESOLVED_NODES,
        MAX_PAYLOAD_BYTES,
        Some(clipboard_limit),
    )
    .map_err(|error| {
        match error {
            nickel_ui::BoundedSemanticActionError::StaleGeneration => "stale semantic tree",
            nickel_ui::BoundedSemanticActionError::InputBusy => "local surface input is held",
            nickel_ui::BoundedSemanticActionError::MissingTarget => "semantic target unavailable",
            nickel_ui::BoundedSemanticActionError::ActionUnavailable => {
                "semantic action unavailable"
            }
            nickel_ui::BoundedSemanticActionError::Snapshot(_) => {
                "semantic projection is protected or exceeds budget"
            }
        }
        .into()
    })
}

impl LiveShell {
    pub(crate) fn perform_bounded_shell_action(
        &mut self,
        role: SurfaceRole,
        output: Option<&str>,
        generation: u64,
        node: usize,
        action: nickel_ui::SemanticAction,
        clipboard_limit: usize,
    ) -> Result<RemoteShellOutcome, String> {
        if self.bounded_shell_semantics(role, output)?.0 != generation {
            return Err("stale semantic tree".into());
        }
        let kind = match &action {
            nickel_ui::SemanticAction::Invoke(kind) => *kind,
            nickel_ui::SemanticAction::SetValue(_) => nickel_ui::ActionKind::SetValue,
        };
        let projection = self.bounded_shell_semantics(role, output)?.1;
        if projection
            .get(node)
            .is_none_or(|target| !target.actions.contains(&kind))
        {
            return Err("semantic action has no guarded production disposition".into());
        }
        if self.pointer_interaction_active() {
            return Err("local surface input is held".into());
        }
        let mut effects = Vec::new();
        let host = match role {
            SurfaceRole::Launcher if self.run_visible => {
                let plugin = self
                    .plugin_run_host
                    .as_mut()
                    .ok_or("Run plugin is unavailable")?;
                let outcome = mutate(plugin, generation, node, action, clipboard_limit)?;
                if !plugin.application_mut().take_effects().is_empty() {
                    return Err("run plugin requested an unguarded effect".into());
                }
                outcome
            }
            SurfaceRole::Launcher if self.plugin_launcher_host.is_some() => {
                let plugin = self
                    .plugin_launcher_host
                    .as_mut()
                    .expect("launcher plugin exists");
                let outcome = mutate(plugin, generation, node, action, clipboard_limit)?;
                let requested = plugin.application_mut().take_effects();
                let [crate::plugin_panel::PluginEffect::SetLauncherQuery(query)] =
                    requested.as_slice()
                else {
                    return Err("launcher plugin requested an unguarded effect".into());
                };
                self.apply_launcher_action(LauncherAction::SetQuery(query.clone()));
                if self.sync_plugin_launcher()
                    && let Some(plugin) = self.plugin_launcher_host.as_mut()
                {
                    plugin.step(HostBatch {
                        application_changed: true,
                        ..HostBatch::default()
                    });
                }
                outcome
            }
            SurfaceRole::Launcher => return Err("Launcher plugin is unavailable".into()),
            SurfaceRole::ControlCenter => {
                if self.control_plugin_active() {
                    return Err("control center plugin actions require shell input".into());
                }
                if !self.control_host.application().view_state().projection_only {
                    return Err("Control Center plugin is unavailable".into());
                }
                let outcome = mutate(
                    &mut self.control_host,
                    generation,
                    node,
                    action,
                    clipboard_limit,
                )?;
                self.control_change_token = outcome.change_token;
                self.control_deadline = outcome.next_deadline;
                effects.extend(
                    self.control_host
                        .application_mut()
                        .take_effects()
                        .into_iter()
                        .map(RemoteShellEffect::Control),
                );
                outcome
            }
            SurfaceRole::Taskbar if self.plugin_taskbar_host.is_some() => {
                let previous = self.panel_output.clone();
                let token = self.panel_change_token;
                self.switch_panel_output(output.map(str::to_owned));
                let result: Result<_, String> = (|| {
                    let plugin = self
                        .plugin_taskbar_host
                        .as_mut()
                        .ok_or("panel plugin viewport is unavailable")?;
                    let outcome = mutate(plugin, generation, node, action, clipboard_limit)?;
                    let requested = plugin.application_mut().take_effects();
                    let panel_action = match requested.as_slice() {
                        [crate::plugin_panel::PluginEffect::ToggleLauncher] => {
                            TaskbarAction::Launcher
                        }
                        [crate::plugin_panel::PluginEffect::ToggleControlCenter] => {
                            TaskbarAction::Control
                        }
                        _ => return Err("taskbar plugin requested an unguarded effect".into()),
                    };
                    Ok((outcome, panel_action))
                })();
                self.switch_panel_output(previous);
                self.panel_change_token = token;
                let (outcome, panel_action) = result?;
                effects.push(RemoteShellEffect::Panel(
                    panel_action,
                    output.map(str::to_owned),
                ));
                outcome
            }
            SurfaceRole::Taskbar => {
                let previous = self.panel_output.clone();
                let token = self.panel_change_token;
                self.switch_panel_output(output.map(str::to_owned));
                let result = mutate(
                    &mut self.panel_host,
                    generation,
                    node,
                    action,
                    clipboard_limit,
                );
                if result.is_ok() {
                    effects.extend(
                        std::mem::take(&mut self.panel_host.application_mut().effects)
                            .into_iter()
                            .map(|action| {
                                RemoteShellEffect::Panel(action, output.map(str::to_owned))
                            }),
                    );
                }
                self.switch_panel_output(previous);
                self.panel_change_token = token;
                result?
            }
            SurfaceRole::VolumeOsd => {
                return Err("volume overlay has no remote actions".into());
            }
            // Desktop messages currently perform native file effects directly.
            // They require staging before the remote dispatcher can admit them.
            _ => return Err("semantic mutation effects are unavailable for this role".into()),
        };
        self.host_runtime_samples.record(host.telemetry);
        Ok(RemoteShellOutcome { host, effects })
    }
}

impl LiveShell {
    #[cfg(target_os = "linux")]
    pub(crate) fn resolve_remote_installed_launch(
        &mut self,
        effect: &RemoteShellEffect,
    ) -> Result<Option<Application>, String> {
        let selected = match effect {
            RemoteShellEffect::Panel(TaskbarAction::Task(index), output) => {
                let previous = self.panel_output.clone();
                let token = self.panel_change_token;
                self.switch_panel_output(output.clone());
                let groups = self.panel_groups();
                self.switch_panel_output(previous);
                self.panel_change_token = token;
                let group = groups.get(*index).ok_or("panel application has retired")?;
                if !group.windows.is_empty() {
                    return Ok(None);
                }
                Some(group.application_id.as_ref().and_then(|id| {
                    self.launcher
                        .applications()
                        .find(|app| app.id() == id.as_str())
                        .cloned()
                }))
            }
            _ => None,
        };
        match selected {
            Some(Some(application)) => Ok(Some(application)),
            Some(None) => Err("selected application is unavailable".into()),
            None => Ok(None),
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn stage_remote_shell_effect(
        &mut self,
        effect: RemoteShellEffect,
    ) -> Result<Vec<crate::platform::ShellCommand>, String> {
        let original = self.session_host.clone();
        let staged = Arc::new(crate::session_host::StagedSessionHost::new(
            original.clone(),
        ));
        self.session_host = staged.clone();
        let result: Result<(), String> = match effect {
            RemoteShellEffect::Panel(
                action @ (TaskbarAction::Launcher | TaskbarAction::Control),
                output,
            ) => {
                let previous = self.panel_output.clone();
                let token = self.panel_change_token;
                self.switch_panel_output(output);
                self.apply_panel_action(action);
                self.switch_panel_output(previous);
                self.panel_change_token = token;
                Ok(())
            }
            RemoteShellEffect::Control(
                ControlAction::ToggleWifiSection
                | ControlAction::ToggleBluetoothSection
                | ControlAction::ToggleAudioSection
                | ControlAction::CancelSessionAction
                | ControlAction::RequestSessionAction(_),
            ) => Ok(()),
            RemoteShellEffect::Panel(action, output) => {
                drop((action, output));
                Err("panel native effect requires guarded delivery".into())
            }
            RemoteShellEffect::Control(action) => {
                drop(action);
                Err("control native effect requires guarded delivery".into())
            }
        };
        self.session_host = original;
        result?;
        Ok(staged.take_commands())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_taskbar_remote_controls_use_the_jsx_tree_and_guarded_effects() {
        let mut shell = LiveShell::new().expect("live shell");
        let _ = shell.scene(SurfaceRole::Taskbar, 1280, 56);
        let (generation, nodes) = shell
            .bounded_shell_semantics(SurfaceRole::Taskbar, None)
            .expect("active taskbar semantics");
        let launcher = nodes
            .iter()
            .position(|node| node.id.as_str().ends_with("/taskbar-launcher"))
            .expect("JSX launcher button");
        assert_eq!(
            nodes[launcher].actions,
            vec![nickel_ui::ActionKind::Activate]
        );
        assert!(nodes.iter().any(|node| {
            node.id.as_str().ends_with("/taskbar-control")
                && node.actions == [nickel_ui::ActionKind::Activate]
        }));
        assert!(nodes.iter().all(|node| {
            !node.id.as_str().contains("/taskbar-item-") || node.actions.is_empty()
        }));
        let outcome = shell
            .perform_bounded_shell_action(
                SurfaceRole::Taskbar,
                None,
                generation,
                launcher,
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                2048,
            )
            .expect("guarded plugin action");
        assert!(matches!(
            outcome.effects.as_slice(),
            [RemoteShellEffect::Panel(TaskbarAction::Launcher, None)]
        ));
    }

    #[test]
    fn bundled_run_remote_text_mutation_uses_the_jsx_form() {
        let mut shell = LiveShell::new().expect("live shell");
        shell.run_visible = true;
        shell.launcher_visible = true;
        let _ = shell.scene(SurfaceRole::Launcher, 620, 180);
        let (generation, nodes) = shell
            .bounded_shell_semantics(SurfaceRole::Launcher, None)
            .expect("active Run semantics");
        let command = nodes
            .iter()
            .position(|node| node.id.as_str().ends_with("/run-command"))
            .expect("JSX Run field");
        assert_eq!(nodes[command].actions, [nickel_ui::ActionKind::SetValue]);
        let outcome = shell
            .perform_bounded_shell_action(
                SurfaceRole::Launcher,
                None,
                generation,
                command,
                nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Text(
                    "echo ready".into(),
                )),
                2048,
            )
            .expect("guarded Run input");
        assert!(outcome.effects.is_empty());
        let (_, updated) = shell
            .bounded_shell_semantics(SurfaceRole::Launcher, None)
            .unwrap();
        assert!(updated.iter().any(|node| {
            node.id.as_str().ends_with("/run-command")
                && node.value == Some(nickel_ui::SemanticValueSnapshot::Text("echo ready".into()))
        }));
    }

    fn assert_advertised_actions_are_guarded<A: UiApplication>(
        host: &nickel_ui::UiHost<A>,
        classify: impl Fn(&A::Message) -> RemoteActionDisposition,
    ) {
        let (_, nodes) = project(host, &classify).expect("bounded production semantics");
        for node in nodes {
            for action in node.actions {
                let callback = host
                    .message_for_semantic_action(&node.id, action)
                    .map(&classify)
                    .unwrap_or(RemoteActionDisposition::Unavailable);
                assert_eq!(
                    action_disposition(action, callback),
                    RemoteActionDisposition::Guarded,
                    "{:?} advertises {action:?} without a guarded callback",
                    node.id
                );
            }
        }
    }

    #[test]
    fn every_advertised_production_shell_action_has_a_dispatch_disposition() {
        let shell = LiveShell::new().expect("live shell");

        assert_advertised_actions_are_guarded(&shell.control_host, control_activate);
        assert_advertised_actions_are_guarded(&shell.panel_host, panel_activate);
    }

    #[test]
    fn volume_overlay_semantics_follow_the_active_jsx_host() {
        let mut shell = LiveShell::new().expect("live shell");
        shell.audio.volume_percent = 47;
        shell.audio.muted = false;
        shell.volume_osd_until =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
        let _ = shell.scene(SurfaceRole::VolumeOsd, 420, 96);
        let (_, nodes) = shell
            .bounded_shell_semantics(SurfaceRole::VolumeOsd, None)
            .expect("volume overlay semantics");
        assert!(nodes.iter().any(|node| node.name.as_deref() == Some("47%")));
        assert!(nodes.iter().any(|node| {
            node.name
                .as_deref()
                .is_some_and(|name| name.starts_with("Volume 47%"))
        }));
    }
}
