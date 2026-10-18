use super::{ChatMessage, draft_changed};
use crate::{ChatState, ConnectionStatus};
use nickel_codex::CodexSettings;
use twinkle_ui::SemanticRole;
use twinkle_ui::prelude::*;

pub(super) fn connection_menu(settings: &CodexSettings) -> Menu<ChatMessage> {
    let mut items = vec![MenuItem::new(
        if settings.selected == "local" {
            "✓ Local"
        } else {
            "Local"
        },
        ChatMessage::SelectConnection("local".into()),
    )];
    items.extend(settings.hosts.iter().map(|host| {
        MenuItem::new(
            if settings.selected == host.id {
                format!("✓ {}", host.name)
            } else {
                host.name.clone()
            },
            ChatMessage::SelectConnection(host.id.clone()),
        )
    }));
    items.push(MenuItem::new(
        "Manage remote hosts…",
        ChatMessage::ManageRemoteHosts,
    ));
    Menu::new(ChatMessage::ToggleFileMenu, "Connection", items).id(id!(connection_menu))
}

pub(super) fn bounded_menu_context(value: &str) -> String {
    // The complete title remains in the thread picker/window title; a source-
    // supplied title must not set the width of every app-chrome popover.
    let mut characters = value.chars();
    let prefix = characters.by_ref().take(48).collect::<String>();
    if characters.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

pub(super) fn thread_menu(
    state: &ChatState,
    project_root: Option<&std::path::Path>,
) -> Menu<ChatMessage> {
    // The menu remains available before the backend's thread list arrives.
    // Identity labels are context, not authority or a fabricated history row.
    let project = project_root
        .and_then(std::path::Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("Current workspace");
    let thread = state.selected_thread.as_ref().map_or_else(
        || "New conversation".to_owned(),
        |id| {
            state
                .threads
                .iter()
                .find(|thread| thread.id == *id)
                .and_then(|thread| thread.title.clone())
                .unwrap_or_else(|| format!("Conversation {}", id.0))
        },
    );
    Menu::new(
        ChatMessage::ToggleFileMenu,
        "Thread",
        [
            MenuItem::disabled(format!("Project: {}", bounded_menu_context(project))),
            MenuItem::disabled(format!("Current: {}", bounded_menu_context(&thread))),
            MenuItem::new("New conversation", ChatMessage::NewChat),
            MenuItem::new("Browse conversations…", ChatMessage::ToggleResumePicker),
        ],
    )
    .id(id!(thread_menu))
}

pub(super) fn project_menu_view(
    state: &ChatState,
    settings_error: Option<&str>,
    theme: SemanticTheme,
) -> impl View<ChatMessage> {
    let controller_focus = theme.borders.controller_focus;
    let project_query = state.draft.trim().to_lowercase();
    let matching_projects = state
        .projects
        .iter()
        .filter(|project| {
            !project.roots.is_empty()
                && (project_query.is_empty()
                    || project.name.to_lowercase().contains(&project_query)
                    || project.id.to_lowercase().contains(&project_query)
                    || project.roots.iter().any(|root| {
                        root.to_string_lossy()
                            .to_lowercase()
                            .contains(&project_query)
                    }))
        })
        .collect::<Vec<_>>();
    let status = match state.status {
        ConnectionStatus::Loading => "Loading projects…",
        ConnectionStatus::Ready if state.projects.is_empty() => "No projects available",
        ConnectionStatus::Ready if matching_projects.is_empty() => "No matching projects",
        ConnectionStatus::Ready => "Choose a project",
        ConnectionStatus::Unavailable => "Codex is not installed",
        ConnectionStatus::Disconnected => "Codex is disconnected",
        ConnectionStatus::Incompatible => "Codex is incompatible",
    };
    ui! {
        <Column fill_width fill_height padding={Insets::all(18.0)} gap={12.0}
            background={theme.surfaces.window} border={Border::new(theme.borders.ordinary, 1.0)} radius={14.0}>
            <Container fill_width shrink={0.0} padding={Insets::all(14.0)} gap={6.0}
                background={theme.surfaces.sidebar} border={Border::new(theme.borders.ordinary, 1.0)} radius={10.0}>
                <Row fill_width shrink={0.0} gap={8.0}>
                    <Text scale={1.5} color={theme.text.primary} grow={1.0}>{nickel_i18n::system_text("ui-view-codex-projects")}</Text>
                    <Button on_press={ChatMessage::Refresh} background={theme.surfaces.raised} color={theme.text.primary}
                        controller_focus_background_tint={controller_focus} radius={7.0}>{nickel_i18n::system_text("ui-view-refresh")}</Button>
                </Row>
                <Text color={theme.text.secondary} shrink={0.0}>{status}</Text>
            </Container>
            <Container id={id!(project_search_container)} accessibility_label={"Search projects"}
                semantic_role={SemanticRole::Group} fill_width shrink={0.0}
                padding={Insets::symmetric(12.0, 10.0)} background={theme.surfaces.card}
                border={Border::new(theme.borders.ordinary, 1.0)} radius={9.0}>
                <TextField id={id!(project_search)} value={&state.draft}
                    on_change={draft_changed} color={theme.text.primary} />
            </Container>
            {state.diagnostics.back().map(|diagnostic| ui! {
                <Container fill_width padding={Insets::all(8.0)} background={theme.surfaces.raised} radius={6.0}>
                    <Text color={theme.text.primary} max_lines={3}>{diagnostic}</Text>
                </Container>
            })}
            {settings_error.map(|error| ui! {
                <Container fill_width padding={Insets::all(8.0)} background={theme.surfaces.raised} radius={6.0}>
                    <Text color={theme.text.primary}>{error}</Text>
                </Container>
            })}
            <Column id={id!(project_menu_list)} grow={1.0} min_height={0.0}
                overflow_y={Overflow::Auto} gap={8.0}>
                {matching_projects.into_iter().map(|project| {
                    let root = project.roots[0].clone();
                    ui! {
                        <Button key={project.id.clone()} height={48.0}
                            on_press={ChatMessage::NewChatIn(root, project.id.clone())}
                            background={theme.surfaces.raised} color={theme.text.primary} label_align={TextAlign::Start}
                            controller_focus_background_tint={controller_focus}
                            focus_background_tint={theme.surfaces.hover}
                            border={Border::new(theme.borders.ordinary, 1.0)} radius={9.0}
                            padding={Insets::symmetric(14.0, 10.0)} fill_width>{&project.name}</Button>
                    }
                })}
            </Column>
        </Column>
    }
}

const RESUME_PREVIEW_LIMIT: usize = 180;

pub(crate) fn resume_preview(thread: &nickel_codex::Thread) -> Option<String> {
    let text = thread
        .turns
        .iter()
        .rev()
        .flat_map(|turn| turn.items.iter().rev())
        .find(|item| {
            matches!(
                item.item_type.as_str(),
                "userMessage" | "agentMessage" | "user_message" | "agent_message"
            ) && !item.text.trim().is_empty()
        })?
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = text.chars();
    let preview = chars
        .by_ref()
        .take(RESUME_PREVIEW_LIMIT)
        .collect::<String>();
    Some(if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    })
}

fn resume_recency(last_used_at: Option<i64>, now: i64) -> String {
    let Some(timestamp) = last_used_at else {
        return "Last used unknown".into();
    };
    let age = now.saturating_sub(timestamp).max(0);
    match age {
        0..=59 => "Used just now".into(),
        60..=3_599 => format!("Used {} min ago", age / 60),
        3_600..=86_399 => format!("Used {} hr ago", age / 3_600),
        _ => format!("Used {} days ago", age / 86_400),
    }
}

fn thread_belongs_to_project(
    thread: &nickel_codex::Thread,
    runtime: Option<&nickel_codex::ThreadRuntime>,
    project_root: Option<&std::path::Path>,
    project_id: Option<&str>,
) -> bool {
    runtime
        .and_then(|runtime| runtime.project_id.as_deref())
        .map_or_else(
            || {
                project_root.is_none_or(|root| {
                    thread
                        .cwd
                        .as_deref()
                        .is_some_and(|cwd| cwd.starts_with(root))
                })
            },
            |thread_project| project_id == Some(thread_project),
        )
}

pub(super) fn resume_picker(
    state: &ChatState,
    project_root: Option<&std::path::Path>,
    project_id: Option<&str>,
    loading: bool,
    pending: Option<&nickel_codex::ThreadId>,
    shell_host: bool,
    theme: SemanticTheme,
) -> AnyView<ChatMessage> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64);
    let threads = state
        .threads
        .iter()
        .filter_map(|thread| {
            let runtime = state.thread_runtime.get(&thread.id);
            thread_belongs_to_project(thread, runtime, project_root, project_id)
                .then_some((thread, runtime))
        })
        .collect::<Vec<_>>();
    let has_threads = !threads.is_empty();
    AnyView::new(ui! {
        <Column id={id!(resume_picker)} fill_width grow={1.0} min_height={0.0}
            padding={Insets::all(8.0)} gap={6.0} background={theme.surfaces.card}
            border={Border::new(theme.borders.ordinary, 1.0)} radius={8.0}>
            <Row fill_width gap={8.0}>
                <Text color={theme.text.primary} grow={1.0}>{nickel_i18n::system_text("ui-view-resume-conversation")}</Text>
                <Button on_press={ChatMessage::NewChat} background={theme.surfaces.hover} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-new")}</Button>
                <Button on_press={ChatMessage::CloseResumePicker} background={theme.surfaces.hover} color={theme.text.primary}>{nickel_i18n::system_text("ui-view-back-4")}</Button>
            </Row>
            {[()].into_iter().filter(|_| loading).map(|_| ui! {
                <Text color={theme.text.secondary}>{"Loading recent conversations…"}</Text>
            })}
            {[()].into_iter().filter(|_| !loading && state.thread_error.is_some()).map(|_| ui! {
                <Row fill_width gap={8.0}>
                    <Text color={theme.text.primary} grow={1.0}>{format!("Could not load conversations: {}", state.thread_error.as_deref().unwrap_or("Unknown error"))}</Text>
                    <Button on_press={ChatMessage::RefreshResumePicker} background={theme.surfaces.hover} color={theme.text.primary}>{"Retry"}</Button>
                </Row>
            })}
            {[()].into_iter().filter(|_| !loading && state.thread_error.is_none() && !has_threads).map(|_| ui! {
                <Text color={theme.text.secondary}>{if state.thread_next_cursor.is_some() {
                    "No conversations for this project on the loaded page; older pages remain."
                } else {
                    "No conversations yet for this project."
                }}</Text>
            })}
            <Column id={id!(resume_conversation_list)} fill_width grow={1.0} min_height={0.0}
                overflow_y={Overflow::Auto} gap={4.0}>
                {threads.into_iter().filter(|_| !loading && state.thread_error.is_none()).map(|(thread, runtime)| {
                    let title = thread.title.clone().filter(|title| !title.trim().is_empty())
                        .unwrap_or_else(|| "Untitled conversation".into());
                    let identity = thread.id.0.clone();
                    let preview = resume_preview(thread).unwrap_or_else(|| "No message preview available".into());
                    let label = format!("{title} — {} — {identity}\n{preview}", resume_recency(thread.last_used_at, now));
                    let active = runtime.is_some_and(|runtime| runtime.status == nickel_codex::ThreadRuntimeStatus::Active);
                    let resumable = runtime.is_some_and(|runtime| matches!(runtime.status, nickel_codex::ThreadRuntimeStatus::Idle | nickel_codex::ThreadRuntimeStatus::NotLoaded));
                    let waiting = pending == Some(&thread.id);
                    if (resumable || active && shell_host) && pending.is_none() {
                        AnyView::new(ui! {
                            <Column key={thread.id.0.clone()} fill_width gap={2.0}>
                                <Button label_align={TextAlign::Start}
                                    on_press={ChatMessage::SelectThread(thread.id.clone())}
                                    background={theme.surfaces.hover} color={theme.text.primary}
                                    accessibility_label={title.clone()} fill_width>{label}</Button>
                                {[()].into_iter().filter(|_| active).map(|_| ui! {
                                    <Text color={theme.text.secondary}>{"Already active"}</Text>
                                })}
                            </Column>
                        })
                    } else {
                        let status = if waiting { "Resuming…" } else if active { "Already active" } else { "Unavailable" };
                        AnyView::new(ui! {
                            <Column key={thread.id.0.clone()} fill_width padding={Insets::all(10.0)} background={theme.surfaces.sidebar} radius={6.0}>
                                <Text color={theme.text.secondary}>{label}</Text>
                                <Text color={theme.text.secondary}>{status}</Text>
                            </Column>
                        })
                    }
                })}
            </Column>
            {[()].into_iter().filter(|_| state.thread_windowed).map(|_| ui! {
                <Text color={theme.text.secondary}>{"Showing a bounded older-conversation window. Refresh to return to the newest page."}</Text>
            })}
            {state.thread_page_error.as_ref().map(|error| ui! {
                <Text color={theme.text.danger}>{format!("Could not load older conversations: {error}")}</Text>
            })}
            {[()].into_iter().filter(|_| !loading && state.thread_next_cursor.is_some() && !state.thread_paging).map(|_| ui! {
                <Button on_press={ChatMessage::LoadMoreThreads} background={theme.surfaces.hover}
                    color={theme.text.primary}>{"Older conversations"}</Button>
            })}
            {[()].into_iter().filter(|_| state.thread_paging).map(|_| ui! {
                <Text color={theme.text.secondary}>{"Loading older conversations…"}</Text>
            })}
        </Column>
    })
}
