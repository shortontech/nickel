//! Read the production shell hosts without switching their active viewport.
use super::*;
use nickel_remote_control::semantics::{MAX_PAYLOAD_BYTES, MAX_RESOLVED_NODES};

type Projection = (u64, Vec<nickel_ui::SemanticNodeSnapshot>);

fn project<A: UiApplication>(host: &nickel_ui::UiHost<A>) -> Result<Projection, String> {
    Ok((
        host.resolved_frame_generation(),
        host.bounded_semantic_nodes(MAX_RESOLVED_NODES, MAX_PAYLOAD_BYTES)
            .map_err(|_| "shell semantics are protected or exceed budget")?,
    ))
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
                    project(&self.desktop_host)
                } else {
                    self.desktop_viewports
                        .get(output)
                        .ok_or("desktop viewport is unavailable")?
                        .host
                        .bounded_semantics(MAX_RESOLVED_NODES, MAX_PAYLOAD_BYTES)
                        .map_err(|_| "shell semantics are protected or exceed budget".into())
                }
            }
            SurfaceRole::Panel => {
                if output == self.panel_output.as_deref() {
                    project(&self.panel_host)
                } else {
                    let key = output.map(str::to_owned);
                    project(
                        self.panel_hosts
                            .get(&key)
                            .ok_or("panel viewport is unavailable")?,
                    )
                }
            }
            SurfaceRole::Launcher if self.run_visible => project(&self.run_host),
            SurfaceRole::Launcher => project(&self.launcher_host),
            SurfaceRole::ControlCenter => project(&self.control_host),
            SurfaceRole::VolumeOsd => project(&self.volume_osd_host),
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
