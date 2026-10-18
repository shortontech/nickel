use super::{
    APPROVAL_POLICIES, ChatMessage, approval_policy_description, approval_policy_label,
    sandbox_policy_label,
};
use crate::ChatState;
use nickel_codex::SandboxPolicy;
use twinkle_ui::SemanticRole;
use twinkle_ui::prelude::*;

#[derive(Clone, Copy)]
pub(super) struct SlashCommandOption {
    pub(super) command: &'static str,
    pub(super) label: &'static str,
    pub(super) available: bool,
}

const SLASH_COMMANDS: &[SlashCommandOption] = &[
    SlashCommandOption {
        command: "/model",
        label: "/model — choose model and reasoning",
        available: true,
    },
    SlashCommandOption {
        command: "/resume",
        label: "/resume — resume a project conversation",
        available: true,
    },
    SlashCommandOption {
        command: "/new",
        label: "/new — start a new conversation",
        available: true,
    },
    SlashCommandOption {
        command: "/clear",
        label: "/clear — clear into a new conversation",
        available: true,
    },
    SlashCommandOption {
        command: "/review",
        label: "/review — review uncommitted changes",
        available: true,
    },
    SlashCommandOption {
        command: "/compact",
        label: "/compact — compact the current conversation",
        available: true,
    },
    SlashCommandOption {
        command: "/plan",
        label: "/plan — toggle Plan mode",
        available: true,
    },
    SlashCommandOption {
        command: "/status",
        label: "/status — show Codex connection and run status",
        available: true,
    },
    SlashCommandOption {
        command: "/diff",
        label: "/diff — show working-tree changes",
        available: true,
    },
    SlashCommandOption {
        command: "/mention",
        label: "/mention — mention a project file",
        available: true,
    },
    SlashCommandOption {
        command: "/permissions",
        label: "/permissions — choose approval policy",
        available: true,
    },
    SlashCommandOption {
        command: "/feedback",
        label: "/feedback — send feedback to the Codex team",
        available: true,
    },
    SlashCommandOption {
        command: "/logout",
        label: "/logout — sign out of Codex",
        available: true,
    },
];

pub(super) fn slash_search_results(draft: &str) -> Option<Vec<SlashCommandOption>> {
    let fragment = draft.trim_start().strip_prefix('/')?;
    if fragment.chars().any(char::is_whitespace) {
        return None;
    }
    let needle = fragment.to_ascii_lowercase();
    Some(
        SLASH_COMMANDS
            .iter()
            .copied()
            .filter(|option| option.command[1..].contains(&needle))
            .collect(),
    )
}

pub(super) fn mention_fragment(draft: &str) -> Option<&str> {
    let token = draft
        .rsplit_once(char::is_whitespace)
        .map_or(draft, |(_, token)| token);
    token.strip_prefix('@')
}

pub(super) fn replace_mention_fragment(draft: &str, path: &str) -> String {
    let token_start = draft
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            character
                .is_whitespace()
                .then_some(index + character.len_utf8())
        })
        .unwrap_or(0);
    let mut result = draft[..token_start].to_owned();
    result.push('@');
    result.push_str(path);
    result.push(' ');
    result
}

pub(super) fn available_slash_commands(draft: &str) -> Vec<SlashCommandOption> {
    slash_search_results(draft)
        .unwrap_or_default()
        .into_iter()
        .filter(|option| option.available)
        .collect()
}

pub(super) fn run_settings_controls(
    state: &ChatState,
    model_picker_generation: u64,
    reasoning_picker_generation: u64,
    approval_picker_generation: u64,
    sandbox_picker_generation: u64,
    compact: bool,
    theme: SemanticTheme,
) -> AnyView<ChatMessage> {
    let mut controls: Vec<AnyView<ChatMessage>> = Vec::with_capacity(4);
    controls.push(AnyView::new(
        Dropdown::new(
            ChatMessage::ToggleModelPicker,
            state
                .models
                .iter()
                .find(|model| Some(model.id.as_str()) == state.selected_model.as_deref())
                .map(|model| model.display_name.clone())
                .unwrap_or_else(|| "Model".into()),
            state.models.iter().map(|model| {
                (
                    model.display_name.clone(),
                    ChatMessage::SelectModel(model.id.clone()),
                )
            }),
        )
        .id(id!(model_selector))
        .accessibility_label("Model selector")
        .semantic_role(SemanticRole::Button)
        .overlay(true)
        .open_generation(model_picker_generation)
        .colors(
            theme.surfaces.card,
            theme.surfaces.sidebar,
            theme.text.primary,
        ),
    ));
    if let Some(model) = state
        .models
        .iter()
        .find(|model| Some(model.id.as_str()) == state.selected_model.as_deref())
        .filter(|model| !model.supported_reasoning_efforts.is_empty())
    {
        controls.push(AnyView::new(
            Dropdown::new(
                ChatMessage::ToggleReasoningPicker,
                state
                    .selected_reasoning_effort
                    .clone()
                    .unwrap_or_else(|| "Effort".into()),
                model.supported_reasoning_efforts.iter().map(|option| {
                    (
                        format!("{} — {}", option.reasoning_effort, option.description),
                        ChatMessage::SelectReasoningEffort(option.reasoning_effort.clone()),
                    )
                }),
            )
            .id(id!(reasoning_effort_selector))
            .accessibility_label("Reasoning effort selector")
            .semantic_role(SemanticRole::Button)
            .overlay(true)
            .open_generation(reasoning_picker_generation)
            .colors(
                theme.surfaces.card,
                theme.surfaces.sidebar,
                theme.text.primary,
            ),
        ));
    }
    controls.push(AnyView::new(
        Dropdown::new(
            ChatMessage::ToggleApprovalPicker,
            approval_policy_label(state.selected_approval_policy),
            APPROVAL_POLICIES.into_iter().map(|policy| {
                (
                    format!(
                        "{} — {}",
                        approval_policy_label(policy),
                        approval_policy_description(policy)
                    ),
                    ChatMessage::SelectApprovalPolicy(policy),
                )
            }),
        )
        .id(id!(approval_policy_selector))
        .overlay(true)
        .open_generation(approval_picker_generation)
        .colors(
            theme.surfaces.card,
            theme.surfaces.sidebar,
            theme.text.primary,
        )
        .accessibility_label("Approval policy selector")
        .accessibility_description(format!(
            "Effective: {}. This does not change sandbox or filesystem access.",
            approval_policy_label(state.effective_approval_policy),
        ))
        .semantic_role(SemanticRole::Button),
    ));
    controls.push(AnyView::new(
        Dropdown::new(
            ChatMessage::ToggleSandboxPicker,
            sandbox_policy_label(state.selected_sandbox_policy),
            [
                SandboxPolicy::ReadOnly,
                SandboxPolicy::WorkspaceWrite,
                SandboxPolicy::DangerFullAccess,
            ]
            .into_iter()
            .map(|policy| {
                (
                    sandbox_policy_label(Some(policy)).to_owned(),
                    ChatMessage::SelectSandboxPolicy(Some(policy)),
                )
            }),
        )
        .id(id!(sandbox_policy_selector))
        .overlay(true)
        .open_generation(sandbox_picker_generation)
        .colors(
            theme.surfaces.card,
            theme.surfaces.sidebar,
            theme.text.primary,
        )
        .accessibility_label("Sandbox access selector")
        .accessibility_description(format!(
            "Effective: {}. YOLO is unsandboxed and sets approval policy to Never ask.",
            sandbox_policy_label(state.effective_sandbox_policy),
        ))
        .semantic_role(SemanticRole::Button),
    ));
    if compact {
        AnyView::new(Column::new().fill_width().gap(4.0).children(controls))
    } else {
        AnyView::new(Row::new().gap(8.0).children(controls))
    }
}

