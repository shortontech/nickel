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
        ControlAction::PreviewProjection(_)
        | ControlAction::ConfirmProjection
        | ControlAction::CancelProjection => RemoteActionDisposition::Unavailable,
        _ => RemoteActionDisposition::Guarded,
    }
}

impl LiveShell {
    pub(crate) fn bounded_plugin_panel_semantics(
        &self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        _output: Option<&str>,
    ) -> Result<Projection, String> {
        let host = self
            .plugin_panel_host_ref(key)
            .ok_or("plugin surface is unavailable")?;
        if self.locked || host.remote_access_protected() {
            return Err("plugin surface is protected".into());
        }
        Ok(observe_only(plugin_projection(host, |_, _| false)?))
    }

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
            SurfaceRole::Taskbar => Err("historical native taskbar surface is unavailable".into()),

            SurfaceRole::Launcher => {
                Err("historical native launcher surface is unavailable".into())
            }
            SurfaceRole::ControlCenter => {
                if self.quick_settings_surface_active() {
                    Ok(observe_only(plugin_projection(
                        self.plugin_panel_host_ref(
                            &self.active_shell_surface_key("quick-settings"),
                        )
                        .unwrap(),
                        |_, _| false,
                    )?))
                } else if self.control_host.application().view_state().projection_only {
                    project(&self.control_host, control_activate)
                } else {
                    Err("Control Center plugin is unavailable".into())
                }
            }
            SurfaceRole::Notification => {
                if self.trusted_notification_visible() {
                    Ok(observe_only(project(&self.notification_host, |_| {
                        RemoteActionDisposition::Unavailable
                    })?))
                } else {
                    Err("Trusted notification is unavailable".into())
                }
            }
            SurfaceRole::VolumeOsd => Err("Retired native volume surface is unavailable".into()),
            SurfaceRole::WindowPreview => {
                if self.preview_plugin_active() {
                    Ok(observe_only(plugin_projection(
                        self.preview_plugin_host_ref().unwrap(),
                        |_, _| false,
                    )?))
                } else {
                    Err("Window preview plugin is unavailable".into())
                }
            }
            SurfaceRole::WindowContextMenu => Err("Legacy menu surface is unavailable".into()),
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
    pub(crate) fn perform_bounded_plugin_panel_action(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        output: Option<&str>,
        generation: u64,
        node: usize,
        action: nickel_ui::SemanticAction,
        clipboard_limit: usize,
    ) -> Result<RemoteShellOutcome, String> {
        let _ = (key, output, generation, node, action, clipboard_limit);
        Err("plugin semantic mutation is unavailable".into())
    }

    pub(crate) fn perform_bounded_shell_action(
        &mut self,
        role: SurfaceRole,
        output: Option<&str>,
        generation: u64,
        node: usize,
        action: nickel_ui::SemanticAction,
        clipboard_limit: usize,
    ) -> Result<RemoteShellOutcome, String> {
        if role == SurfaceRole::Taskbar {
            return Err("historical native taskbar surface is unavailable".into());
        }
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
            SurfaceRole::Launcher => return Err("Launcher plugin is unavailable".into()),
            SurfaceRole::ControlCenter => {
                if self.quick_settings_surface_active() {
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
            SurfaceRole::Taskbar => unreachable!("taskbar actions use the plugin surface key"),
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
        _effect: &RemoteShellEffect,
    ) -> Result<Option<Application>, String> {
        let selected = None;
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
        with_package_runtime_stack(|| {
            let shell = LiveShell::new().expect("live shell");

            assert_advertised_actions_are_guarded(&shell.control_host, control_activate);
        });
    }
}
