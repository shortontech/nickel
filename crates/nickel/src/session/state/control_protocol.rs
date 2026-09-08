use super::*;
use crate::session::SessionAuthorityRequest;

static CONTROL_SOCKET_GENERATION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

impl NickelSession {
    pub(super) fn init_control_socket(
        event_loop: &mut EventLoop<'static, NickelSession>,
    ) -> PathBuf {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let generation = CONTROL_SOCKET_GENERATION.fetch_add(1, Ordering::Relaxed);
        let path = runtime.join(format!(
            "nickel-session-{}-{generation}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let socket =
            UnixDatagram::bind(&path).expect("failed to bind Nickel session control socket");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("failed to restrict Nickel session control socket permissions");
        socket
            .set_nonblocking(true)
            .expect("failed to make Nickel session control socket nonblocking");
        nix::sys::socket::setsockopt(&socket, nix::sys::socket::sockopt::PassCred, &true)
            .expect("failed to enable Nickel session peer credentials");

        // SAFETY: session initialization is single-threaded and happens before
        // the shell child is spawned.
        unsafe { std::env::set_var("NICKEL_SESSION_CONTROL", &path) };

        event_loop
            .handle()
            .insert_source(
                Generic::new(socket, Interest::READ, Mode::Level),
                |_, socket, data| {
                    let mut frame = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
                    while let Ok((length, source, peer_pid)) =
                        recv_control_frame(socket.as_ref(), &mut frame)
                    {
                        let request = decode::<ClientEnvelope>(&frame[..length]);
                        let (request_id, message, request_redraw) = match request {
                            Ok(envelope) => {
                                let request_id = envelope.request_id;
                                let request_redraw = matches!(
                                    &envelope.request,
                                    Request::RegisterShell { .. } | Request::Command(_)
                                );
                                let message = data.handle_protocol_request(
                                    envelope,
                                    source.as_deref(),
                                    peer_pid,
                                );
                                (request_id, message, request_redraw)
                            }
                            Err(error) => (
                                0,
                                ServerMessage::Error {
                                    code: ErrorCode::IncompatibleVersion,
                                    message: error.to_string(),
                                },
                                false,
                            ),
                        };
                        if let Some(path) = source.as_deref() {
                            let is_preview = matches!(&message, ServerMessage::Preview(_));
                            match encode(&ServerEnvelope {
                                request_id,
                                message,
                            }) {
                                Ok(response) => {
                                    if is_preview {
                                        data.record_preview_protocol_encoding(
                                            response.len().saturating_sub(
                                                nickel_session_protocol::FRAME_HEADER_BYTES,
                                            ),
                                            response.len(),
                                        );
                                    }
                                    if let Err(error) = socket.as_ref().send_to(&response, path) {
                                        tracing::warn!(
                                            ?error,
                                            ?path,
                                            "failed to reply on session control socket"
                                        );
                                    }
                                }
                                Err(error) => tracing::warn!(
                                    ?error,
                                    request_id,
                                    "failed to encode session control response"
                                ),
                            }
                        }
                        data.windows.finish_snapshot();
                        if request_redraw {
                            data.request_output_redraw();
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .expect("failed to register Nickel session control socket");
        path
    }

    pub(super) fn handle_protocol_request(
        &mut self,
        envelope: ClientEnvelope,
        source: Option<&std::path::Path>,
        peer_pid: u32,
    ) -> ServerMessage {
        let request_id = envelope.request_id;
        let Some(control) = self.compatibility_control.as_ref() else {
            return protocol_error(ErrorCode::Unauthorized, "external control is disabled");
        };
        if envelope.token != control.protocol_token {
            return protocol_error(ErrorCode::Unauthorized, "invalid session capability");
        }
        match envelope.request {
            Request::RegisterShell { pid } => {
                let expected_pid = control.expected_shell_pid;
                if let Some(rejection) = shell_registration_rejection(
                    expected_pid,
                    pid,
                    peer_pid,
                    same_session_user(pid),
                ) {
                    tracing::warn!(
                        operation = "register-shell",
                        correlation_id = request_id,
                        claimed_pid = pid,
                        peer_pid,
                        expected_pid,
                        rejection_category = ?rejection,
                        "shell registration rejected"
                    );
                    return protocol_error(
                        ErrorCode::Unauthorized,
                        "shell process is outside the active user session",
                    );
                }
                let already_authenticated = control.authenticated_shell_pids.contains(&pid);
                if !already_authenticated {
                    self.retire_shell_surface_roles();
                }
                let control = self
                    .compatibility_control
                    .as_mut()
                    .expect("control adapter exists while servicing its socket");
                control.authenticated_shell_pids.clear();
                control.authenticated_shell_pids.insert(pid);
                self.shell_surface_identities.clear();
                ServerMessage::Snapshot(self.protocol_snapshot())
            }
            Request::Subscribe => {
                let Some(path) = source else {
                    return protocol_error(ErrorCode::InvalidRequest, "subscriber has no path");
                };
                if !self.launcher_subscribers.iter().any(|entry| entry == path) {
                    if self.launcher_subscribers.len() >= nickel_session_protocol::MAX_SUBSCRIBERS {
                        return protocol_error(
                            ErrorCode::ResourceLimit,
                            "subscriber limit reached",
                        );
                    }
                    self.launcher_subscribers.push(path.to_path_buf());
                }
                ServerMessage::Event(SessionEvent::Snapshot(self.protocol_snapshot()))
            }
            Request::Query(query) => self.handle_authority_request(query.into()),
            Request::Command(command) => {
                if command_requires_shell_identity(&command)
                    && !control.authenticated_shell_pids.contains(&peer_pid)
                    && !(self.test_control_enabled && test_control_may_invoke(&command))
                {
                    return protocol_error(
                        ErrorCode::Unauthorized,
                        "command requires the authenticated Nickel shell process",
                    );
                }
                // Keep reply routing for the one asynchronous transport
                // command. All other behavior shares the same typed dispatch
                // used by trusted in-process components.
                if matches!(command, SessionCommand::CaptureOutput { .. }) {
                    self.handle_protocol_command(command, source, request_id)
                } else {
                    self.handle_authority_request(command.into())
                }
            }
        }
    }

    /// Dispatch a request from code hosted by Nickel itself.
    ///
    /// Unlike `handle_protocol_request`, this intentionally performs no
    /// PID/token authentication: exclusive mutable access to the compositor
    /// state is the capability. External callers must continue through the
    /// authenticated protocol path above.
    pub(crate) fn handle_authority_request(
        &mut self,
        request: SessionAuthorityRequest,
    ) -> ServerMessage {
        match request {
            SessionAuthorityRequest::Query(query) => self.handle_protocol_query(query),
            SessionAuthorityRequest::Command(command) => {
                self.handle_protocol_command(command, None, 0)
            }
        }
    }

    pub(super) fn handle_protocol_query(&mut self, query: Query) -> ServerMessage {
        match query {
            Query::OnScreenKeyboard => {
                ServerMessage::OnScreenKeyboard(self.on_screen_keyboard_snapshot())
            }
            Query::Snapshot => ServerMessage::Snapshot(self.protocol_snapshot()),
            Query::Windows => ServerMessage::Windows(
                self.protocol_windows()
                    .into_iter()
                    .filter(|window| window.workspace.0 == self.workspaces.active().0)
                    .collect(),
            ),
            Query::Outputs => ServerMessage::Outputs(self.protocol_outputs()),
            Query::ShellSurfaces => ServerMessage::ShellSurfaces(self.protocol_shell_surfaces()),
            Query::ShellReadiness => ServerMessage::ShellReadiness(self.protocol_shell_readiness()),
            Query::LauncherVisibility => ServerMessage::LauncherVisibility {
                visible: self.launcher_visibility.is_visible(),
            },
            Query::SecureStorage => ServerMessage::SecureStorage {
                state: self.protocol_secure_storage_state(),
                reason: (self.secure_storage_state()
                    == crate::session::login_services::SecureStorageState::Unavailable)
                    .then(crate::session::login_services::secure_storage_unavailable_reason)
                    .flatten(),
            },
            Query::IdleInhibition => ServerMessage::IdleInhibition {
                surfaces: {
                    self.prune_dead_idle_inhibitors();
                    u16::try_from(self.idle_inhibitors.len()).unwrap_or(u16::MAX)
                },
            },
            Query::CacheDiagnostics => {
                let metadata = self.windows.metadata_diagnostics();
                let titlebar = crate::session::window_frame::titlebar_cache_diagnostics();
                let recovery = self.recovery_ui.raster_diagnostics();
                let internal_ui = self.internal_ui.aggregate_renderer_diagnostics();
                let shell_images = self
                    .internal_shell
                    .as_ref()
                    .map(crate::internal_shell::InternalShellCoordinator::image_cache_diagnostics)
                    .unwrap_or_default();
                #[cfg(feature = "backend-udev")]
                let identify = self
                    .native
                    .as_ref()
                    .map(crate::session::backend::udev::UdevData::identify_badge_diagnostics)
                    .unwrap_or_default();
                ServerMessage::CacheDiagnostics(Box::new(
                    nickel_session_protocol::CacheDiagnostics {
                        internal_ui_surfaces: u16::try_from(internal_ui.surfaces)
                            .unwrap_or(u16::MAX),
                        internal_ui_gpu_frames: internal_ui.gpu_frames,
                        internal_ui_fallback_frames: internal_ui.fallback_frames,
                        internal_ui_software_frame_bytes: internal_ui.software_frame_bytes as u64,
                        internal_ui_fallback_raster_bytes: internal_ui.fallback_raster_bytes as u64,
                        internal_ui_text_scratch_bytes: internal_ui.text_scratch_bytes as u64,
                        internal_ui_text_private_cache_bytes: internal_ui.text_private_cache_bytes
                            as u64,
                        internal_ui_fallback_buffer_creations: internal_ui
                            .fallback_buffer_creations,
                        internal_ui_fallback_buffer_reuses: internal_ui.fallback_buffer_reuses,
                        internal_ui_fallback_converted_bytes: internal_ui.fallback_converted_bytes,
                        internal_ui_fallback_upload_damage_bytes: internal_ui
                            .fallback_upload_damage_bytes,
                        internal_ui_fallback_full_repaints: internal_ui.fallback_full_repaints,
                        internal_ui_fallback_partial_repaints: internal_ui
                            .fallback_partial_repaints,
                        internal_ui_image_cache_entries: u16::try_from(
                            internal_ui.image_cache_entries,
                        )
                        .unwrap_or(u16::MAX),
                        internal_ui_image_cache_bytes: internal_ui.image_cache_bytes as u64,
                        internal_ui_text_cache_entries: u16::try_from(
                            internal_ui.text_cache_entries,
                        )
                        .unwrap_or(u16::MAX),
                        internal_ui_text_cache_bytes: internal_ui.text_cache_bytes as u64,
                        internal_ui_texture_import_failures: internal_ui.texture_import_failures,
                        internal_ui_fallback_import_failures: internal_ui.fallback_import_failures,
                        internal_shell_wallpaper_entries: u16::try_from(
                            shell_images.wallpaper_entries,
                        )
                        .unwrap_or(u16::MAX),
                        internal_shell_wallpaper_bytes: shell_images.wallpaper_bytes as u64,
                        preview_entries: u16::try_from(self.preview_frames.len())
                            .unwrap_or(u16::MAX),
                        native_preview_work: {
                            #[cfg(feature = "backend-udev")]
                            {
                                self.native
                                    .as_ref()
                                    .map_or_else(Default::default, |native| {
                                        native.native_preview_diagnostics()
                                    })
                            }
                            #[cfg(not(feature = "backend-udev"))]
                            {
                                Default::default()
                            }
                        },
                        preview_capacity: u16::try_from(PREVIEW_ENTRY_CAPACITY).unwrap_or(u16::MAX),
                        preview_bytes: self.preview_bytes() as u64,
                        preview_byte_capacity: PREVIEW_BYTE_CAPACITY as u64,
                        preview_peak_bytes: self.preview_counters.peak_bytes,
                        preview_admissions: self.preview_counters.admissions,
                        preview_evictions: self.preview_counters.evictions,
                        preview_invalidations: self.preview_counters.invalidations,
                        preview_captures: self.preview_counters.captures,
                        preview_skipped_unchanged: self.preview_counters.skipped_unchanged,
                        preview_readback_bytes: self.preview_counters.readback_bytes,
                        preview_protocol_copy_bytes: self.preview_counters.protocol_copy_bytes,
                        preview_protocol_raw_copy_bytes: self
                            .preview_counters
                            .protocol_raw_copy_bytes,
                        preview_protocol_base64_bytes: self.preview_counters.protocol_base64_bytes,
                        preview_protocol_json_payload_bytes: self
                            .preview_counters
                            .protocol_json_payload_bytes,
                        preview_protocol_framed_copy_bytes: self
                            .preview_counters
                            .protocol_framed_copy_bytes,
                        preview_capture_failures: self.preview_counters.capture_failures,
                        preview_cache_generation: self.preview_counters.presentation_generation,
                        metadata_entries: u16::try_from(metadata.entries).unwrap_or(u16::MAX),
                        metadata_title_bytes: metadata.title_bytes as u64,
                        metadata_peak_title_bytes: metadata.peak_title_bytes as u64,
                        metadata_app_id_bytes: metadata.app_id_bytes as u64,
                        metadata_peak_app_id_bytes: metadata.peak_app_id_bytes as u64,
                        metadata_truncations: metadata.truncations,
                        metadata_canonicalizations: metadata.canonicalizations,
                        metadata_updates: metadata.updates,
                        metadata_live_snapshot_bytes: metadata.live_snapshot_bytes as u64,
                        metadata_peak_snapshot_bytes: metadata.peak_snapshot_bytes as u64,
                        titlebar_entries: u16::try_from(titlebar.entries).unwrap_or(u16::MAX),
                        titlebar_live_bytes: titlebar.live_bytes as u64,
                        titlebar_peak_bytes: titlebar.peak_bytes as u64,
                        titlebar_hits: titlebar.hits,
                        titlebar_misses: titlebar.misses,
                        titlebar_rasterizations: titlebar.rasterizations,
                        titlebar_avoided_rasterizations: titlebar.avoided_rasterizations,
                        titlebar_evictions: titlebar.evictions,
                        titlebar_generation: titlebar.generation,
                        titlebar_font_database_loads: titlebar.font_database_loads,
                        titlebar_renderer_bytes: titlebar.renderer_bytes.map(|bytes| bytes as u64),
                        recovery_entries: u16::try_from(recovery.entries).unwrap_or(u16::MAX),
                        recovery_live_bytes: recovery.live_bytes as u64,
                        recovery_peak_bytes: recovery.peak_bytes as u64,
                        recovery_rasterizations: recovery.rasterizations,
                        recovery_avoided_rasterizations: recovery.avoided_rasterizations,
                        recovery_evictions: recovery.evictions,
                        recovery_generation: recovery.generation,
                        recovery_renderer_bytes: recovery.renderer_bytes.map(|bytes| bytes as u64),
                        #[cfg(feature = "backend-udev")]
                        identify_entries: u16::try_from(identify.entries).unwrap_or(u16::MAX),
                        #[cfg(not(feature = "backend-udev"))]
                        identify_entries: 0,
                        #[cfg(feature = "backend-udev")]
                        identify_live_bytes: identify.live_bytes as u64,
                        #[cfg(not(feature = "backend-udev"))]
                        identify_live_bytes: 0,
                        #[cfg(feature = "backend-udev")]
                        identify_peak_bytes: identify.peak_bytes as u64,
                        #[cfg(not(feature = "backend-udev"))]
                        identify_peak_bytes: 0,
                        #[cfg(feature = "backend-udev")]
                        identify_rasterizations: identify.rasterizations,
                        #[cfg(not(feature = "backend-udev"))]
                        identify_rasterizations: 0,
                        #[cfg(feature = "backend-udev")]
                        identify_avoided_rasterizations: identify.avoided_rasterizations,
                        #[cfg(not(feature = "backend-udev"))]
                        identify_avoided_rasterizations: 0,
                        #[cfg(feature = "backend-udev")]
                        identify_evictions: identify.evictions,
                        #[cfg(not(feature = "backend-udev"))]
                        identify_evictions: 0,
                        #[cfg(feature = "backend-udev")]
                        identify_renderer_bytes: identify.renderer_bytes.map(|bytes| bytes as u64),
                        #[cfg(not(feature = "backend-udev"))]
                        identify_renderer_bytes: None,
                    },
                ))
            }
            Query::Workspaces => ServerMessage::Workspaces(self.protocol_workspaces()),
            Query::ShellBehavior => {
                let _ = self.refresh_output_topology_generation();
                ServerMessage::ShellBehavior(self.protocol_shell_behavior())
            }
            Query::Preview { window } => {
                let id = WindowId(window.0);
                if !self.windows.snapshot().iter().any(|entry| entry.id == id) {
                    return protocol_error(ErrorCode::InvalidWindow, "unknown window id");
                }
                let Some(preview) =
                    protocol_preview_from_cached(window, self.preview_frames.get(&id))
                else {
                    return protocol_error(ErrorCode::InvalidRequest, "preview is not ready");
                };
                self.preview_counters.protocol_copy_bytes += preview.rgba.len() as u64;
                self.preview_counters.protocol_raw_copy_bytes += preview.rgba.len() as u64;
                let base64_bytes = 4 * preview.rgba.len().div_ceil(3);
                self.preview_counters.protocol_base64_bytes += base64_bytes as u64;
                self.preview_counters.protocol_copy_bytes += base64_bytes as u64;
                if preview.validate().is_err() {
                    return protocol_error(ErrorCode::ResourceLimit, "preview exceeds bounds");
                }
                ServerMessage::Preview(preview)
            }
            Query::ShellSemanticTarget { .. } | Query::ShellRuntimeDiagnostics => protocol_error(
                ErrorCode::InvalidRequest,
                "shell-only queries are resolved by the nested shell test endpoint",
            ),
        }
    }

    pub(super) fn handle_protocol_command(
        &mut self,
        command: SessionCommand,
        source: Option<&std::path::Path>,
        request_id: u64,
    ) -> ServerMessage {
        match command {
            SessionCommand::ObservePendingLaunch {
                generation,
                root_pid,
                root_start_time,
                deadline_ms,
            } => {
                if root_pid == 0
                    || root_start_time == 0
                    || linux_process_start_time(root_pid) != Some(root_start_time)
                    || deadline_ms == 0
                    || deadline_ms > 2_000
                {
                    return protocol_error(
                        ErrorCode::InvalidRequest,
                        "invalid pending-launch observation",
                    );
                }
                if self
                    .pending_launch_observations
                    .iter()
                    .any(|pending| pending.generation == generation)
                {
                    return protocol_error(
                        ErrorCode::InvalidRequest,
                        "pending-launch generation is already active",
                    );
                }
                if self.pending_launch_observations.len()
                    >= nickel_session_protocol::MAX_PENDING_LAUNCHES
                {
                    return protocol_error(
                        ErrorCode::ResourceLimit,
                        "pending-launch observation limit reached",
                    );
                }
                self.pending_launch_observations
                    .push(PendingLaunchObservation {
                        generation,
                        root_pid,
                        root_start_time,
                        registered_at: Instant::now(),
                        deadline: Duration::from_millis(u64::from(deadline_ms)),
                    });
                let timer = Timer::from_duration(Duration::from_millis(u64::from(deadline_ms)));
                if self
                    .event_loop_handle
                    .insert_source(timer, move |_, _, data| {
                        let was_active = data
                            .pending_launch_observations
                            .iter()
                            .any(|pending| pending.generation == generation);
                        data.pending_launch_observations
                            .retain(|pending| pending.generation != generation);
                        if was_active {
                            data.notify_pending_launch_expired(generation);
                        }
                        TimeoutAction::Drop
                    })
                    .is_err()
                {
                    self.pending_launch_observations
                        .retain(|pending| pending.generation != generation);
                    return protocol_error(
                        ErrorCode::Internal,
                        "pending-launch expiry could not be scheduled",
                    );
                }
            }
            SessionCommand::CancelPendingLaunch { generation } => {
                self.pending_launch_observations
                    .retain(|pending| pending.generation != generation);
            }
            SessionCommand::RegisterShellSurface { mut identity } => {
                let output_scoped = matches!(
                    identity.role,
                    ShellRole::Desktop | ShellRole::Panel | ShellRole::Lock
                );
                if !identity
                    .application_id
                    .starts_with(nickel_session_protocol::SHELL_SURFACE_APPLICATION_ID_PREFIX)
                    || identity.application_id.len()
                        > nickel_session_protocol::MAX_WINDOW_APP_ID_BYTES
                    || identity.output.is_some() != output_scoped
                {
                    return protocol_error(
                        ErrorCode::InvalidRequest,
                        "invalid shell surface identity",
                    );
                }
                if let Some(output) = identity.output.as_deref() {
                    let outputs = self.space.outputs().map(Output::name).collect::<Vec<_>>();
                    let Some(index) = output_index_for_shell_surface(output, &outputs) else {
                        return protocol_error(ErrorCode::InvalidRequest, "unknown shell output");
                    };
                    identity.output = Some(outputs[index].clone());
                }
                if self.shell_surface_identities.len() >= nickel_session_protocol::MAX_WINDOWS
                    && !self
                        .shell_surface_identities
                        .contains_key(&identity.application_id)
                {
                    return protocol_error(ErrorCode::ResourceLimit, "shell surface limit reached");
                }
                self.shell_surface_identities
                    .insert(identity.application_id.clone(), identity);
            }
            SessionCommand::RequestOnScreenKeyboard => self.request_on_screen_keyboard(),
            SessionCommand::ConfigureOnScreenKeyboard {
                height,
                dock_top,
                enabled,
                visible,
                generation,
                environment_override,
            } => {
                self.configure_on_screen_keyboard(
                    enabled,
                    visible,
                    generation,
                    environment_override,
                    dock_top,
                    height,
                );
            }
            SessionCommand::OnScreenKeyboardInput { epoch, input } => {
                if let Err(message) = self.deliver_on_screen_keyboard_input(epoch, input) {
                    return protocol_error(ErrorCode::InvalidRequest, message);
                }
            }
            SessionCommand::ReloadShellSettings => {
                self.apply_configured_workspace_count();
                self.notify_shell_settings_changed();
            }
            SessionCommand::ApplyShellBehavior { transaction } => {
                return self.apply_shell_behavior_transaction(transaction);
            }
            SessionCommand::ToggleLauncher => self.toggle_launcher(),
            SessionCommand::SetLauncherVisible { visible } => self.set_launcher_visible(visible),
            SessionCommand::SetLauncherVisibleFromController { visible } => {
                self.set_launcher_visible_from(visible, InvocationSource::Controller)
            }
            SessionCommand::SetShellRoleVisible { role, visible } => {
                self.set_shell_role_visible(role, visible);
            }
            SessionCommand::ShowAnchoredShellRole { role, anchor } => {
                self.show_anchored_shell_role(role, anchor);
            }
            SessionCommand::LogOut => self.loop_signal.stop(),
            SessionCommand::SessionAction { action } => match action {
                nickel_session_protocol::SessionAction::RestartShell => {
                    return protocol_error(
                        ErrorCode::InvalidRequest,
                        "the compositor-owned shell cannot be restarted independently",
                    );
                }
                nickel_session_protocol::SessionAction::Lock => {
                    self.lock_session();
                }
                nickel_session_protocol::SessionAction::Suspend => {
                    crate::session::session_services::request(
                        crate::session::session_services::SystemAction::Suspend,
                    );
                }
                nickel_session_protocol::SessionAction::Reboot => {
                    crate::session::session_services::request(
                        crate::session::session_services::SystemAction::Reboot,
                    );
                }
                nickel_session_protocol::SessionAction::PowerOff => {
                    crate::session::session_services::request(
                        crate::session::session_services::SystemAction::PowerOff,
                    );
                }
            },
            SessionCommand::Unlock => self.unlock_session(),
            SessionCommand::RetrySecureStorage => self
                .secure_storage_retry
                .store(true, std::sync::atomic::Ordering::Release),
            SessionCommand::HideOverlay => self.hide_overlays(),
            SessionCommand::ShowOverlay {
                role,
                geometry,
                windows,
            } => match role {
                ShellRole::ContextMenu => {
                    if !windows.is_empty() {
                        return protocol_error(
                            ErrorCode::InvalidRequest,
                            "context menu does not accept preview windows",
                        );
                    }
                    self.show_context_menu(
                        geometry.x,
                        geometry.y,
                        geometry.width,
                        geometry.height,
                        true,
                    )
                }
                ShellRole::Preview => {
                    if windows.iter().any(|window| !self.window_exists(*window)) {
                        return protocol_error(ErrorCode::InvalidWindow, "unknown preview window");
                    }
                    self.set_overlay_preview_interest(
                        windows.iter().map(|window| WindowId(window.0)).collect(),
                    );
                    self.show_preview(geometry.x, geometry.y, geometry.width, geometry.height)
                }
                _ => {
                    return protocol_error(
                        ErrorCode::InvalidRequest,
                        "role is not a transient overlay",
                    );
                }
            },
            SessionCommand::FocusShellRole { role } => {
                if !shell_role_accepts_ordinary_focus(role) {
                    return protocol_error(
                        ErrorCode::InvalidRequest,
                        "role does not accept ordinary shell focus",
                    );
                }
                self.focus_shell_role(role);
            }
            SessionCommand::RestoreApplicationFocus => self.restore_application_focus(),
            SessionCommand::IdentifyOutputs => self.begin_output_identification(),
            SessionCommand::CaptureOutput { path, output } => {
                if path.is_empty() {
                    return protocol_error(ErrorCode::InvalidRequest, "capture path is empty");
                }
                if let Some(name) = output.as_deref()
                    && !self
                        .protocol_outputs()
                        .iter()
                        .any(|candidate| candidate.enabled && candidate.name == name)
                {
                    return protocol_error(
                        ErrorCode::InvalidRequest,
                        "capture output was not found",
                    );
                }
                self.output_capture_path = Some(PathBuf::from(path));
                self.output_capture_name = output;
                self.output_capture_reply_path = source.map(PathBuf::from);
                self.output_capture_request_id = Some(request_id);
            }
            SessionCommand::ApplyOutputs { layout } => {
                if let Err(error) = self.apply_output_layout(layout) {
                    return protocol_error(ErrorCode::InvalidRequest, error);
                }
            }
            SessionCommand::CreateWorkspace => {
                if let Err(error) = self.workspaces.create() {
                    return protocol_error(ErrorCode::ResourceLimit, workspace_error(error));
                }
                self.notify_workspace_state();
                return ServerMessage::Workspaces(self.protocol_workspaces());
            }
            SessionCommand::ToggleShowDesktop => self.toggle_show_desktop(),
            SessionCommand::RemoveWorkspace { workspace } => {
                let transition = match self.workspaces.remove(WorkspaceId(workspace.0)) {
                    Ok(transition) => transition,
                    Err(error) => {
                        return protocol_error(ErrorCode::InvalidRequest, workspace_error(error));
                    }
                };
                self.apply_workspace_transition(transition);
            }
            SessionCommand::SwitchWorkspace { workspace, output } => {
                if output.as_ref().is_some_and(|name| {
                    !self
                        .space
                        .outputs()
                        .any(|candidate| candidate.name() == *name)
                }) {
                    return protocol_error(ErrorCode::InvalidRequest, "unknown output");
                }
                let transition = match self.workspaces.switch_to(WorkspaceId(workspace.0), output) {
                    Ok(transition) => transition,
                    Err(error) => {
                        return protocol_error(ErrorCode::InvalidRequest, workspace_error(error));
                    }
                };
                self.apply_workspace_transition(transition);
            }
            SessionCommand::MoveWindowToWorkspace { window, workspace } => {
                let id = WindowId(window.0);
                let transition = match self.workspaces.move_window(&id, WorkspaceId(workspace.0)) {
                    Ok(transition) => transition,
                    Err(error) => {
                        return protocol_error(ErrorCode::InvalidRequest, workspace_error(error));
                    }
                };
                self.apply_workspace_transition(transition);
            }
            SessionCommand::MoveWindowToOutput { window, output } => {
                if !self.window_exists(window) {
                    return protocol_error(ErrorCode::InvalidWindow, "unknown window id");
                }
                if !self.move_window_to_output(WindowId(window.0), &output) {
                    return protocol_error(ErrorCode::InvalidRequest, "unknown output");
                }
            }
            SessionCommand::HighlightWindow { window } => {
                if let Some(window) = window
                    && !self.window_exists(window)
                {
                    return protocol_error(ErrorCode::InvalidWindow, "unknown window id");
                }
                self.preview_highlight = window.map(|id| WindowId(id.0));
            }
            SessionCommand::WindowAction { window, action } => {
                if !self.window_exists(window) {
                    return protocol_error(ErrorCode::InvalidWindow, "unknown window id");
                }
                let id = WindowId(window.0);
                match action {
                    ProtocolWindowAction::Activate => self.activate_window(id),
                    ProtocolWindowAction::Close => self.close_window(id),
                    ProtocolWindowAction::Minimize => self.minimize_window(id),
                    ProtocolWindowAction::MaximizeRestore => self.maximize_window(id),
                    ProtocolWindowAction::FullscreenRestore => self.toggle_fullscreen_window(id),
                    ProtocolWindowAction::SnapLeading => {
                        self.activate_window(id);
                        self.snap_active_window(true);
                    }
                    ProtocolWindowAction::SnapTrailing => {
                        self.activate_window(id);
                        self.snap_active_window(false);
                    }
                }
            }
            SessionCommand::TestInput { input } => {
                if !self.test_control_enabled {
                    return protocol_error(
                        ErrorCode::Unauthorized,
                        "test input control is not enabled for this session",
                    );
                }
                if let Err(error) = self.inject_test_input(input) {
                    return protocol_error(ErrorCode::InvalidRequest, error);
                }
            }
            SessionCommand::TestOutput { output } => {
                if !self.test_control_enabled {
                    return protocol_error(
                        ErrorCode::Unauthorized,
                        "test output control is not enabled for this session",
                    );
                }
                if let Err(error) = self.apply_test_output(output) {
                    return protocol_error(ErrorCode::InvalidRequest, error);
                }
            }
        }
        ServerMessage::Ack
    }

    pub fn complete_output_capture(
        &mut self,
        path: &std::path::Path,
        result: nickel_session_protocol::CaptureResult,
    ) {
        let Some(reply_path) = self.output_capture_reply_path.take() else {
            let completed = {
                let mut internal = self.internal_capture.lock().unwrap();
                if let crate::session::InternalCaptureState::Pending(pending) = &*internal
                    && pending == path
                {
                    *internal =
                        crate::session::InternalCaptureState::Complete(path.to_path_buf(), result);
                    true
                } else {
                    false
                }
            };
            if completed {
                self.wake_internal_shell();
            }
            return;
        };
        let request_id = self.output_capture_request_id.take().unwrap_or_default();
        let message = ServerEnvelope {
            request_id,
            message: ServerMessage::Event(SessionEvent::OutputCaptureCompleted {
                path: path.to_string_lossy().into_owned(),
                result,
            }),
        };
        if let Ok(frame) = encode(&message)
            && let Ok(socket) = UnixDatagram::unbound()
        {
            let _ = socket.send_to(&frame, reply_path);
        }
    }

    pub(super) fn window_exists(&self, id: ProtocolWindowId) -> bool {
        self.windows.snapshot().iter().any(|window| {
            window.id.0 == id.0
                && !self.shell_owned_windows.contains(&window.id)
                && self.registry_window_is_mapped(window.id)
        })
    }

    pub(super) fn registry_window_is_mapped(&self, id: WindowId) -> bool {
        self.internal_window_surfaces.contains_key(&id)
            || self.x11_windows.values().any(|candidate| *candidate == id)
            || self.surface_windows.iter().any(|(surface, candidate)| {
                *candidate == id && self.mapped_xdg_toplevels.contains(surface)
            })
    }

    pub(super) fn protocol_windows(&mut self) -> Vec<WindowSnapshot> {
        let mut shell_ids = self
            .shell_windows()
            .filter_map(|window| {
                self.surface_windows
                    .get(&window.toplevel()?.wl_surface().id())
            })
            .copied()
            .collect::<HashSet<_>>();
        shell_ids.extend(self.shell_owned_windows.iter().copied());
        let windows = self
            .windows
            .snapshot()
            .into_iter()
            .filter(|window| !shell_ids.contains(&window.id))
            .filter(|window| self.registry_window_is_mapped(window.id))
            .filter(|window| self.workspaces.is_visible(&window.id))
            .take(nickel_session_protocol::MAX_WINDOWS)
            .map(|window| {
                let surface = self
                    .surface_windows
                    .iter()
                    .find_map(|(surface, id)| (*id == window.id).then_some(surface));
                let geometry = self
                    .internal_window_surfaces
                    .get(&window.id)
                    .and_then(|surface| self.internal_ui.placement(*surface))
                    .map(|placement| ProtocolGeometry {
                        x: placement.geometry.0,
                        y: placement.geometry.1,
                        width: placement.geometry.2 as i32,
                        height: placement.geometry.3 as i32,
                    })
                    .or_else(|| {
                        self.window_for_registry_id(window.id)
                            .and_then(|candidate| self.space.element_bbox(&candidate))
                            .map(|bounds| ProtocolGeometry {
                                x: bounds.loc.x,
                                y: bounds.loc.y,
                                width: bounds.size.w,
                                height: bounds.size.h,
                            })
                    })
                    .or_else(|| {
                        self.workspace_hidden_windows
                            .get(&window.id)
                            .map(|(hidden, location)| {
                                let size = hidden.geometry().size;
                                ProtocolGeometry {
                                    x: location.x,
                                    y: location.y,
                                    width: size.w,
                                    height: size.h,
                                }
                            })
                    })
                    .or_else(|| {
                        self.minimized_windows
                            .get(&window.id)
                            .map(|(hidden, location)| {
                                let size = hidden.geometry().size;
                                ProtocolGeometry {
                                    x: location.x,
                                    y: location.y,
                                    width: size.w,
                                    height: size.h,
                                }
                            })
                    });
                WindowSnapshot {
                    id: ProtocolWindowId(window.id.0),
                    application_id: window.app_id.clone(),
                    title: window.title.clone(),
                    active: window.active,
                    minimized: self.minimized_windows.contains_key(&window.id)
                        || self.internal_minimized_windows.contains(&window.id),
                    maximized: surface
                        .is_some_and(|surface| self.maximized_restore.contains_key(surface))
                        || self.space.elements().any(|candidate| {
                            candidate.x11_surface().is_some_and(|x11| {
                                self.x11_windows.get(&x11.window_id()).copied() == Some(window.id)
                                    && x11.is_maximized()
                            })
                        }),
                    fullscreen: surface
                        .is_some_and(|surface| self.fullscreen_restore.contains_key(surface))
                        || self.space.elements().any(|candidate| {
                            candidate.x11_surface().is_some_and(|x11| {
                                self.x11_windows.get(&x11.window_id()).copied() == Some(window.id)
                                    && self.x11_fullscreen_restore.contains_key(&x11.window_id())
                            })
                        }),
                    geometry,
                    workspace: ProtocolWorkspaceId(
                        self.workspaces
                            .workspace_for(&window.id)
                            .unwrap_or(WorkspaceId(1))
                            .0,
                    ),
                }
            })
            .collect::<Vec<_>>();
        let snapshot_bytes = windows
            .iter()
            .map(|window| window.title.len() + window.application_id.len())
            .sum();
        self.windows.begin_snapshot(snapshot_bytes);
        windows
    }

    pub(super) fn apply_test_output(&mut self, operation: TestOutput) -> Result<(), &'static str> {
        match operation {
            TestOutput::Connect {
                name,
                logical_width,
                logical_height,
                scale_120,
                transform,
            } => {
                if name.trim().is_empty() || name == "winit" {
                    return Err("test output needs a distinct non-empty name");
                }
                if self.space.outputs().any(|output| output.name() == name) {
                    return Err("output is already connected");
                }
                if self.space.outputs().count() >= nickel_session_protocol::MAX_OUTPUTS {
                    return Err("output limit reached");
                }
                if !self.output_global_admission_available() {
                    return Err("output global retirement backlog is full");
                }
                if logical_width < 320 || logical_height < 240 || !(60..=480).contains(&scale_120) {
                    return Err("invalid test output geometry or scale");
                }
                let smithay_transform = match transform {
                    OutputTransform::Normal => Transform::Normal,
                    OutputTransform::Rotate90 => Transform::_90,
                    OutputTransform::Rotate180 => Transform::_180,
                    OutputTransform::Rotate270 => Transform::_270,
                    OutputTransform::Flipped => Transform::Flipped,
                    OutputTransform::Flipped90 => Transform::Flipped90,
                    OutputTransform::Flipped180 => Transform::Flipped180,
                    OutputTransform::Flipped270 => Transform::Flipped270,
                };
                let scale = f64::from(scale_120) / 120.0;
                let scaled_width = (f64::from(logical_width) * scale).round() as i32;
                let scaled_height = (f64::from(logical_height) * scale).round() as i32;
                let rotated = matches!(
                    smithay_transform,
                    Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
                );
                let mode = OutputMode {
                    size: if rotated {
                        (scaled_height, scaled_width).into()
                    } else {
                        (scaled_width, scaled_height).into()
                    },
                    refresh: 60_000,
                };
                let x = self
                    .space
                    .outputs()
                    .filter_map(|output| self.space.output_geometry(output))
                    .map(|geometry| geometry.loc.x + geometry.size.w)
                    .max()
                    .unwrap_or(0);
                let output = Output::new(
                    name.clone(),
                    PhysicalProperties {
                        size: (0, 0).into(),
                        subpixel: Subpixel::Unknown,
                        make: "Nickel".into(),
                        model: "Nested test output".into(),
                        serial_number: name.clone(),
                    },
                );
                output.set_preferred(mode);
                output.change_current_state(
                    Some(mode),
                    Some(smithay_transform),
                    Some(OutputScale::Fractional(scale)),
                    Some((x, 0).into()),
                );
                let global = self
                    .output_global_identity_available(&name)
                    .then(|| output.create_global::<NickelSession>(&self.display_handle));
                self.space.map_output(&output, (x, 0));
                self.restore_output_windows(&output);
                self.virtual_test_outputs.insert(name, (output, global));
            }
            TestOutput::Disconnect { name } => {
                let Some((output, global)) = self.virtual_test_outputs.remove(&name) else {
                    return Err("unknown virtual test output");
                };
                self.stage_output_removal(&output);
                self.space.unmap_output(&output);
                output.leave_all();
                if let Some(global) = global {
                    self.defer_output_global_retirement(name.clone(), global);
                }
                self.reconcile_output_removal(&name);
                self.rescue_stranded_windows();
                self.relayout_maximized_windows();
                self.relayout_fullscreen_windows();
            }
        }
        self.relayout_shell_surfaces();
        self.reconstrain_all_reactive_popups();
        self.request_output_redraw();
        self.notify_protocol_snapshot();
        Ok(())
    }

    pub(crate) fn reconcile_output_removal(&mut self, name: &str) {
        if self.primary_output_name.as_deref() == Some(name) {
            self.primary_output_name = self.space.outputs().next().map(|output| output.name());
        }
        self.workspaces
            .output_disconnected(name, self.primary_output_name.clone());
        if self.last_interaction_output_name.as_deref() == Some(name) {
            self.last_interaction_output_name = None;
        }
        if self.launcher_output_name.as_deref() == Some(name) {
            self.launcher_output_name =
                self.resolve_interaction_output(InvocationSource::RecentInteraction);
        }
    }

    pub(super) fn protocol_outputs(&self) -> Vec<OutputSnapshot> {
        let outputs = self
            .space
            .outputs()
            .filter_map(|output| {
                let geometry = self.space.output_geometry(output)?;
                let physical = output.physical_properties();
                Some(OutputSnapshot {
                    name: output.name(),
                    model: physical.model,
                    geometry: ProtocolGeometry {
                        x: geometry.loc.x,
                        y: geometry.loc.y,
                        width: geometry.size.w,
                        height: geometry.size.h,
                    },
                    work_area: {
                        let area = self.work_area_for_output(Geometry {
                            x: geometry.loc.x,
                            y: geometry.loc.y,
                            width: geometry.size.w,
                            height: geometry.size.h,
                        });
                        ProtocolGeometry {
                            x: area.x,
                            y: area.y,
                            width: area.width,
                            height: area.height,
                        }
                    },
                    scale_120: (output.current_scale().fractional_scale() * 120.0).round() as u32,
                    transform: match output.current_transform() {
                        Transform::Normal => OutputTransform::Normal,
                        Transform::_90 => OutputTransform::Rotate90,
                        Transform::_180 => OutputTransform::Rotate180,
                        Transform::_270 => OutputTransform::Rotate270,
                        Transform::Flipped => OutputTransform::Flipped,
                        Transform::Flipped90 => OutputTransform::Flipped90,
                        Transform::Flipped180 => OutputTransform::Flipped180,
                        Transform::Flipped270 => OutputTransform::Flipped270,
                    },
                    physical_width_mm: physical.size.w,
                    physical_height_mm: physical.size.h,
                    primary: self.primary_output_name.as_deref() == Some(output.name().as_str()),
                    enabled: true,
                })
            })
            .take(nickel_session_protocol::MAX_OUTPUTS)
            .collect::<Vec<_>>();
        #[cfg(feature = "backend-udev")]
        let mut outputs = outputs;
        #[cfg(feature = "backend-udev")]
        if let Some(native) = self.native.as_ref() {
            for output in native.disabled_outputs() {
                if outputs.len() >= nickel_session_protocol::MAX_OUTPUTS {
                    break;
                }
                let Some(mode) = output.current_mode() else {
                    continue;
                };
                let physical = output.physical_properties();
                let location = output.current_location();
                outputs.push(OutputSnapshot {
                    name: output.name(),
                    model: physical.model,
                    geometry: ProtocolGeometry {
                        x: location.x,
                        y: location.y,
                        width: mode.size.w,
                        height: mode.size.h,
                    },
                    work_area: ProtocolGeometry {
                        x: location.x,
                        y: location.y,
                        width: mode.size.w,
                        height: mode.size.h,
                    },
                    scale_120: (output.current_scale().fractional_scale() * 120.0).round() as u32,
                    transform: OutputTransform::Normal,
                    physical_width_mm: physical.size.w,
                    physical_height_mm: physical.size.h,
                    primary: false,
                    enabled: false,
                });
            }
        }
        outputs
    }

    pub(crate) fn protocol_shell_surfaces(&self) -> Vec<ShellSurfaceSnapshot> {
        if let Some(shell) = &self.internal_shell {
            return shell
                .surfaces()
                .iter()
                .filter_map(|surface| {
                    let role = match surface.role {
                        crate::winit_shell::SurfaceRole::Desktop => ShellRole::Desktop,
                        crate::winit_shell::SurfaceRole::Panel => ShellRole::Panel,
                        crate::winit_shell::SurfaceRole::Launcher => ShellRole::Launcher,
                        crate::winit_shell::SurfaceRole::ControlCenter => ShellRole::ControlCenter,
                        crate::winit_shell::SurfaceRole::Notification => ShellRole::Notification,
                        crate::winit_shell::SurfaceRole::VolumeOsd => ShellRole::VolumeOsd,
                        crate::winit_shell::SurfaceRole::WindowPreview => ShellRole::Preview,
                        crate::winit_shell::SurfaceRole::WindowContextMenu => {
                            ShellRole::ContextMenu
                        }
                        crate::winit_shell::SurfaceRole::CodexProjectMenu => ShellRole::ProjectMenu,
                        crate::winit_shell::SurfaceRole::Lock => ShellRole::Lock,
                        crate::winit_shell::SurfaceRole::Screenshot => ShellRole::Screenshot,
                        crate::winit_shell::SurfaceRole::OnScreenKeyboard => {
                            ShellRole::OnScreenKeyboard
                        }
                        crate::winit_shell::SurfaceRole::CodexChat => return None,
                    };
                    Some(ShellSurfaceSnapshot {
                        role,
                        geometry: shell.visible(surface.id).then_some(ProtocolGeometry {
                            x: 0,
                            y: 0,
                            width: i32::try_from(surface.size.0).unwrap_or(i32::MAX),
                            height: i32::try_from(surface.size.1).unwrap_or(i32::MAX),
                        }),
                        output: surface.output.clone(),
                    })
                })
                .collect();
        }
        let registry = self.windows.snapshot();
        let outputs = self.space.outputs().cloned().collect::<Vec<_>>();
        let output_names = outputs.iter().map(Output::name).collect::<Vec<_>>();
        let mut registered_surfaces = HashSet::new();
        let mut snapshots = self
            .shell_windows()
            .filter_map(|window| {
                let surface = window.wl_surface()?;
                let id = self.surface_windows.get(&surface.id())?;
                let app_id = registry
                    .iter()
                    .find(|entry| entry.id == *id)
                    .map(|entry| entry.app_id.as_str())?;
                let role = ShellRole::from_application_id(app_id)?;
                registered_surfaces.insert(surface.id());
                let bounds = self.space.element_bbox(window);
                let geometry = bounds.map(|bounds| ProtocolGeometry {
                    x: bounds.loc.x,
                    y: bounds.loc.y,
                    width: bounds.size.w,
                    height: bounds.size.h,
                });
                let output = self
                    .shell_surface_output_name(window)
                    .and_then(|name| output_index_for_shell_surface(&name, &output_names))
                    .map(|index| output_names[index].clone())
                    .or_else(|| {
                        bounds.and_then(|bounds| {
                            outputs
                                .iter()
                                .filter_map(|output| {
                                    let output_bounds = self.space.output_geometry(output)?;
                                    let left = bounds.loc.x.max(output_bounds.loc.x);
                                    let top = bounds.loc.y.max(output_bounds.loc.y);
                                    let right = (bounds.loc.x + bounds.size.w)
                                        .min(output_bounds.loc.x + output_bounds.size.w);
                                    let bottom = (bounds.loc.y + bounds.size.h)
                                        .min(output_bounds.loc.y + output_bounds.size.h);
                                    let area = i64::from((right - left).max(0))
                                        * i64::from((bottom - top).max(0));
                                    Some((area, output.name()))
                                })
                                .max_by_key(|(area, _)| *area)
                                .filter(|(area, _)| *area > 0)
                                .map(|(_, name)| name)
                        })
                    });
                Some(ShellSurfaceSnapshot {
                    role,
                    geometry,
                    output,
                })
            })
            .take(nickel_session_protocol::MAX_WINDOWS)
            .collect::<Vec<_>>();

        // Registration is the shell protocol's surface authority. During the
        // startup barrier (and briefly while an output is being retired), a
        // valid role can exist before its first buffer maps a `Window`.
        for registration in &self.registered_shell_role_slots {
            if snapshots.len() == nickel_session_protocol::MAX_WINDOWS {
                break;
            }
            if registered_surfaces.insert(registration.surface.clone()) {
                snapshots.push(ShellSurfaceSnapshot {
                    role: registration.role,
                    geometry: None,
                    output: registration.output.clone(),
                });
            }
        }
        snapshots
    }

    pub(crate) fn protocol_shell_readiness(
        &self,
    ) -> nickel_session_protocol::ShellReadinessSnapshot {
        if let Some(shell) = &self.internal_shell {
            let outputs = u16::try_from(self.space.outputs().count()).unwrap_or(u16::MAX);
            let count = |role| {
                u16::try_from(
                    shell
                        .surfaces()
                        .iter()
                        .filter(|surface| surface.role == role)
                        .count(),
                )
                .unwrap_or(u16::MAX)
            };
            let desktops = count(crate::winit_shell::SurfaceRole::Desktop);
            let panels = count(crate::winit_shell::SurfaceRole::Panel);
            let locks = count(crate::winit_shell::SurfaceRole::Lock);
            let launchers = count(crate::winit_shell::SurfaceRole::Launcher);
            return nickel_session_protocol::ShellReadinessSnapshot {
                expected_shell_pid: None,
                authenticated_shell_pid: None,
                outputs,
                desktops,
                panels,
                locks,
                launchers,
                required_singletons_ready: true,
                output_roles_ready: desktops == outputs && locks == outputs && panels > 0,
                reserved_ordinary_windows: 0,
                ready: outputs > 0
                    && desktops == outputs
                    && locks == outputs
                    && panels > 0
                    && launchers == 1,
            };
        }
        let expected_shell_pid = match self
            .compatibility_control
            .as_ref()
            .map_or(0, |control| control.expected_shell_pid)
        {
            0 => None,
            pid => Some(pid),
        };
        let authenticated_shell_pid = self
            .compatibility_control
            .as_ref()
            .and_then(|control| control.authenticated_shell_pids.iter().copied().next());
        let outputs = u16::try_from(self.space.outputs().count()).unwrap_or(u16::MAX);
        let surfaces = self.protocol_shell_surfaces();
        let internal_surface_role = |role| {
            use crate::winit_shell::SurfaceRole;
            Some(match role {
                ShellRole::Desktop => SurfaceRole::Desktop,
                ShellRole::Panel => SurfaceRole::Panel,
                ShellRole::Launcher => SurfaceRole::Launcher,
                ShellRole::ControlCenter => SurfaceRole::ControlCenter,
                ShellRole::ContextMenu => SurfaceRole::WindowContextMenu,
                ShellRole::Preview => SurfaceRole::WindowPreview,
                ShellRole::Notification => SurfaceRole::Notification,
                ShellRole::VolumeOsd => SurfaceRole::VolumeOsd,
                ShellRole::ProjectMenu => SurfaceRole::CodexProjectMenu,
                ShellRole::Lock => SurfaceRole::Lock,
                ShellRole::Screenshot => SurfaceRole::Screenshot,
                ShellRole::OnScreenKeyboard => SurfaceRole::OnScreenKeyboard,
                ShellRole::Recovery => return None,
            })
        };
        let internal_role_count = |role| {
            self.internal_shell.as_ref().and_then(|shell| {
                let role = internal_surface_role(role)?;
                Some(
                    shell
                        .surfaces()
                        .iter()
                        .filter(|surface| surface.role == role)
                        .count(),
                )
            })
        };
        let role_count = |role| {
            let count = internal_role_count(role).unwrap_or_else(|| {
                surfaces
                    .iter()
                    .filter(|surface| surface.role == role)
                    .count()
            });
            u16::try_from(count).unwrap_or(u16::MAX)
        };
        let output_names = self
            .space
            .outputs()
            .map(|output| output.name())
            .collect::<HashSet<_>>();
        let settings = ShellSettings::load_default();
        let expected_panel_outputs = if settings.bar_on_all_displays {
            output_names.clone()
        } else {
            self.primary_output_name
                .clone()
                .or_else(|| output_names.iter().min().cloned())
                .into_iter()
                .collect::<HashSet<_>>()
        };
        let registered_role_count = |role| {
            let count = internal_role_count(role).unwrap_or_else(|| {
                self.registered_shell_role_slots
                    .iter()
                    .filter(|registration| {
                        registration.role == role
                            && shell_registration_is_active(
                                registration,
                                &output_names,
                                &expected_panel_outputs,
                            )
                    })
                    .count()
            });
            u16::try_from(count).unwrap_or(u16::MAX)
        };
        // Readiness is a protocol registration barrier, not a rendering
        // barrier. The shell deliberately waits for readiness before it
        // presents its first buffers, while XDG surfaces must remain unmapped
        // until those buffers arrive.
        let desktops = registered_role_count(ShellRole::Desktop);
        let panels = registered_role_count(ShellRole::Panel);
        let locks = registered_role_count(ShellRole::Lock);
        let launchers = registered_role_count(ShellRole::Launcher);
        let reserved_ordinary_windows = u16::try_from(
            self.windows
                .snapshot()
                .iter()
                .filter(|window| {
                    ShellRole::from_application_id(&window.app_id).is_some()
                        && !self.shell_owned_windows.contains(&window.id)
                })
                .count(),
        )
        .unwrap_or(u16::MAX);
        let required_singletons_ready = [
            ShellRole::Launcher,
            ShellRole::ControlCenter,
            ShellRole::ContextMenu,
            ShellRole::Preview,
            ShellRole::Notification,
            ShellRole::VolumeOsd,
            ShellRole::ProjectMenu,
            ShellRole::Screenshot,
            ShellRole::OnScreenKeyboard,
        ]
        .into_iter()
        .all(|role| registered_role_count(role) == 1 && role_count(role) <= 1);
        let expected_panels = u16::try_from(expected_panel_outputs.len()).unwrap_or(u16::MAX);
        let registered_role_outputs = |role| {
            if let Some(shell) = &self.internal_shell {
                let Some(role) = internal_surface_role(role) else {
                    return HashSet::new();
                };
                shell
                    .surfaces()
                    .iter()
                    .filter(|surface| surface.role == role)
                    .filter_map(|surface| surface.output.clone())
                    .collect::<HashSet<_>>()
            } else {
                self.registered_shell_role_slots
                    .iter()
                    .filter(|registration| {
                        registration.role == role
                            && shell_registration_is_active(
                                registration,
                                &output_names,
                                &expected_panel_outputs,
                            )
                    })
                    .filter_map(|registration| registration.output.clone())
                    .collect::<HashSet<_>>()
            }
        };
        let output_roles_ready = registered_role_outputs(ShellRole::Desktop) == output_names
            && registered_role_outputs(ShellRole::Panel) == expected_panel_outputs
            && registered_role_outputs(ShellRole::Lock) == output_names;
        let shell_authority_ready = self.internal_shell.is_some()
            || (expected_shell_pid.is_some() && expected_shell_pid == authenticated_shell_pid);
        let ready = shell_authority_ready
            && desktops == outputs
            && panels == expected_panels
            && locks == outputs
            && launchers == 1
            && required_singletons_ready
            && output_roles_ready
            && reserved_ordinary_windows == 0;
        nickel_session_protocol::ShellReadinessSnapshot {
            expected_shell_pid,
            authenticated_shell_pid,
            outputs,
            desktops,
            panels,
            locks,
            launchers,
            required_singletons_ready,
            output_roles_ready,
            reserved_ordinary_windows,
            ready,
        }
    }

    pub(crate) fn log_shell_readiness_if_changed(&mut self) {
        let readiness = self.protocol_shell_readiness();
        if self.last_logged_shell_readiness.as_ref() == Some(&readiness) {
            return;
        }
        tracing::info!(
            expected_shell_pid = ?readiness.expected_shell_pid,
            authenticated_shell_pid = ?readiness.authenticated_shell_pid,
            outputs = readiness.outputs,
            desktops = readiness.desktops,
            panels = readiness.panels,
            locks = readiness.locks,
            launchers = readiness.launchers,
            required_singletons_ready = readiness.required_singletons_ready,
            output_roles_ready = readiness.output_roles_ready,
            reserved_ordinary_windows = readiness.reserved_ordinary_windows,
            control_channel_health = "available",
            ready = readiness.ready,
            "shell readiness changed"
        );
        self.last_logged_shell_readiness = Some(readiness);
    }

    pub(super) fn protocol_snapshot(&mut self) -> SessionSnapshot {
        let windows = self.protocol_windows();
        SessionSnapshot {
            focused: windows
                .iter()
                .find(|window| window.active)
                .map(|window| window.id),
            stacking_front_to_back: windows.iter().map(|window| window.id).collect(),
            outputs: self.protocol_outputs(),
            windows,
            launcher_visible: self.launcher_visibility.is_visible(),
            locked: self.locked,
            workspaces: self.protocol_workspaces(),
        }
    }

    pub(super) fn protocol_workspaces(&self) -> WorkspaceState {
        let shell_ids = self
            .shell_windows()
            .filter_map(|window| {
                self.surface_windows
                    .get(&window.toplevel()?.wl_surface().id())
            })
            .copied()
            .collect::<HashSet<_>>();
        WorkspaceState {
            active: ProtocolWorkspaceId(self.workspaces.active().0),
            active_output: self.workspaces.active_output().map(str::to_owned),
            ordered: self
                .workspaces
                .ordered()
                .iter()
                .take(nickel_session_protocol::MAX_WORKSPACES)
                .map(|workspace| WorkspaceSnapshot {
                    id: ProtocolWorkspaceId(workspace.id.0),
                    windows: workspace
                        .windows
                        .iter()
                        .filter(|window| !shell_ids.contains(window))
                        .map(|window| ProtocolWindowId(window.0))
                        .collect(),
                    focused: workspace
                        .last_focused
                        .filter(|window| !shell_ids.contains(window))
                        .map(|window| ProtocolWindowId(window.0)),
                })
                .collect(),
        }
    }

    pub(super) fn protocol_secure_storage_state(&self) -> ProtocolSecureStorage {
        match self.secure_storage_state() {
            crate::session::login_services::SecureStorageState::Starting => {
                ProtocolSecureStorage::Starting
            }
            crate::session::login_services::SecureStorageState::Locked => {
                ProtocolSecureStorage::Locked
            }
            crate::session::login_services::SecureStorageState::PromptRequired => {
                ProtocolSecureStorage::PromptRequired
            }
            crate::session::login_services::SecureStorageState::Ready => {
                ProtocolSecureStorage::Ready
            }
            crate::session::login_services::SecureStorageState::Unavailable => {
                ProtocolSecureStorage::Unavailable
            }
        }
    }
}
