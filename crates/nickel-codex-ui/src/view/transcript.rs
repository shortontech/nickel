use super::{ChatMessage, CodexApprovalChoice, interaction_answer_changed};
use crate::model::{ActivityOutcome, is_collapsible_activity, item_label};
use crate::{ChatItem, ChatItemKind, ChatState, PendingInteraction};
use nickel_codex::CommandDecision;
use nickel_markdown::{MarkdownDocument, MarkdownPalette, markdown_content_view};
use twinkle_ui::approval::{ApprovalPresentation, RequesterIdentity};
use twinkle_ui::prelude::*;

pub(super) const TRANSCRIPT_GAP: f32 = 10.0;
const TRANSCRIPT_OVERSCAN: f32 = 900.0;
// Height estimates are deliberately cheap, not layout-exact. For a modest
// transcript, drawing every row is inexpensive and avoids a false leading
// spacer when rich Markdown renders much shorter than its text estimate.
pub(super) const TRANSCRIPT_VIRTUALIZATION_THRESHOLD: usize = 64;
pub(super) static EMPTY_ACTIVITY_OUTCOME: std::sync::LazyLock<ActivityOutcome> =
    std::sync::LazyLock::new(ActivityOutcome::default);

fn transcript_heights(state: &ChatState) -> Vec<f32> {
    state.estimated_item_heights()
}

pub(super) fn transcript_window(state: &ChatState) -> VirtualWindow {
    if state.items.len() <= TRANSCRIPT_VIRTUALIZATION_THRESHOLD {
        return VirtualWindow {
            range: 0..state.items.len(),
            leading: 0.0,
            trailing: 0.0,
            total: 0.0,
        };
    }
    let heights = transcript_heights(state);
    let offset = if state.conversation_pinned {
        f32::MAX
    } else {
        state.conversation_scroll
    };
    let mut window = VirtualWindow::from_heights(
        &heights,
        TRANSCRIPT_GAP,
        offset,
        state.conversation_viewport_height,
        TRANSCRIPT_OVERSCAN,
    );
    if state.conversation_pinned {
        // A long Markdown source can estimate much taller than its rendered
        // card. Keep a bounded tail mounted so a pinned transcript cannot
        // collapse into an apparent blank viewport above its last message.
        let earliest_tail = heights
            .len()
            .saturating_sub(TRANSCRIPT_VIRTUALIZATION_THRESHOLD);
        if window.range.start > earliest_tail {
            window.range.start = earliest_tail;
            window.leading = heights[..earliest_tail].iter().sum::<f32>()
                + TRANSCRIPT_GAP * earliest_tail as f32;
        }
    }
    window
}

