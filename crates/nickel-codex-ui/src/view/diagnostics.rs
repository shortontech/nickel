use super::ChatMessage;
use crate::model::RunPresentationStatus;
use crate::{BackendMode, ChatState, ConnectionStatus};
use twinkle_ui::prelude::*;

pub(super) fn run_status_label(status: RunPresentationStatus) -> &'static str {
    match status {
        RunPresentationStatus::Connecting => "Connecting to Codex…",
        RunPresentationStatus::Recovering => "Reloading conversation…",
        RunPresentationStatus::RecoveryFailed => "Conversation reload failed — reconnect to retry",
        RunPresentationStatus::Unconfirmed => "Earlier work is unconfirmed — reconnect",
        RunPresentationStatus::AuthenticationRequired => "Sign in to Codex",
        RunPresentationStatus::Ready => "Ready",
        RunPresentationStatus::Starting => "Starting turn…",
        RunPresentationStatus::Working => "Codex is working…",
        RunPresentationStatus::Interrupting => "Interrupting…",
        RunPresentationStatus::AwaitingApproval => "Awaiting approval",
        RunPresentationStatus::AwaitingInput => "Awaiting your answer",
        RunPresentationStatus::Retrying => "Codex is retrying…",
        RunPresentationStatus::TurnFailed => "Turn failed — see diagnostics",
        RunPresentationStatus::Unavailable => "Codex is unavailable",
        RunPresentationStatus::Disconnected => "Codex is disconnected",
        RunPresentationStatus::Incompatible => "Codex is incompatible",
    }
}

fn diagnostic_guidance(state: &ChatState) -> (&'static str, &'static str) {
    match state.status {
        ConnectionStatus::Unavailable => (
            "Codex is not available.",
            "Check that the selected Codex backend is installed, then reconnect.",
        ),
        ConnectionStatus::Disconnected => (
            "The Codex connection was interrupted.",
            "Reconnect to reload the conversation; unsent work will not be sent automatically.",
        ),
        ConnectionStatus::Incompatible => (
            "The Codex backend is incompatible with this version of Nickel.",
            "Use a supported Codex version or backend, then reconnect.",
        ),
        ConnectionStatus::Loading => (
            "Nickel is connecting to Codex.",
            "Wait for the connection and conversation reload to finish.",
        ),
        ConnectionStatus::Ready if state.recovery_failed => (
            "The conversation could not be reloaded.",
            "Reconnect to retry. Nickel will not replay a prompt or approval.",
        ),
        ConnectionStatus::Ready if state.recovery_pending => (
            "The conversation is being reloaded.",
            "Wait before sending a prompt or responding to a request.",
        ),
        ConnectionStatus::Ready if state.unconfirmed_work => (
            "An earlier turn or request outcome is unconfirmed.",
            "Reconnect to reload the conversation before acting again.",
        ),
        ConnectionStatus::Ready if !state.account.authenticated => (
            "Codex needs sign-in.",
            "Sign in to Codex, then retry the action.",
        ),
        ConnectionStatus::Ready if state.last_turn_error.is_some() => (
            "The last turn failed, but the connection is healthy.",
            "Review the turn details before deciding whether to send a new prompt.",
        ),
        ConnectionStatus::Ready => ("Codex is connected.", "No recovery action is needed."),
    }
}

pub(super) fn safe_diagnostic_summary(state: &ChatState, mode: &BackendMode) -> String {
    let backend = match mode {
        BackendMode::Live { .. } => match state.backend_source.as_ref() {
            Some(nickel_codex::CandidateSource::Installed) => "Installed Codex",
            Some(nickel_codex::CandidateSource::Bundled) => "Bundled Codex",
            Some(nickel_codex::CandidateSource::Explicit) => "Explicit local Codex",
            None => "Local Codex",
        },
        BackendMode::Remote { .. } => "Remote Codex",
        BackendMode::Replay { .. } => "Replay fixture",
    };
    let connection = match state.status {
        ConnectionStatus::Loading => "Connecting",
        ConnectionStatus::Ready => "Connected",
        ConnectionStatus::Unavailable => "Unavailable",
        ConnectionStatus::Disconnected => "Disconnected",
        ConnectionStatus::Incompatible => "Incompatible",
    };
    let (cause, next_action) = diagnostic_guidance(state);
    // Fixed labels only: backend diagnostics, IDs, paths, prompts, credentials,
    // approval payloads, and pairing secrets are deliberately never copied.
    format!(
        "Nickel Codex diagnostics\nBackend: {backend}\nConnection: {connection}\nStatus: {}\nCause: {cause}\nNext action: {next_action}",
        run_status_label(state.run_presentation_status())
    )
}

