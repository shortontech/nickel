use nickel_codex::{CommandDecision, FileChangeDecision, ServerRequestId};
use nickel_ui::prelude::*;

use crate::model::{TranscriptBlock, item_label, transcript_blocks};
use crate::{
    BackendMode, ChatController, ChatItem, ChatItemKind, ChatState, ConnectionStatus,
    ControllerCommand, PendingInteraction,
};

mod sidebar;

const BACKGROUND: Color = 0x101318;
const SIDEBAR: Color = 0x171b22;
const PANEL: Color = 0x202630;
const BORDER: Color = 0x343d4b;
const TEXT: Color = 0xe8edf4;
const MUTED: Color = 0x9ca8b8;
const ACCENT: Color = 0x70a5ff;
const USER: Color = 0x1d3557;
const ERROR: Color = 0x542a2a;
const TRANSCRIPT_GAP: f32 = 10.0;
const TRANSCRIPT_VIEWPORT_ESTIMATE: f32 = 600.0;
const TRANSCRIPT_OVERSCAN: f32 = 900.0;

#[derive(Clone, Debug, PartialEq)]
pub enum ChatMessage {
    DraftChanged(String),
    Send,
    NewChat,
    Refresh,
    Reconnect,
    SelectThread(nickel_codex::ThreadId),
    Interrupt,
    Approve(ServerRequestId, String),
    Decline(ServerRequestId, String),
    InteractionAnswerChanged(String),
    SubmitInput(ServerRequestId, Vec<String>),
    DismissInput(ServerRequestId),
    ConversationScrolled(f32),
    ToggleProject(String),
    ToggleFileMenu,
}

fn draft_changed(value: String) -> ChatMessage {
    ChatMessage::DraftChanged(value)
}

fn interaction_answer_changed(value: String) -> ChatMessage {
    ChatMessage::InteractionAnswerChanged(value)
}

fn conversation_scrolled(offset: f32) -> ChatMessage {
    ChatMessage::ConversationScrolled(offset)
}

fn transcript_heights(state: &ChatState) -> Vec<f32> {
    state.estimated_item_heights()
}

pub struct ChatApplication {
    pub state: ChatState,
    controller: ChatController,
    mode: BackendMode,
}

impl ChatApplication {
    pub fn new(mode: BackendMode) -> Self {
        Self {
            state: ChatState::default(),
            controller: ChatController::spawn(mode.clone()),
            mode,
        }
    }

    pub fn poll_controller(&mut self) -> bool {
        let mut changed = false;
        while let Some((generation, event)) = self.controller.try_recv() {
            changed |= self.state.apply(generation, event);
        }
        changed
    }
}

