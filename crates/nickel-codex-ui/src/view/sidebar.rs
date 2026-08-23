use std::path::Path;

use nickel_codex::Thread;
use nickel_ui::prelude::*;

use super::{ACCENT, BORDER, ChatMessage, MUTED, PANEL, SIDEBAR, TEXT};
use crate::{ChatState, ConnectionStatus};

const DEFAULT_TASK_LIMIT: usize = 10;

#[derive(Debug)]
struct ProjectSection<'a> {
    key: String,
    name: String,
    path: Option<String>,
    threads: Vec<&'a Thread>,
}

impl<'a> ProjectSection<'a> {
    fn visible_threads(&self, state: &ChatState) -> Vec<&'a Thread> {
        if state.expanded_projects.contains(&self.key) {
            return self.threads.clone();
        }
        let mut visible = self
            .threads
            .iter()
            .take(DEFAULT_TASK_LIMIT)
            .copied()
            .collect::<Vec<_>>();
        if let Some(selected) = self
            .threads
            .iter()
            .skip(DEFAULT_TASK_LIMIT)
            .find(|thread| state.selected_thread.as_ref() == Some(&thread.id))
        {
            visible.push(*selected);
        }
        visible
    }
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
    for thread in threads.iter().filter(|thread| {
        thread
            .cwd
            .as_deref()
            .is_none_or(|path| !path.starts_with("/tmp"))
    }) {
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
    for section in &mut sections {
        section
            .threads
            .sort_by_key(|thread| std::cmp::Reverse(thread.last_used_at.unwrap_or(i64::MIN)));
    }
    sections.sort_by_key(|section| {
        std::cmp::Reverse(
            section
                .threads
                .iter()
                .filter_map(|thread| thread.last_used_at)
                .max()
                .unwrap_or(i64::MIN),
        )
    });
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
                {projects.iter().map(|project| {
                    let visible_threads = project.visible_threads(state);
                    let expanded = state.expanded_projects.contains(&project.key);
                    ui! { <Column key={project.key.clone()} fill_width shrink={0.0} gap={5.0}>
                        <Text color={TEXT} scale={0.92} shrink={0.0}>{&project.name}</Text>
                        {project.path.as_ref().map(|path| ui! {
                            <Text color={MUTED} scale={0.72} ellipsis={true} shrink={0.0}>{path}</Text>
                        })}
                        {visible_threads.iter().map(|thread| ui! {
                            <Button key={thread.id.0.clone()}
                                on_press={ChatMessage::SelectThread(thread.id.clone())}
                                background={if state.selected_thread.as_ref() == Some(&thread.id) { 0x2a4261 } else { PANEL }}
                                color={TEXT} max_lines={2} fill_width>
                                {thread.title.as_deref().unwrap_or("Untitled conversation")}
                            </Button>
                        })}
                        {if project.threads.len() > DEFAULT_TASK_LIMIT {
                            let label = if expanded {
                                "Show less".to_owned()
                            } else {
                                format!("Show {} more", project.threads.len() - DEFAULT_TASK_LIMIT)
                            };
                            ui! {
                                <Button key={format!("{}-disclosure", project.key)}
                                    on_press={ChatMessage::ToggleProject(project.key.clone())}
                                    background={SIDEBAR} color={MUTED} fill_width>{label}</Button>
                            }
                        } else {
                            ui! { <Spacer height={0.0} /> }
                        }}
                    </Column> }
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
            last_used_at: None,
            turns: Vec::new(),
        }
    }

    fn recent_thread(id: &str, cwd: Option<&str>, last_used_at: i64) -> Thread {
        Thread {
            last_used_at: Some(last_used_at),
            ..thread(id, cwd)
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

    #[test]
    fn project_disclosure_is_bounded_independent_and_keeps_selection_visible() {
        let threads = (0..11)
            .map(|index| thread(&format!("task-{index}"), Some("/projects/nickel")))
            .chain((0..11).map(|index| thread(&format!("other-{index}"), Some("/projects/galen"))))
            .collect::<Vec<_>>();
        let sections = project_sections(&threads);
        let mut state = ChatState::default();
        assert_eq!(
            sections[0].visible_threads(&state).len(),
            DEFAULT_TASK_LIMIT
        );
        assert_eq!(
            sections[1].visible_threads(&state).len(),
            DEFAULT_TASK_LIMIT
        );

        state.selected_thread = Some(ThreadId("task-10".into()));
        assert_eq!(
            sections[0].visible_threads(&state).len(),
            DEFAULT_TASK_LIMIT + 1
        );
        assert_eq!(
            sections[0].visible_threads(&state).last().unwrap().id.0,
            "task-10"
        );

        state.expanded_projects.insert("/projects/galen".into());
        assert_eq!(
            sections[0].visible_threads(&state).len(),
            DEFAULT_TASK_LIMIT + 1
        );
        assert_eq!(sections[1].visible_threads(&state).len(), 11);
    }

    #[test]
    fn projects_and_tasks_sort_by_recency_and_temporary_work_is_hidden() {
        let threads = vec![
            recent_thread("nickel-old", Some("/projects/nickel"), 10),
            recent_thread("galen", Some("/projects/galen"), 30),
            recent_thread("nickel-new", Some("/projects/nickel"), 20),
            recent_thread("temporary", Some("/tmp/codex-worktree"), 100),
            recent_thread("not-temporary", Some("/tmp-project"), 40),
            thread("stable-a", Some("/projects/stable")),
            thread("stable-b", Some("/projects/stable")),
        ];

        let sections = project_sections(&threads);
        assert_eq!(
            sections
                .iter()
                .map(|section| section.name.as_str())
                .collect::<Vec<_>>(),
            vec!["tmp-project", "galen", "nickel", "stable"]
        );
        assert_eq!(
            sections[2]
                .threads
                .iter()
                .map(|thread| thread.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["nickel-new", "nickel-old"]
        );
        assert_eq!(
            sections[3]
                .threads
                .iter()
                .map(|thread| thread.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["stable-a", "stable-b"]
        );
        assert!(
            sections
                .iter()
                .all(|section| section.name != "codex-worktree")
        );
    }
}
