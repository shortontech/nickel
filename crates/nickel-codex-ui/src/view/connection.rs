use super::{
    ChatMessage, RemoteHostEditor, remote_host_cwd_changed, remote_host_endpoint_changed,
    remote_host_id_changed, remote_host_name_changed, remote_host_token_env_changed,
};
use crate::ChatState;
use nickel_codex::CodexSettings;
use twinkle_ui::prelude::*;

pub(super) fn remote_hosts_panel(
    settings: &CodexSettings,
    editor: Option<&RemoteHostEditor>,
    settings_error: Option<&str>,
    theme: SemanticTheme,
) -> AnyView<ChatMessage> {
    if let Some(editor) = editor {
        return AnyView::new(ui! {
            <Column fill_width grow={1.0} min_height={0.0} padding={Insets::all(24.0)} gap={12.0}
                background={theme.surfaces.window} overflow_y={Overflow::Auto}>
                <Row fill_width gap={8.0} align={Align::Center}>
                    <Column grow={1.0}>
                        <Text scale={1.6} color={theme.text.primary}>{if editor.original_id.is_some() { "Edit remote host" } else { "Add remote host" }}</Text>
                    </Column>
                    <Button on_press={ChatMessage::ManageRemoteHosts} background={theme.surfaces.hover} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-back")}</Button>
                </Row>
                <Text color={theme.text.secondary}>{nickel_i18n::system_text("ui-view-nickel-stores-only-the-environment-variable-name-never-its-secret-value")}</Text>
                <Text color={theme.text.primary}>{nickel_i18n::system_text("ui-view-identifier")}</Text>
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={6.0}>
                    <TextField value={&editor.id} on_change={remote_host_id_changed} color={theme.text.primary} />
                </Container>
                <Text color={theme.text.primary}>{nickel_i18n::system_text("ui-view-display-name")}</Text>
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={6.0}>
                    <TextField value={&editor.name} on_change={remote_host_name_changed} color={theme.text.primary} />
                </Container>
                <Text color={theme.text.primary}>{nickel_i18n::system_text("ui-view-websocket-endpoint")}</Text>
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={6.0}>
                    <TextField value={&editor.endpoint} on_change={remote_host_endpoint_changed} color={theme.text.primary} />
                </Container>
                <Text color={theme.text.primary}>{nickel_i18n::system_text("ui-view-bearer-token-environment-variable-optional")}</Text>
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={6.0}>
                    <TextField value={&editor.token_env} on_change={remote_host_token_env_changed} color={theme.text.primary} />
                </Container>
                <Text color={theme.text.primary}>{nickel_i18n::system_text("ui-view-default-working-directory-on-the-remote-host")}</Text>
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={6.0}>
                    <TextField value={&editor.default_cwd} on_change={remote_host_cwd_changed} color={theme.text.primary} />
                </Container>
                {settings_error.map(|error| ui! {
                    <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.raised} radius={6.0}>
                        <Text color={theme.text.danger}>{error}</Text>
                    </Container>
                })}
                <Row gap={8.0}>
                    <Button on_press={ChatMessage::ManageRemoteHosts} background={theme.surfaces.card} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-cancel")}</Button>
                    <Button on_press={ChatMessage::SaveRemoteHost} background={theme.accent.ordinary} color={theme.accent.on_accent}>{nickel_i18n::system_text("ui-view-save-host")}</Button>
                </Row>
            </Column>
        });
    }

    AnyView::new(ui! {
        <Column fill_width grow={1.0} min_height={0.0} padding={Insets::all(24.0)} gap={12.0}
            background={theme.surfaces.window} overflow_y={Overflow::Auto}>
            <Row fill_width gap={8.0} align={Align::Center}>
                <Column grow={1.0}>
                    <Text scale={1.6} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-remote-codex-hosts")}</Text>
                </Column>
                <Button on_press={ChatMessage::CloseRemoteHosts} background={theme.surfaces.hover} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-back-2")}</Button>
            </Row>
            <Text color={theme.text.secondary}>{nickel_i18n::system_text("ui-view-these-are-nickel-settings-nickel-does-not-read-or-modify-codex-desktop-configuration")}</Text>
            <Container fill_width padding={Insets::all(12.0)} background={theme.surfaces.card} radius={8.0}>
                <Text color={theme.text.primary}>{nickel_i18n::system_text("ui-view-local")}</Text>
                <Text color={theme.text.secondary}>{if settings.selected == "local" { "Selected" } else { "Uses the installed or bundled Codex CLI" }}</Text>
            </Container>
            {settings.hosts.iter().map(|host| ui! {
                <Container key={host.id.clone()} fill_width padding={Insets::all(12.0)} gap={6.0}
                    background={theme.surfaces.card} radius={8.0}>
                    <Text color={theme.text.primary}>{if settings.selected == host.id { format!("{} · Selected", host.name) } else { host.name.clone() }}</Text>
                    <Text color={theme.text.secondary}>{&host.endpoint}</Text>
                    <Text color={theme.text.secondary}>{format!("Remote cwd: {}", host.default_cwd)}</Text>
                    <Row gap={8.0}>
                        <Button on_press={ChatMessage::EditRemoteHost(host.id.clone())} background={theme.surfaces.sidebar} color={theme.text.primary}>{"Edit"}</Button>
                        <Button on_press={ChatMessage::RemoveRemoteHost(host.id.clone())} background={theme.surfaces.hover} color={theme.text.danger}>{"Remove"}</Button>
                    </Row>
                </Container>
            })}
            {settings_error.map(|error| ui! {
                <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.raised} radius={6.0}>
                    <Text color={theme.text.danger}>{error}</Text>
                </Container>
            })}
            <Row gap={8.0}>
                <Button on_press={ChatMessage::CloseRemoteHosts} background={theme.surfaces.card} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-done")}</Button>
                <Button on_press={ChatMessage::AddRemoteHost} background={theme.accent.ordinary} color={theme.accent.on_accent}>{nickel_i18n::system_text("ui-view-add-remote-host")}</Button>
            </Row>
        </Column>
    })
}