impl Application for ChatApplication {
    type Message = ChatMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            ChatMessage::DraftChanged(value) => self.state.draft = value,
            ChatMessage::Send => {
                if let Some(text) = self.state.begin_send() {
                    self.controller.send(ControllerCommand::Send(text));
                }
            }
            ChatMessage::NewChat => {
                self.state.new_chat();
                self.controller.send(ControllerCommand::NewChat);
            }
            ChatMessage::Refresh => {
                self.controller.send(ControllerCommand::Refresh);
            }
            ChatMessage::Reconnect => {
                self.state.generation += 1;
                self.state.status = ConnectionStatus::Loading;
                self.state.active_turn = None;
                self.state.interrupt_requested = false;
                self.controller =
                    ChatController::spawn_generation(self.mode.clone(), self.state.generation);
            }
            ChatMessage::SelectThread(id) => {
                self.state.begin_thread_selection(id.clone());
                self.controller.send(ControllerCommand::SelectThread(id));
            }
            ChatMessage::Interrupt => {
                if self.state.active_turn.is_some() && !self.state.interrupt_requested {
                    self.state.interrupt_requested = true;
                    self.controller.send(ControllerCommand::Interrupt);
                }
            }
            ChatMessage::Approve(request_id, approval_type) => {
                self.state.pending.retain(|pending| match pending {
                    PendingInteraction::Approval {
                        request_id: pending,
                        ..
                    }
                    | PendingInteraction::UserInput {
                        request_id: pending,
                        ..
                    } => pending != &request_id,
                });
                if approval_type.contains("fileChange") {
                    self.controller.send(ControllerCommand::FileApproval {
                        request_id,
                        decision: FileChangeDecision::Accept,
                    });
                } else {
                    self.controller.send(ControllerCommand::CommandApproval {
                        request_id,
                        decision: CommandDecision::Accept,
                    });
                }
            }
            ChatMessage::Decline(request_id, approval_type) => {
                self.state.pending.retain(|pending| match pending {
                    PendingInteraction::Approval {
                        request_id: pending,
                        ..
                    }
                    | PendingInteraction::UserInput {
                        request_id: pending,
                        ..
                    } => pending != &request_id,
                });
                if approval_type.contains("fileChange") {
                    self.controller.send(ControllerCommand::FileApproval {
                        request_id,
                        decision: FileChangeDecision::Decline,
                    });
                } else {
                    self.controller.send(ControllerCommand::CommandApproval {
                        request_id,
                        decision: CommandDecision::Decline,
                    });
                }
            }
            ChatMessage::InteractionAnswerChanged(value) => {
                self.state.interaction_answer = value;
            }
            ChatMessage::SubmitInput(request_id, question_ids) => {
                let answer = std::mem::take(&mut self.state.interaction_answer);
                let lines: Vec<_> = answer.lines().map(str::to_owned).collect();
                self.state.pending.retain(|pending| match pending {
                    PendingInteraction::Approval {
                        request_id: pending,
                        ..
                    }
                    | PendingInteraction::UserInput {
                        request_id: pending,
                        ..
                    } => pending != &request_id,
                });
                self.controller.send(ControllerCommand::UserInput {
                    request_id,
                    answers: question_ids
                        .into_iter()
                        .enumerate()
                        .map(|(index, question_id)| nickel_codex::UserInputAnswer {
                            question_id,
                            answer: lines.get(index).cloned().unwrap_or_default(),
                        })
                        .collect(),
                });
            }
            ChatMessage::DismissInput(request_id) => {
                self.state.pending.retain(|pending| match pending {
                    PendingInteraction::Approval {
                        request_id: pending,
                        ..
                    }
                    | PendingInteraction::UserInput {
                        request_id: pending,
                        ..
                    } => pending != &request_id,
                });
                self.controller.send(ControllerCommand::UserInput {
                    request_id,
                    answers: Vec::new(),
                });
            }
            ChatMessage::ConversationScrolled(offset) => {
                let heights = transcript_heights(&self.state);
                let total = VirtualWindow::from_heights(
                    &heights,
                    TRANSCRIPT_GAP,
                    f32::MAX,
                    TRANSCRIPT_VIEWPORT_ESTIMATE,
                    0.0,
                )
                .total;
                let maximum = (total - TRANSCRIPT_VIEWPORT_ESTIMATE).max(0.0);
                self.state.conversation_scroll = offset;
                self.state.conversation_pinned = offset >= maximum - 2.0;
            }
            ChatMessage::ToggleProject(project) => {
                if !self.state.expanded_projects.remove(&project) {
                    self.state.expanded_projects.insert(project);
                }
            }
            ChatMessage::ToggleFileMenu => {}
        }
    }

    fn poll(&mut self) -> bool {
        self.poll_controller()
    }

    fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        match shortcut {
            Shortcut::Submit if self.state.can_send() => {
                self.update(ChatMessage::Send);
                true
            }
            Shortcut::Newline => false,
            Shortcut::Escape if self.state.active_turn.is_some() => {
                self.update(ChatMessage::Interrupt);
                true
            }
            _ => false,
        }
    }

    fn view(&self) -> impl View<Self::Message> {
        chat_view(&self.state)
    }

    fn title(&self) -> &str {
        "Nickel"
    }

    fn initial_size(&self) -> (u32, u32) {
        (1120, 760)
    }
}

#[component]
fn ItemCard(item: &ChatItem) -> impl View<ChatMessage> {
    let (background, color) = match &item.kind {
        ChatItemKind::User => (USER, TEXT),
        ChatItemKind::Agent => (PANEL, TEXT),
        ChatItemKind::Reasoning => (0x252331, MUTED),
        ChatItemKind::Command => (0x242a24, TEXT),
        ChatItemKind::FileChange => (0x2c2920, TEXT),
        ChatItemKind::Plan => (0x202a35, TEXT),
        ChatItemKind::Error => (ERROR, TEXT),
        ChatItemKind::Unknown(_) => (PANEL, MUTED),
    };
    let label = item_label(&item.kind);
    let blocks = transcript_blocks(item);
    let label_run_id = format!("{}/label", item.id);
    let (maximum_width, alignment) = if item.kind == ChatItemKind::User {
        (760.0, Align::End)
    } else {
        (920.0, Align::Start)
    };
    ui! {
        <Container fill_width max_width={maximum_width} align_self={alignment}
            padding={Insets::all(14.0)} gap={7.0}
            background={background} border={Border::new(BORDER, 1.0)} radius={10.0}>
            <Text color={color} scale={0.9} selection_run_id={label_run_id}
                selection_boundary={TextBoundary::Block}>{label}</Text>
            <Column fill_width gap={5.0}>
                {blocks.iter().enumerate().map(|(index, block)| render_markdown_block(block, color, &item.id, index))}
            </Column>
        </Container>
    }
}