#[component]
pub(super) fn ItemCard(
    item: &ChatItem,
    document: std::sync::Arc<MarkdownDocument>,
    expanded: bool,
    outcome: Option<&ActivityOutcome>,
    file_summary: Option<&str>,
    theme: SemanticTheme,
) -> impl View<ChatMessage> {
    let (background, color) = match &item.kind {
        ChatItemKind::User => (theme.surfaces.selected, theme.text.primary),
        ChatItemKind::Agent => (theme.surfaces.window, theme.text.primary),
        ChatItemKind::Reasoning => (theme.surfaces.sidebar, theme.text.secondary),
        ChatItemKind::Command => (theme.surfaces.hover, theme.text.primary),
        ChatItemKind::Activity => (theme.surfaces.card, theme.text.secondary),
        ChatItemKind::FileChange => (theme.surfaces.raised, theme.text.primary),
        ChatItemKind::Tool | ChatItemKind::Search | ChatItemKind::Image => {
            (theme.surfaces.hover, theme.text.primary)
        }
        ChatItemKind::Delegation => (theme.surfaces.sidebar, theme.text.primary),
        ChatItemKind::ApprovalReference => (theme.surfaces.raised, theme.text.warning),
        ChatItemKind::QuestionReference => (theme.surfaces.selected, theme.text.primary),
        ChatItemKind::SessionNotice => (theme.surfaces.card, theme.text.secondary),
        ChatItemKind::Plan => (theme.surfaces.selected, theme.text.primary),
        ChatItemKind::Warning => (theme.surfaces.raised, theme.text.warning),
        ChatItemKind::Error => (theme.surfaces.raised, theme.text.danger),
        ChatItemKind::Unknown(_) => (theme.surfaces.card, theme.text.secondary),
    };
    let label = item_label(&item.kind);
    let label_run_id = format!("{}/label", item.id);
    let is_activity = is_collapsible_activity(&item.kind);
    let summary = activity_summary(item, outcome, file_summary);
    let outcome_detail = outcome.and_then(|outcome| {
        let mut details = Vec::new();
        if let Some(duration_ms) = outcome.duration_ms {
            details.push(format!("Duration: {duration_ms} ms"));
        }
        (!details.is_empty()).then(|| details.join(" · "))
    });
    let exit_status = outcome.and_then(|outcome| outcome.exit_code).map(|code| {
        if code == 0 {
            (theme.text.success, format!("● {code}"))
        } else {
            (theme.text.danger, format!("● {code}"))
        }
    });
    let is_prose = matches!(item.kind, ChatItemKind::User | ChatItemKind::Agent);
    let padding = if item.kind == ChatItemKind::Agent {
        Insets::all(4.0)
    } else {
        Insets::all(10.0)
    };
    ui! {
        <Container fill_width padding={padding} gap={7.0}
            background={background}
            border={Border::new(theme.borders.ordinary, if is_prose { 0.0 } else { 1.0 })}
            radius={if is_prose { 0.0 } else { 10.0 }}>
            {[()].into_iter().filter(|_| item.kind != ChatItemKind::Agent).map(|_| ui! {
                <Text color={color} scale={0.9} selection_run_id={label_run_id.clone()}
                    selection_boundary={TextBoundary::Block}>{label}</Text>
            })}
            {[()].into_iter().filter(|_| is_activity).map(|_| ui! {
                <Column fill_width gap={6.0}>
                    <Row fill_width gap={6.0}>
                        {exit_status.iter().map(|(status_color, status)| ui! {
                            <Text color={*status_color}>{status.clone()}</Text>
                        })}
                        <Text color={color} wrap={true}>{summary.clone()}</Text>
                        {[()].into_iter().filter(|_| !item.text.is_empty()).map(|_| ui! {
                            <Button on_press={ChatMessage::ToggleActivityDetails(item.id.clone())}
                                background={theme.surfaces.hover} color={theme.text.secondary}>
                                {if expanded { "▾" } else { "▸" }}
                            </Button>
                        })}
                    </Row>
                    {outcome_detail.iter().map(|detail| ui! {
                        <Text color={theme.text.secondary}>{detail}</Text>
                    })}
                </Column>
            })}
            {[()].into_iter().filter(|_| !is_activity || expanded).map(|_| markdown_content_view(
                &document,
                MarkdownPalette {
                    foreground: color,
                    muted: theme.text.secondary,
                    accent: theme.accent.ordinary,
                    surface: theme.surfaces.sidebar,
                    border: theme.borders.ordinary,
                    code: theme.text.primary,
                },
                &format!("{}/body", item.id),
                |destination| ChatMessage::OpenMarkdownLink(destination.to_owned()),
            ))}
        </Container>
    }
}

pub(super) fn empty_activity_document() -> std::sync::Arc<MarkdownDocument> {
    static EMPTY: std::sync::OnceLock<std::sync::Arc<MarkdownDocument>> =
        std::sync::OnceLock::new();
    EMPTY
        .get_or_init(|| std::sync::Arc::new(MarkdownDocument::parse("")))
        .clone()
}

