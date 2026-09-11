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

fn launcher_activate(action: &LauncherAction) -> RemoteActionDisposition {
    match action {
        LauncherAction::SetView(_)
        | LauncherAction::ActivateResult(_)
        | LauncherAction::TogglePin(_)
        | LauncherAction::LaunchApplication(_)
        | LauncherAction::ShowNarrowPrimary
        | LauncherAction::SetQuery(_)
        | LauncherAction::SearchScroll
        | LauncherAction::DashboardScroll
        | LauncherAction::Dismiss => RemoteActionDisposition::Guarded,
        LauncherAction::RetryPreferencePersistence
        | LauncherAction::OpenProject(_)
        | LauncherAction::SeeAllProjects
        | LauncherAction::OpenSettings(_)
        | LauncherAction::OpenAccount
        | LauncherAction::RequestLogout => RemoteActionDisposition::Unavailable,
    }
}

fn run_activate(action: &RunAction) -> RemoteActionDisposition {
    match action {
        RunAction::SetCommand(_) | RunAction::Dismiss => RemoteActionDisposition::Guarded,
        RunAction::Submit => RemoteActionDisposition::Unavailable,
    }
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
        | ControlAction::SelectAudioDevice { .. }
        | ControlAction::RequestSessionAction(_)
        | ControlAction::CancelSessionAction => RemoteActionDisposition::Guarded,
        ControlAction::SwitchWorkspace(_)
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

fn panel_activate(action: &PanelAction) -> RemoteActionDisposition {
    match action {
        PanelAction::Launcher
        | PanelAction::ToggleTaskPin(_)
        | PanelAction::MoveTaskPinLeft(_)
        | PanelAction::MoveTaskPinRight(_)
        | PanelAction::Control => RemoteActionDisposition::Guarded,
        PanelAction::OnScreenKeyboard
        | PanelAction::Task(_)
        | PanelAction::TaskContext(_)
        | PanelAction::TaskDrag(_, _)
        | PanelAction::Codex
        | PanelAction::Tray(_)
        | PanelAction::TrayContext(_) => RemoteActionDisposition::Unavailable,
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
            SurfaceRole::Panel => {
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
            SurfaceRole::Launcher if self.run_visible => project(&self.run_host, run_activate),
            SurfaceRole::Launcher => project(&self.launcher_host, launcher_activate),
            SurfaceRole::ControlCenter => project(&self.control_host, control_activate),
            SurfaceRole::Notification => {
                Ok(observe_only(project(&self.notification_host, |_| {
                    RemoteActionDisposition::Unavailable
                })?))
            }
            SurfaceRole::VolumeOsd => project(&self.volume_osd_host, |_| {
                RemoteActionDisposition::Unavailable
            }),
            SurfaceRole::WindowPreview => self
                .preview_frame
                .as_ref()
                .ok_or_else(|| "window preview is unavailable".to_owned())
                .and_then(|frame| {
                    Ok(observe_only((
                        frame.change_token().semantic_generation,
                        frame
                            .bounded_semantics(MAX_RESOLVED_NODES, MAX_PAYLOAD_BYTES)
                            .map_err(|_| {
                                "shell semantics are protected or exceed budget".to_owned()
                            })?,
                    )))
                }),
            SurfaceRole::WindowContextMenu => {
                if let Some(host) = self.window_menu_host.as_ref() {
                    Ok(observe_only(project(host, |_| {
                        RemoteActionDisposition::Unavailable
                    })?))
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

pub(crate) enum RemoteShellEffect {
    Launcher(LauncherShellEffect),
    Panel(PanelAction, Option<String>),
    Control(ControlAction),
    Run(String),
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
                let outcome = mutate(
                    &mut self.run_host,
                    generation,
                    node,
                    action,
                    clipboard_limit,
                )?;
                for effect in self.run_host.application_mut().take_effects() {
                    match effect {
                        RunEffect::Submit(command) => effects.push(RemoteShellEffect::Run(command)),
                        RunEffect::Dismiss => {
                            effects.push(RemoteShellEffect::Launcher(LauncherShellEffect::Dismiss))
                        }
                    }
                }
                outcome
            }
            SurfaceRole::Launcher => {
                let outcome = mutate(
                    &mut self.launcher_host,
                    generation,
                    node,
                    action,
                    clipboard_limit,
                )?;
                for action in self.launcher_host.application_mut().take_effects() {
                    if let Some(effect) =
                        reduce_launcher_action(&mut self.launcher, &mut self.launcher_view, action)
                    {
                        effects.push(RemoteShellEffect::Launcher(effect));
                    }
                }
                outcome
            }
            SurfaceRole::ControlCenter => {
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
            SurfaceRole::Panel => {
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
            SurfaceRole::VolumeOsd => mutate(
                &mut self.volume_osd_host,
                generation,
                node,
                action,
                clipboard_limit,
            )?,
            // Desktop messages currently perform native file effects directly.
            // They require staging before the remote dispatcher can admit them.
            _ => return Err("semantic mutation effects are unavailable for this role".into()),
        };
        self.host_runtime_samples.record(host.telemetry);
        Ok(RemoteShellOutcome { host, effects })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        assert_advertised_actions_are_guarded(&shell.launcher_host, launcher_activate);
        assert_advertised_actions_are_guarded(&shell.run_host, run_activate);
        assert_advertised_actions_are_guarded(&shell.control_host, control_activate);
        assert_advertised_actions_are_guarded(&shell.panel_host, panel_activate);
        assert_advertised_actions_are_guarded(&shell.volume_osd_host, |_| {
            RemoteActionDisposition::Unavailable
        });

        let submit = shell
            .run_host
            .unique_semantic_target_for_message(&RunAction::Submit)
            .expect("run submit target");
        let (_, projected) = project(&shell.run_host, run_activate).expect("run semantics");
        let submit = projected
            .iter()
            .find(|node| node.id == submit.id)
            .expect("projected submit node");
        assert!(!submit.actions.contains(&nickel_ui::ActionKind::Activate));
        assert!(!submit.enabled);
    }
}

impl LiveShell {
    pub(crate) fn resolve_remote_installed_launch(
        &mut self,
        effect: &RemoteShellEffect,
    ) -> Result<Option<Application>, String> {
        let selected = match effect {
            RemoteShellEffect::Launcher(LauncherShellEffect::ActivateResult(index)) => {
                Some(self.launcher.result_at(*index).cloned())
            }
            RemoteShellEffect::Launcher(LauncherShellEffect::LaunchApplication(id)) => Some(
                self.launcher
                    .applications()
                    .find(|app| app.id() == id)
                    .cloned(),
            ),
            RemoteShellEffect::Panel(PanelAction::Task(index), output) => {
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
            RemoteShellEffect::Launcher(LauncherShellEffect::Dismiss) => {
                self.apply_launcher_effect(LauncherShellEffect::Dismiss);
                Ok(())
            }
            RemoteShellEffect::Panel(
                action @ (PanelAction::Launcher | PanelAction::Control),
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
            RemoteShellEffect::Launcher(effect) => {
                drop(effect);
                Err("launcher native effect requires guarded delivery".into())
            }
            RemoteShellEffect::Panel(action, output) => {
                drop((action, output));
                Err("panel native effect requires guarded delivery".into())
            }
            RemoteShellEffect::Control(action) => {
                drop(action);
                Err("control native effect requires guarded delivery".into())
            }
            RemoteShellEffect::Run(command) => {
                drop(command);
                Err("command launch requires guarded delivery".into())
            }
        };
        self.session_host = original;
        result?;
        Ok(staged.take_commands())
    }
}