fn render_markdown_block(
    block: &TranscriptBlock,
    color: Color,
    item_id: &str,
    index: usize,
) -> AnyView<ChatMessage> {
    let run_id = format!("{item_id}/body/{index}");
    match block {
        TranscriptBlock::Heading(text) => AnyView::new(ui! {
            <Text color={color} scale={1.25} width_length={Length::Fill} wrap={true}
                selection_run_id={run_id} selection_boundary={TextBoundary::Block}>{text}</Text>
        }),
        TranscriptBlock::ListItem(text) => AnyView::new(ui! {
            <Text color={color} width_length={Length::Fill} wrap={true}
                selection_run_id={run_id} selection_boundary={TextBoundary::Block}>{text}</Text>
        }),
        TranscriptBlock::Code(text) => AnyView::new(ui! {
            <Container fill_width padding={Insets::all(9.0)} background={0x11151b}
                border={Border::new(BORDER, 1.0)} radius={6.0} overflow_x={Overflow::Auto}>
                <Text color={0xc8d6e5} selection_run_id={run_id}
                    selection_boundary={TextBoundary::Block}>{text}</Text>
            </Container>
        }),
        TranscriptBlock::Paragraph(text) => AnyView::new(ui! {
            <Text color={color} width_length={Length::Fill} wrap={true}
                selection_run_id={run_id} selection_boundary={TextBoundary::Block}>{text}</Text>
        }),
    }
}

#[component]
fn InteractionCard(interaction: &PendingInteraction, answer: &str) -> impl View<ChatMessage> {
    match interaction {
        PendingInteraction::Approval {
            request_id,
            approval_type,
            summary,
        } => ui! {
            <Container fill_width padding={Insets::all(12.0)} gap={8.0}
                background={0x332b1f} border={Border::new(0x8b6f35, 1.0)} radius={8.0}>
                <Text color={TEXT}>{"Approval requested"}</Text>
                <Text color={MUTED}>{summary}</Text>
                <Row gap={8.0}>
                    <Button on_press={ChatMessage::Decline(request_id.clone(), approval_type.clone())}
                        background={0x4a3030} color={TEXT}>{"Decline"}</Button>
                    <Button on_press={ChatMessage::Approve(request_id.clone(), approval_type.clone())}
                        background={0x27452f} color={TEXT}>{"Approve"}</Button>
                </Row>
            </Container>
        },
        PendingInteraction::UserInput {
            request_id,
            question_ids,
        } => ui! {
            <Container fill_width padding={Insets::all(12.0)} gap={8.0}
                background={0x262b38} border={Border::new(BORDER, 1.0)} radius={8.0}>
                <Text color={TEXT}>{"Codex requested input"}</Text>
                <Text color={MUTED}>{format!("Questions: {}", question_ids.join(", "))}</Text>
                <Text color={MUTED}>{"Enter one answer per line"}</Text>
                <Container fill_width padding={Insets::all(8.0)} background={PANEL} radius={6.0}>
                    <TextField value={answer} on_change={interaction_answer_changed} color={TEXT} />
                </Container>
                <Row gap={8.0}>
                    <Button on_press={ChatMessage::DismissInput(request_id.clone())}
                        background={0x4a3030} color={TEXT}>{"Cancel"}</Button>
                    <Button on_press={ChatMessage::SubmitInput(request_id.clone(), question_ids.clone())}
                        background={0x27452f} color={TEXT}>{"Submit"}</Button>
                </Row>
            </Container>
        },
    }
}