pub(super) fn activity_summary(
    item: &ChatItem,
    outcome: Option<&ActivityOutcome>,
    file_summary: Option<&str>,
) -> String {
    // Summaries use fields projected by known item adapters, not arbitrary prose in other cards.
    let state = outcome
        .and_then(|outcome| outcome.status.as_deref())
        .unwrap_or(if item.complete { "Finished" } else { "Running" });
    match &item.kind {
        ChatItemKind::Command => {
            let command = item.text.lines().next().unwrap_or_default();
            if command.starts_with("$ ") || command.starts_with('!') {
                format!("{state}: {}", command.chars().take(180).collect::<String>())
            } else {
                format!("{state} command")
            }
        }
        ChatItemKind::FileChange => file_summary
            .filter(|source| !source.is_empty())
            .map_or_else(
                || format!("{state} file change"),
                |source| format!("{state}: {source}"),
            ),
        ChatItemKind::Unknown(kind) => format!("Unsupported Codex activity: {kind}"),
        ChatItemKind::Tool
        | ChatItemKind::Search
        | ChatItemKind::Image
        | ChatItemKind::Delegation => item
            .text
            .lines()
            .next()
            .filter(|line| !line.is_empty())
            .map(|line| line.chars().take(180).collect())
            .unwrap_or_else(|| format!("{state} {}", item_label(&item.kind).to_lowercase())),
        _ => String::new(),
    }
}

// Keep every approval surface aligned with the decisions the controller can dispatch.
fn supported_approval_type(approval_type: &str) -> bool {
    matches!(
        approval_type,
        "item/fileChange/requestApproval" | "item/commandExecution/requestApproval"
    )
}

pub(super) fn approval_choices(
    approval_type: &str,
    context: &nickel_codex::ApprovalContext,
) -> Vec<(String, CodexApprovalChoice)> {
    if approval_type == "item/commandExecution/requestApproval"
        && let Some(decisions) = &context.available_decisions
    {
        return decisions
            .iter()
            .map(|decision| {
                let label = match decision {
                    CommandDecision::Accept => "Approve",
                    CommandDecision::AcceptForSession => "Approve for session",
                    CommandDecision::AcceptWithExecpolicyAmendment { .. } => {
                        "Approve and save command rule"
                    }
                    CommandDecision::ApplyNetworkPolicyAmendment { .. } => "Apply network rule",
                    CommandDecision::Decline => "Decline",
                    CommandDecision::Cancel => "Cancel turn",
                };
                (label.into(), CodexApprovalChoice::Command(decision.clone()))
            })
            .collect();
    }
    if !supported_approval_type(approval_type) {
        return Vec::new();
    }
    // Older servers and file-change approvals do not supply a decision list.
    [
        ("Cancel".into(), CodexApprovalChoice::Cancel),
        ("Decline".into(), CodexApprovalChoice::Decline),
        ("Approve".into(), CodexApprovalChoice::Approve),
    ]
    .into_iter()
    .collect()
}

pub(super) fn approval_choice_allowed(
    approval_type: &str,
    context: &nickel_codex::ApprovalContext,
    choice: &CodexApprovalChoice,
) -> bool {
    approval_choices(approval_type, context)
        .iter()
        .any(|(_, available)| available == choice)
}

pub(super) fn codex_approval_presentation(
    approval_type: &str,
    summary: &str,
    context: &nickel_codex::ApprovalContext,
) -> ApprovalPresentation {
    let mut detail = match &context.command {
        Some(command) => format!("Command: {command}\nReason: {summary}"),
        None => summary.into(),
    };
    // Session-wide and rule-changing choices require their exact scope in the
    // owning card before the user can choose them. Shell notifications route
    // these decisions here rather than compressing them into inline actions.
    for decision in context.available_decisions.iter().flatten() {
        match decision {
            CommandDecision::AcceptForSession => {
                detail.push_str("\nApprove for session: future matching prompts in this session may run without asking again.");
            }
            CommandDecision::AcceptWithExecpolicyAmendment {
                execpolicy_amendment,
            } => {
                detail.push_str("\nProposed command rule: ");
                detail.push_str(&execpolicy_amendment.join(", "));
            }
            CommandDecision::ApplyNetworkPolicyAmendment {
                network_policy_amendment,
            } => {
                detail.push_str(&format!(
                    "\nProposed network rule: {:?} {}",
                    network_policy_amendment.action, network_policy_amendment.host
                ));
            }
            _ => {}
        }
    }
    let warning = match (context.asks_network_access, context.proposes_session_rule) {
        (true, true) => Some("Requests network access and proposes a rule for future commands; review the available decision before approving.".into()),
        (true, false) => Some("Requests network access.".into()),
        (false, true) => Some("Proposes a rule for future commands; review the available decision before approving.".into()),
        (false, false) => None,
    };
    ApprovalPresentation {
        requester: "Codex".into(),
        identity: RequesterIdentity::BackendReported,
        action: match approval_type {
            "item/commandExecution/requestApproval"
                if context.kind.as_deref() == Some("writeStdin") =>
            {
                "Send input to a running command".into()
            }
            "item/commandExecution/requestApproval" => "Run a command".into(),
            "item/fileChange/requestApproval" => "Change files".into(),
            _ => "Perform an operation".into(),
        },
        scope: context
            .grant_root
            .as_ref()
            .map(|root| format!("Requested write root: {root}"))
            .or_else(|| {
                context
                    .cwd
                    .as_ref()
                    .map(|cwd| format!("Working directory: {cwd}"))
            }),
        duration: None,
        warning,
        detail: Some(detail),
    }
}