pub(super) fn login_panel(state: &ChatState, theme: SemanticTheme) -> AnyView<ChatMessage> {
    let challenge = state.login_challenge.as_ref();
    let (login_id, url, user_code) = match challenge {
        Some(nickel_codex::LoginChallenge::Browser { login_id, auth_url }) => {
            (Some(login_id), Some(auth_url), None)
        }
        Some(nickel_codex::LoginChallenge::DeviceCode {
            login_id,
            user_code,
            verification_url,
        }) => (Some(login_id), Some(verification_url), Some(user_code)),
        None => (None, None, None),
    };
    AnyView::new(ui! {
        <Column grow={1.0} fill_width align_self={Align::Center} max_width={560.0}
            padding={Insets::all(28.0)} gap={14.0}>
            <Text scale={1.8} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-sign-in-to-codex")}</Text>
            <Text color={theme.text.secondary}>{nickel_i18n::system_text("ui-view-authenticate-this-codex-profile-qr-codes-are-generated-locally-by-nickel")}</Text>
            {state.login_qr.as_ref().map(|qr| ui! {
                <Image asset_id={65001} image={qr.clone()} generation={1}
                    width={264.0} height={264.0} fit={ImageFit::Contain}
                    accessibility_label={"Login QR code"} />
            })}
            {user_code.map(|code| ui! {
                <Container fill_width padding={Insets::all(14.0)} background={theme.surfaces.card}
                    border={Border::new(theme.borders.ordinary, 1.0)} radius={8.0}>
                    <Text scale={1.4} color={theme.text.primary} selection_run_id={"login/user-code"}
                        selection_boundary={TextBoundary::Block}>{code}</Text>
                </Container>
            })}
            {url.map(|url| ui! {
                <Text color={theme.text.secondary} max_lines={3} selection_run_id={"login/url"}
                    selection_boundary={TextBoundary::Block}>{url}</Text>
            })}
            <Row gap={8.0}>
                {if let (Some(id), Some(url)) = (login_id, url) {
                    AnyView::new(ui! {
                        <Row gap={8.0}>
                            <Button on_press={ChatMessage::OpenLoginUrl(url.clone())}
                                background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Open"}</Button>
                            <Button on_press={ChatMessage::CopyLoginText(user_code.unwrap_or(url).clone())}
                                background={theme.surfaces.card} color={theme.text.primary}>
                                {if user_code.is_some() { "Copy code" } else { "Copy link" }}
                            </Button>
                            <Button on_press={ChatMessage::CancelLogin(id.clone())}
                                background={theme.surfaces.hover} color={theme.text.danger}>{"Cancel"}</Button>
                        </Row>
                    })
                } else {
                    AnyView::new(ui! {
                        <Row gap={8.0}>
                            <Button on_press={ChatMessage::StartLogin(nickel_codex::LoginMethod::Browser)}
                                background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Sign in with browser"}</Button>
                            <Button on_press={ChatMessage::StartLogin(nickel_codex::LoginMethod::DeviceCode)}
                                background={theme.surfaces.card} color={theme.text.primary}>{"Use phone or code"}</Button>
                        </Row>
                    })
                }}
            </Row>
            {if state.login_pending {
                AnyView::new(ui! { <Text color={theme.text.secondary}>{"Waiting for Codex…"}</Text> })
            } else if state.login_status == crate::model::LoginPresentationState::Waiting {
                AnyView::new(ui! { <Text color={theme.text.secondary}>{"Waiting for sign in…"}</Text> })
            } else if state.login_status == crate::model::LoginPresentationState::Cancelled {
                AnyView::new(ui! { <Text color={theme.text.secondary}>{"Sign in cancelled"}</Text> })
            } else if state.login_status == crate::model::LoginPresentationState::Failed {
                AnyView::new(ui! { <Text color={theme.text.danger}>{"Sign in failed — try again"}</Text> })
            } else {
                AnyView::new(Spacer::vertical(0.0))
            }}
        </Column>
    })
}