pub fn chat_view(state: &ChatState) -> impl View<ChatMessage> {
    let transcript_heights = transcript_heights(state);
    let transcript_offset = if state.conversation_pinned {
        f32::MAX
    } else {
        state.conversation_scroll
    };
    let transcript_window = VirtualWindow::from_heights(
        &transcript_heights,
        TRANSCRIPT_GAP,
        transcript_offset,
        TRANSCRIPT_VIEWPORT_ESTIMATE,
        TRANSCRIPT_OVERSCAN,
    );
    let transcript_range = transcript_window.range.clone();
    let transcript_document = state.transcript_selection_document();
    ui! {
        <Column fill_width fill_height background={BACKGROUND}>
            <MenuBar id={id!(menu_bar)}>
                <Menu id={id!(file_menu)} on_toggle={ChatMessage::ToggleFileMenu} label={"File"}>
                    <MenuItem label={"New conversation"} on_press={ChatMessage::NewChat} />
                    <MenuItem label={"Refresh"} on_press={ChatMessage::Refresh} />
                </Menu>
            </MenuBar>
            <Row fill_width grow={1.0} min_height={0.0}>
                {sidebar::thread_sidebar(state)}
                <Column grow={1.0} min_width={0.0} fill_height padding={Insets::all(18.0)} gap={12.0}>
                <Column id={id!(conversation)} grow={1.0} fill_width gap={10.0}
                    overflow_y={Overflow::Auto} follow_scroll_end={state.conversation_pinned}
                    on_scroll={conversation_scrolled}>
                    {if state.items.is_empty() {
                        ui! {
                            <Container grow={1.0} fill_width padding={Insets::all(28.0)}>
                                <Text scale={2.0} color={TEXT}>{"What are we building?"}</Text>
                                <Text color={MUTED}>{"Start a conversation with Codex. Tool requests always require an explicit decision."}</Text>
                            </Container>
                        }
                    } else {
                        ui! {
                            {SelectionRegion::new(transcript_document)
                                .id(id!(transcript_selection))
                                .fill_width()
                                .child(VirtualColumn::new()
                                    .window(transcript_window)
                                    .gap(TRANSCRIPT_GAP)
                                    .max_width(1000.0)
                                    .align_self(Align::Center)
                                    .children(state.items.iter().enumerate()
                                        .skip(transcript_range.start)
                                        .take(transcript_range.len())
                                        .map(|(_, item)| ui! { <ItemCard key={item.id.clone()} item={item} /> })))}
                        }
                    }}
                </Column>
                <Column id={id!(composer)} fill_width shrink={0.0} gap={8.0}>
                    {state.pending.iter().map(|interaction| ui! {
                        <InteractionCard interaction={interaction} answer={&state.interaction_answer} />
                    })}
                    {state.diagnostics.back().map(|diagnostic| ui! {
                        <Container fill_width padding={Insets::all(10.0)} background={ERROR} radius={6.0}>
                            <Text color={TEXT}>{diagnostic}</Text>
                        </Container>
                    })}
                    <Container fill_width min_height={52.0} max_height={140.0} shrink={0.0}
                        padding={Insets::all(12.0)} background={PANEL}
                        border={Border::new(BORDER, 1.0)} radius={10.0}
                        overflow_y={Overflow::Auto} follow_scroll_end={true}>
                        <TextField id={id!(chat_draft)} value={&state.draft} on_change={draft_changed}
                            color={TEXT} wrap={true} />
                    </Container>
                    <Row shrink={0.0} gap={8.0}>
                        <Text color={MUTED}>{if state.active_turn.is_some() { "Codex is working…" } else { "Explicit approval is always required" }}</Text>
                        <Spacer fill />
                        {if state.interrupt_requested {
                            ui! { <Text color={MUTED}>{"Interrupting…"}</Text> }
                        } else if state.active_turn.is_some() {
                            ui! { <Button on_press={ChatMessage::Interrupt} background={0x663333} color={TEXT}>{"Interrupt"}</Button> }
                        } else if state.can_send() {
                            ui! { <Button on_press={ChatMessage::Send} background={0x245b91} color={TEXT}>{"Send"}</Button> }
                        } else {
                            ui! { <Text color={MUTED}>{"Enter a message"}</Text> }
                        }}
                    </Row>
                </Column>
                </Column>
            </Row>
        </Column>
    }
}

#[cfg(test)]
mod tests {
    use nickel_ui::{PaintCommand, Rect, UiTree};

    use super::*;

    #[test]
    fn markdown_subset_is_safe_and_keeps_unsupported_html_as_text() {
        let item = ChatItem {
            id: "markdown".into(),
            kind: ChatItemKind::Agent,
            text: "# Heading\n- item with `code`\n```rust\nfn main() {}\n```\n<b>plain</b>".into(),
            complete: true,
        };
        assert_eq!(
            transcript_blocks(&item),
            vec![
                TranscriptBlock::Heading("Heading".into()),
                TranscriptBlock::ListItem("• item with ‹code›".into()),
                TranscriptBlock::Code("fn main() {}".into()),
                TranscriptBlock::Paragraph("<b>plain</b>".into()),
            ]
        );
    }

    #[test]
    fn multiline_code_block_reserves_padding_beyond_both_text_lines() {
        let item = ChatItem {
            id: "code".into(),
            kind: ChatItemKind::Agent,
            text: "```text\nfirst line\nsecond line\n```".into(),
            complete: true,
        };
        let tree = UiTree::layout(
            ui! { <ItemCard item={&item} /> },
            Rect::new(0.0, 0.0, 600.0, 200.0),
        );
        let text_bounds = tree
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::Text { bounds, text, .. } if text.contains("second line") => {
                    Some(*bounds)
                }
                _ => None,
            })
            .expect("multiline code text");
        let code_bounds = tree
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::RoundedFill { rect, color, .. } if *color == 0x11151b => Some(*rect),
                _ => None,
            })
            .expect("code container");
        assert!(text_bounds.size.height >= 31.0);
        assert!(code_bounds.size.height >= text_bounds.size.height + 18.0);
        assert!(text_bounds.origin.y >= code_bounds.origin.y + 9.0);
    }
}