#[derive(Clone, Copy)]
pub(super) struct InteractionDeliveryStatus {
    pub(super) response_unconfirmed: bool,
    pub(super) notification_unavailable: bool,
}

#[component]
pub(super) fn InteractionCard(
    interaction: &PendingInteraction,
    answer: &str,
    actionable: bool,
    submitting: bool,
    delivery: InteractionDeliveryStatus,
    theme: SemanticTheme,
    compact_approval_details: bool,
) -> impl View<ChatMessage> {
    match interaction {
        PendingInteraction::Approval {
            request_id,
            approval_type,
            summary,
            context,
        } => {
            let presentation = codex_approval_presentation(approval_type, summary, context);
            let choices = approval_choices(approval_type, context);
            let actionable = actionable && !choices.is_empty();
            let is_command_approval = approval_type == "item/commandExecution/requestApproval";
            let command = context
                .command
                .as_deref()
                .filter(|command| !command.trim().is_empty());
            ui! {
                <Container fill_width padding={Insets::all(8.0)} gap={4.0}
                    background={theme.surfaces.raised} border={Border::new(theme.text.warning, 1.0)} radius={8.0}>
                    {[()].into_iter().filter(|_| !is_command_approval).map(|_| ui! {
                        <Text color={theme.text.primary}>{presentation.title()}</Text>
                    })}
                    {[()].into_iter().filter(|_| is_command_approval).map(|_| ui! {
                        <Container id={id!(approval_command)} fill_width
                            max_height={if compact_approval_details { 48.0 } else { 96.0 }}
                            overflow_y={Overflow::Auto} padding={Insets::all(8.0)}
                            background={theme.surfaces.card} radius={4.0}>
                            <Text color={if command.is_some() { theme.text.primary } else { theme.text.warning }} wrap={true}>
                                {command.unwrap_or("Codex did not provide the command to review.")}
                            </Text>
                        </Container>
                    })}
                    {[()].into_iter().filter(|_| !is_command_approval).map(|_| ui! {
                        <Text color={theme.text.primary}>{presentation.action.clone()}</Text>
                    })}
                    {[()].into_iter().filter(|_| delivery.notification_unavailable).map(|_| ui! {
                        <Text color={theme.text.warning} wrap={true}>{"Shell notification unavailable; review and respond here."}</Text>
                    })}
                    // Scope and source detail stay inspectable without moving the
                    // authority-owned decisions out of a compact viewport.
                    <Column id={id!(approval_details)} fill_width max_height={if compact_approval_details { 18.0 } else { 52.0 }}
                        overflow_y={Overflow::Auto} gap={4.0}>
                    <Text color={theme.text.secondary} wrap={true}>{presentation.scope_line()}</Text>
                    {presentation.warning.iter().map(|warning| ui! {
                        <Text color={theme.text.warning} wrap={true}>{warning}</Text>
                    })}
                    {presentation.detail.iter().map(|detail| ui! {
                        <Text color={theme.text.secondary} wrap={true}>{detail}</Text>
                    })}
                    </Column>
                    {[()].into_iter().filter(|_| !actionable).map(|_| ui! {
                        <Text color={theme.text.secondary}>{if submitting {
                            if delivery.response_unconfirmed { "Response unconfirmed; reconnect before acting again." }
                            else { "Response submitted; awaiting resolution." }
                        } else {
                            "Request unavailable on this connection."
                        }}</Text>
                    })}
                    {[()].into_iter().filter(|_| actionable && choices.len() <= 3).map(|_| ui! { <Row gap={8.0}>
                        {choices.iter().map(|(label, choice)| ui! {
                            <Button on_press={ChatMessage::RespondApproval(request_id.clone(), approval_type.clone(), choice.clone())}
                                background={theme.surfaces.hover} color={theme.text.primary}>{label.clone()}</Button>
                        })}
                    </Row> })}
                    {[()].into_iter().filter(|_| actionable && choices.len() > 3).map(|_| ui! { <Column gap={4.0}>
                        {choices.iter().map(|(label, choice)| ui! {
                            <Button on_press={ChatMessage::RespondApproval(request_id.clone(), approval_type.clone(), choice.clone())}
                                background={theme.surfaces.hover} color={theme.text.primary}>{label.clone()}</Button>
                        })}
                    </Column> })}
                </Container>
            }
        }
        PendingInteraction::UserInput {
            request_id,
            question_ids,
            questions,
        } => ui! {
            <Container fill_width padding={Insets::all(8.0)} gap={4.0}
                background={theme.surfaces.raised} border={Border::new(theme.borders.ordinary, 1.0)} radius={8.0}>
                <Text color={theme.text.primary}>{nickel_i18n::system_text("ui-view-codex-requested-input")}</Text>
                // Questions and option descriptions may be long, but the answer
                // field and authority-owned actions must remain outside this scroll.
                <Column id={id!(user_input_questions)} fill_width max_height={32.0}
                    overflow_y={Overflow::Auto} gap={4.0}>
                {questions.iter().map(|question| ui! {
                    <Column fill_width gap={4.0}>
                        <Text color={theme.text.primary}>{format!("{} — {}", question.header, question.question)}</Text>
                        {question.options.iter().map(|option| ui! {
                            <Text color={theme.text.secondary}>{format!("{}: {}", option.label, option.description)}</Text>
                        })}
                    </Column>
                })}
                <Text color={theme.text.secondary}>{nickel_i18n::system_text("ui-view-enter-one-answer-per-line-in-question-order-use-an-option-label-or-your-own-answer")}</Text>
                </Column>
                <Container fill_width padding={Insets::all(8.0)} background={theme.surfaces.card} radius={6.0}>
                    {[()].into_iter().filter(|_| questions.iter().any(|question| question.is_secret)).map(|_| ui! {
                        {TextField::on_change_masked(answer, '●', interaction_answer_changed).color(theme.text.primary)}
                    })}
                    {[()].into_iter().filter(|_| !questions.iter().any(|question| question.is_secret)).map(|_| ui! {
                        <TextField value={answer} on_change={interaction_answer_changed} color={theme.text.primary} />
                    })}
                </Container>
                {[()].into_iter().filter(|_| !actionable).map(|_| ui! {
                    <Text color={theme.text.secondary}>{if submitting {
                        if delivery.response_unconfirmed { "Answer unconfirmed; reconnect before acting again." }
                        else { "Answer submitted; awaiting resolution." }
                    } else {
                        "Request unavailable on this connection."
                    }}</Text>
                })}
                {[()].into_iter().filter(|_| actionable).map(|_| ui! { <Row gap={8.0}>
                    <Button on_press={ChatMessage::DismissInput(request_id.clone())}
                        background={theme.surfaces.hover} color={theme.text.danger}>{"Cancel"}</Button>
                    <Button on_press={ChatMessage::SubmitInput(request_id.clone(), question_ids.clone())}
                        background={theme.surfaces.hover} color={theme.text.success}>{"Submit"}</Button>
                </Row> })}
            </Container>
        },
    }
}
