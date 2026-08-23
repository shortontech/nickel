use std::path::Path;

use nickel_codex::Thread;
use nickel_ui::prelude::*;

use super::{ACCENT, BORDER, ChatMessage, MUTED, PANEL, SIDEBAR, TEXT};
use crate::{ChatState, ConnectionStatus};

#[derive(Debug)]
struct ProjectSection<'a> {
    key: String,
    name: String,
    path: Option<String>,
    threads: Vec<&'a Thread>,
}

fn project_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

fn project_sections(threads: &[Thread]) -> Vec<ProjectSection<'_>> {
    let mut sections: Vec<ProjectSection<'_>> = Vec::new();
    for thread in threads {
        let key = thread.cwd.as_ref().map_or_else(
            || "other-tasks".to_owned(),
            |path| path.display().to_string(),
        );
        if let Some(section) = sections.iter_mut().find(|section| section.key == key) {
            section.threads.push(thread);
            continue;
        }
        sections.push(ProjectSection {
            key,
            name: thread
                .cwd
                .as_deref()
                .map_or_else(|| "Other tasks".to_owned(), project_name),
            path: thread.cwd.as_ref().map(|path| path.display().to_string()),
            threads: vec![thread],
        });
    }
    sections
}

pub(super) fn thread_sidebar(state: &ChatState) -> impl View<ChatMessage> {
    let projects = project_sections(&state.threads);
    let account = if state.account.authenticated {
        "Authenticated"
    } else {
        "Not authenticated"
    };
    let status = match state.status {
        ConnectionStatus::Loading => "Connecting…",
        ConnectionStatus::Ready => "Ready",
        ConnectionStatus::Disconnected => "Disconnected",
        ConnectionStatus::Incompatible => "Incompatible backend",
    };

    ui! {
        <Column id={id!(thread_sidebar)} width={260.0} min_width={260.0} shrink={0.0}
            fill_height padding={Insets::all(14.0)} gap={10.0}
            background={SIDEBAR} border={Border::new(BORDER, 1.0)}>
            <Text id={id!(sidebar_title)} scale={1.6} color={TEXT} shrink={0.0}>{"Nickel Codex"}</Text>
            <Text color={ACCENT} shrink={0.0}>{status}</Text>
            <Text color={MUTED} scale={0.85} shrink={0.0}>{&state.provenance}</Text>
            <Text color={MUTED} scale={0.85} shrink={0.0}>{account}</Text>
            <Row id={id!(sidebar_actions)} shrink={0.0} gap={6.0}>
                <Button id={id!(new_chat)} on_press={ChatMessage::NewChat} background={0x244a73} color={TEXT}>{"New"}</Button>
                {if matches!(state.status, ConnectionStatus::Disconnected | ConnectionStatus::Incompatible) {
                    ui! { <Button on_press={ChatMessage::Reconnect} background={0x4a3030} color={TEXT}>{"Reconnect"}</Button> }
                } else {
                    ui! { <Button on_press={ChatMessage::Refresh} background={PANEL} color={TEXT}>{"Refresh"}</Button> }
                }}
            </Row>
            <Column id={id!(project_list)} grow={1.0} min_height={0.0} shrink={1.0}
                gap={12.0} overflow_y={Overflow::Auto}>
                {projects.iter().map(|project| ui! {
                    <Column key={project.key.clone()} fill_width shrink={0.0} gap={5.0}>
                        <Text color={TEXT} scale={0.92} shrink={0.0}>{&project.name}</Text>
                        {project.path.as_ref().map(|path| ui! {
                            <Text color={MUTED} scale={0.72} ellipsis={true} shrink={0.0}>{path}</Text>
                        })}
                        {project.threads.iter().map(|thread| ui! {
                            <Button key={thread.id.0.clone()}
                                on_press={ChatMessage::SelectThread(thread.id.clone())}
                                background={if state.selected_thread.as_ref() == Some(&thread.id) { 0x2a4261 } else { PANEL }}
                                color={TEXT} fill_width>
                                {thread.title.as_deref().unwrap_or("Untitled conversation")}
                            </Button>
                        })}
                    </Column>
                })}
            </Column>
        </Column>
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use nickel_codex::{Thread, ThreadId};

    use super::*;

    fn thread(id: &str, cwd: Option<&str>) -> Thread {
        Thread {
            id: ThreadId(id.into()),
            title: Some(id.into()),
            cwd: cwd.map(PathBuf::from),
            turns: Vec::new(),
        }
    }

    #[test]
    fn projects_preserve_first_seen_section_and_task_order() {
        let threads = vec![
            thread("a", Some("/projects/nickel")),
            thread("b", Some("/projects/galen")),
            thread("c", Some("/projects/nickel")),
            thread("d", None),
        ];
        let sections = project_sections(&threads);
        assert_eq!(
            sections
                .iter()
                .map(|section| section.name.as_str())
                .collect::<Vec<_>>(),
            vec!["nickel", "galen", "Other tasks"]
        );
        assert_eq!(
            sections[0]
                .threads
                .iter()
                .map(|thread| thread.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "c"]
        );
    }
}