pub(super) fn feedback_panel(
    classification: Option<&str>,
    reason: &str,
    pending: bool,
    theme: SemanticTheme,
) -> impl View<ChatMessage> {
    const CATEGORIES: [(&str, &str, &str); 5] = [
        ("bug", "Bug", "Crash, error, hang, or broken behavior"),
        (
            "bad_result",
            "Bad result",
            "Incorrect, incomplete, or unhelpful output",
        ),
        (
            "good_result",
            "Good result",
            "Helpful or especially high-quality output",
        ),
        (
            "safety_check",
            "Safety check",
            "Benign work blocked by a safety check",
        ),
        (
            "other",
            "Other",
            "Performance, feature, or general UX feedback",
        ),
    ];
    Column::new()
        .fill_width()
        .gap(10.0)
        .children([
            AnyView::new(ui! {
                <Row fill_width align_items={Align::Center} gap={8.0}>
                    <Text scale={1.2} color={theme.text.primary} grow={1.0}>{nickel_i18n::system_text("ui-view-send-codex-feedback")}</Text>
                    {Button::new(ChatMessage::CloseFeedback, "Back")
                        .background(theme.surfaces.hover)
                        .color(theme.text.primary)
                        .enabled(!pending)}
                </Row>
            }),
            AnyView::new(ui! {
                <Column fill_width gap={6.0}>
                    <Text color={theme.text.secondary}>{nickel_i18n::system_text("ui-view-choose-a-category")}</Text>
                    {CATEGORIES.into_iter().map(|(value, label, description)| ui! {
                        <Button on_press={ChatMessage::SelectFeedbackCategory(value.into())}
                            enabled={!pending}
                            background={if classification == Some(value) { theme.accent.soft } else { theme.surfaces.hover }}
                            color={theme.text.primary} fill_width
                            accessibility_label={format!("{label}: {description}")}>
                            {format!("{label} — {description}")}
                        </Button>
                    })}
                </Column>
            }),
            AnyView::new(ui! {
                <Column fill_width gap={6.0}>
                    <Text color={theme.text.secondary}>{nickel_i18n::system_text("ui-view-optional-note")}</Text>
                    <Container fill_width min_height={80.0} padding={Insets::all(10.0)}
                        background={theme.surfaces.card}
                        border={Border::new(theme.borders.ordinary, 1.0)} radius={6.0}>
                        <TextField id={id!(feedback_reason)} value={reason}
                            on_change={ChatMessage::FeedbackReasonChanged}
                            wrap={true} color={theme.text.primary} />
                    </Container>
                </Column>
            }),
            AnyView::new(ui! {
                <Text color={theme.text.secondary} wrap={true}>
                    {nickel_i18n::system_text("ui-view-sending-diagnostics-may-include-codex-logs-for-this-session-choose-the-no-diagnostics-action-if-you-only-want-to-send-the-category-and-note")}
                </Text>
            }),
            AnyView::new(ui! {
                <Row fill_width gap={8.0}>
                    {Button::new(ChatMessage::SubmitFeedback(false), if pending { "Sending…" } else { "Send without diagnostics" })
                        .background(theme.surfaces.hover)
                        .color(theme.text.primary)
                        .enabled(classification.is_some() && !pending)}
                    {Button::new(ChatMessage::SubmitFeedback(true), "Send with diagnostics")
                        .background(theme.accent.ordinary)
                        .color(theme.accent.on_accent)
                        .enabled(classification.is_some() && !pending)}
                </Row>
            }),
        ])
}