pub(super) fn diagnostics_panel(
    state: &ChatState,
    details_open: bool,
    copy_result: Option<&Result<(), String>>,
    theme: SemanticTheme,
) -> impl View<ChatMessage> {
    let can_reconnect = state.recovery_failed
        || state.unconfirmed_work
        || matches!(
            state.status,
            ConnectionStatus::Unavailable
                | ConnectionStatus::Disconnected
                | ConnectionStatus::Incompatible
        );
    let (cause, next_action) = diagnostic_guidance(state);
    ui! {
        <Column id={id!(codex_diagnostics)} fill_width gap={10.0} padding={Insets::all(12.0)}
            background={theme.surfaces.card} radius={8.0}>
            <Row fill_width gap={8.0} align_items={Align::Center}>
                <Text scale={1.25} color={theme.text.primary} grow={1.0}>{nickel_i18n::system_text("ui-view-codex-diagnostics")}</Text>
                <Button on_press={ChatMessage::ToggleDiagnostics}
                    background={theme.surfaces.hover} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-back-3")}</Button>
            </Row>
            <Text color={theme.text.primary}>{run_status_label(state.run_presentation_status())}</Text>
            {state.run_secondary_status().map(|secondary| ui! {
                <Text color={theme.text.secondary} wrap={true}>{secondary}</Text>
            })}
            <Text color={theme.text.primary} wrap={true}>{cause}</Text>
            <Text color={theme.text.secondary} wrap={true}>{next_action}</Text>
            <Text color={theme.text.primary}>{nickel_i18n::system_text("ui-view-account-usage")}</Text>
            {[()].into_iter().filter(|_| state.rate_limits_pending).map(|_| ui! {
                <Text color={theme.text.secondary}>{"Loading rate limits…"}</Text>
            })}
            {state.rate_limits_error.as_ref().map(|error| ui! {
                <Text color={theme.text.warning} wrap={true}>{format!("Rate limits unavailable: {error}")}</Text>
            })}
            {state.rate_limits.as_ref().map(|limits| ui! {
                <Column fill_width gap={4.0}>
                    <Text color={theme.text.secondary}>{match limits.ordinary_usage_allowed {
                        Some(true) => "Ordinary usage allowed",
                        Some(false) => "Ordinary usage blocked",
                        None => "Ordinary usage availability unknown",
                    }}</Text>
                    {limits.buckets.iter().map(|bucket| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{format!(
                            "{}: primary {} · secondary {}",
                            bucket.name,
                            bucket.primary_used_percent.map_or("unknown".into(), |used| format!("{used}% used")),
                            bucket.secondary_used_percent.map_or("unknown".into(), |used| format!("{used}% used")),
                        )}</Text>
                    })}
                </Column>
            })}
            <Row fill_width gap={8.0}>
                <Button on_press={ChatMessage::ToggleDiagnosticDetails}
                    background={theme.surfaces.hover} color={theme.text.primary}>{if details_open { "Hide technical details" } else { "Show technical details" }}</Button>
                <Button on_press={ChatMessage::CopyDiagnosticSummary}
                    background={theme.surfaces.hover} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-copy-safe-summary")}</Button>
                {[()].into_iter().filter(|_| matches!(copy_result, Some(Ok(())))).map(|_| ui! {
                    <Text id={id!(diagnostic_copy_confirmation)} color={theme.text.success}
                        accessibility_label={"Safe diagnostic summary copied to clipboard"}>{"Copied"}</Text>
                })}
                {[()].into_iter().filter(|_| matches!(copy_result, Some(Err(_)))).map(|_| ui! {
                    <Text id={id!(diagnostic_copy_failure)} color={theme.text.danger}
                        accessibility_label={"Safe diagnostic summary could not be copied"}>{"Copy failed"}</Text>
                })}
            </Row>
            {[()].into_iter().filter(|_| details_open).map(|_| ui! {
                <Column fill_width gap={6.0}>
                    <Text color={theme.text.secondary} wrap={true}>{format!("Backend: {}", state.provenance)}</Text>
                    {state.backend_source.as_ref().map(|source| ui! {
                        <Text color={theme.text.secondary}>{format!("Source: {source:?}")}</Text>
                    })}
                    {state.fallback_reason.as_ref().map(|reason| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{format!("Backend fallback: {reason}")}</Text>
                    })}
                    {state.selected_thread.as_ref().map(|thread| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{format!("Thread: {}", thread.0)}</Text>
                    })}
                    {state.active_turn.as_ref().map(|turn| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{format!("Turn: {}", turn.0)}</Text>
                    })}
                    {state.diagnostics.iter().rev().take(5).map(|message| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{message}</Text>
                    })}
                </Column>
            })}
            {[()].into_iter().filter(|_| can_reconnect).map(|_| ui! {
                <Button on_press={ChatMessage::Reconnect}
                    background={theme.accent.ordinary} color={theme.accent.on_accent}>{"Reconnect"}</Button>
            })}
        </Column>
    }
}