pub(super) fn codex_phone_access_panel(
    state: &ChatState,
    theme: SemanticTheme,
) -> AnyView<ChatMessage> {
    let status = state.remote_control_status.as_ref();
    let status_label = match status.map(|status| status.status) {
        Some(nickel_codex::RemoteControlConnectionStatus::Disabled) => "Disabled",
        Some(nickel_codex::RemoteControlConnectionStatus::Connecting) => "Connecting…",
        Some(nickel_codex::RemoteControlConnectionStatus::Connected) => "Connected",
        Some(nickel_codex::RemoteControlConnectionStatus::Errored) => "Error",
        None => "Checking…",
    };
    let connected = status.is_some_and(|status| {
        status.status == nickel_codex::RemoteControlConnectionStatus::Connected
    });
    let environment_id = status.and_then(|status| status.environment_id.as_deref());
    AnyView::new(ui! {
        <Column grow={1.0} fill_width align_self={Align::Center} max_width={520.0}
            padding={Insets::all(20.0)} gap={10.0}>
            <Row fill_width gap={8.0} align={Align::Center}>
                <Column grow={1.0}>
                    <Text scale={1.5} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-phone-access")}</Text>
                </Column>
                <Button on_press={ChatMessage::CloseRemoteControl}
                    background={theme.surfaces.hover} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-back-5")}</Button>
            </Row>
            <Text color={theme.text.secondary} wrap={true}>
                {nickel_i18n::system_text("ui-view-paired-phones-can-use-codex-remotely-nickel-desktop-permissions-remain-separate")}
            </Text>
            <Container fill_width padding={Insets::all(10.0)} background={theme.surfaces.card} radius={8.0}>
                <Row fill_width gap={10.0} align={Align::Center}>
                    <Column grow={1.0}>
                        <Text color={theme.text.primary}>{nickel_i18n::system_value("ui-codex-connection", "status", status_label)}</Text>
                    </Column>
                    {if connected {
                        AnyView::new(ui! { <Button on_press={ChatMessage::DisableRemoteControl}
                            background={theme.surfaces.hover} color={theme.text.danger}>{"Disable"}</Button> })
                    } else {
                        AnyView::new(ui! { <Button on_press={ChatMessage::EnableRemoteControl}
                            background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Enable"}</Button> })
                    }}
                </Row>
            </Container>
            {state.remote_pairing_qr.as_ref().map(|qr| ui! {
                <Image asset_id={65002} image={qr.clone()} generation={1}
                    width={264.0} height={264.0} fit={ImageFit::Contain}
                    accessibility_label={"Codex phone pairing QR code"} />
            })}
            {state.remote_pairing.as_ref().and_then(|pairing| pairing.manual_pairing_code.as_ref()).map(|code| ui! {
                <Column fill_width gap={6.0}>
                    <Text color={theme.text.secondary}>{"Manual pairing code"}</Text>
                    <Container fill_width padding={Insets::all(14.0)} background={theme.surfaces.card}
                        border={Border::new(theme.borders.ordinary, 1.0)} radius={8.0}>
                        <Text scale={1.4} color={theme.text.primary} selection_run_id={"phone/manual-code"}
                            selection_boundary={TextBoundary::Block}>{code}</Text>
                    </Container>
                    <Text color={theme.text.secondary}>{format!("Expires at {}", state.remote_pairing.as_ref().unwrap().expires_at)}</Text>
                </Column>
            })}
            <Row gap={8.0} align={Align::Center}>
                {if state.remote_pairing.is_some() {
                    AnyView::new(ui! {
                        <Row gap={8.0}>
                            {state.remote_pairing.as_ref().and_then(|pairing| pairing.manual_pairing_code.as_ref()).map(|code| ui! {
                                <Button on_press={ChatMessage::CopyLoginText(code.clone())}
                                    background={theme.surfaces.card} color={theme.text.primary}>{"Copy code"}</Button>
                            })}
                            <Button on_press={ChatMessage::CancelRemotePairing}
                                background={theme.surfaces.hover} color={theme.text.danger}>{"Stop waiting"}</Button>
                        </Row>
                    })
                } else {
                    if connected && !state.remote_control_pending {
                        AnyView::new(ui! { <Button on_press={ChatMessage::StartRemotePairing}
                            background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Pair a phone"}</Button> })
                    } else {
                        AnyView::new(ui! { <Text color={theme.text.secondary}>{"Pair a phone"}</Text> })
                    }
                }}
            </Row>
            {if state.remote_control_pending {
                AnyView::new(ui! { <Text color={theme.text.secondary}>{"Waiting for Codex…"}</Text> })
            } else { AnyView::new(Spacer::vertical(0.0)) }}
            {state.remote_control_message.as_ref().map(|message| ui! {
                <Text color={theme.text.secondary} wrap={true}>{message}</Text>
            })}
            {if state.remote_clients.is_empty() {
                AnyView::new(ui! { <Text color={theme.text.secondary}>{"No paired phones"}</Text> })
            } else {
                AnyView::new(ui! {
                    <Column fill_width gap={6.0}>
                        <Text color={theme.text.secondary}>{"Paired phones"}</Text>
                        {state.remote_clients.iter().map(|client| ui! {
                            <Container key={client.client_id.clone()} fill_width padding={Insets::all(8.0)}
                                background={theme.surfaces.card} radius={8.0}>
                                <Row fill_width gap={8.0} align={Align::Center}>
                                    <Column grow={1.0}>
                                        <Text color={theme.text.primary}>{client.display_name.as_deref().unwrap_or("Phone")}</Text>
                                    </Column>
                                    {environment_id.map(|environment_id| ui! {
                                        <Button on_press={ChatMessage::RevokeRemoteClient(environment_id.to_owned(), client.client_id.clone())}
                                            background={theme.surfaces.hover} color={theme.text.danger}>{"Revoke"}</Button>
                                    })}
                                </Row>
                            </Container>
                        })}
                    </Column>
                })
            }}
        </Column>
    })
}
