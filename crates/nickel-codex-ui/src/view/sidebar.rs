use std::path::Path;

use nickel_codex::{Thread, ThreadRuntimeStatus};
use nickel_ui::prelude::*;

use super::{ACCENT, BORDER, ChatMessage, MUTED, PANEL, SIDEBAR, TEXT};
use crate::{ChatState, ConnectionStatus};

const DEFAULT_TASK_LIMIT: usize = 10;

#[derive(Debug)]
struct ProjectSection<'a> {
    project_id: Option<String>,
    key: String,
    name: String,
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
        .map(|name| {
            name.split(['-', '_'])
                .map(|part| {
                    if part.eq_ignore_ascii_case("ui") {
                        "UI".to_owned()
                    } else {
                        let mut characters = part.chars();
                        characters.next().map_or_else(String::new, |first| {
                            first.to_uppercase().chain(characters).collect()
                        })
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| path.display().to_string())
}

fn project_sections<'a>(
    threads: &'a [Thread],
    state: &ChatState,
    available_only: bool,
) -> Vec<ProjectSection<'a>> {
    let mut sections: Vec<ProjectSection<'_>> = if available_only {
        state
            .projects
            .iter()
            .filter_map(|project| {
                let root = project.roots.first()?;
                Some(ProjectSection {
                    project_id: Some(project.id.clone()),
                    key: root.display().to_string(),
                    name: project.name.clone(),
                    threads: Vec::new(),
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    for thread in threads.iter().filter(|thread| {
        (!available_only
            || state
                .thread_runtime
                .get(&thread.id)
                .is_none_or(|runtime| runtime.status != ThreadRuntimeStatus::Active))
            && thread
                .cwd
                .as_deref()
                .is_none_or(|path| !path.starts_with("/tmp"))
    }) {
        let protocol_project = state
            .thread_runtime
            .get(&thread.id)
            .and_then(|runtime| runtime.project_id.as_deref())
            .and_then(|id| state.projects.iter().find(|project| project.id == id));
        if available_only && protocol_project.is_none() {
            continue;
        }
        let project_root = protocol_project.and_then(|project| project.roots.first());
        let key = project_root.or(thread.cwd.as_ref()).map_or_else(
            || "other-tasks".to_owned(),
            |path| path.display().to_string(),
        );
        if let Some(section) = sections.iter_mut().find(|section| section.key == key) {
            section.threads.push(thread);
            continue;
        }
        sections.push(ProjectSection {
            project_id: protocol_project.map(|project| project.id.clone()),
            key,
            name: protocol_project
                .map(|project| project.name.clone())
                .or_else(|| thread.cwd.as_deref().map(project_name))
                .unwrap_or_else(|| "Other tasks".to_owned()),
            threads: vec![thread],
        });
    }
    for section in &mut sections {
        section
            .threads
            .sort_by_key(|thread| std::cmp::Reverse(thread.last_used_at.unwrap_or(i64::MIN)));
    }
    sections.sort_by_key(|section| {
        (
            section.threads.is_empty(),
            std::cmp::Reverse(
                section
                    .threads
                    .iter()
                    .filter_map(|thread| thread.last_used_at)
                    .max()
                    .unwrap_or(i64::MIN),
            ),
        )
    });
    sections
}

pub(super) fn thread_sidebar(state: &ChatState, shell_hub: bool) -> impl View<ChatMessage> {
    let projects = project_sections(&state.threads, state, shell_hub);
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
            <Text id={id!(sidebar_title)} scale={1.6} color={TEXT} shrink={0.0}>{"Nickel"}</Text>
            <Text color={ACCENT} shrink={0.0}>{status}</Text>
            <Text color={MUTED} scale={0.85} shrink={0.0}>{&state.provenance}</Text>
            <Text color={MUTED} scale={0.85} shrink={0.0}>{account}</Text>
            <Row id={id!(sidebar_actions)} shrink={0.0} gap={6.0}>
                {if shell_hub {
                    ui! { <Text color={MUTED} scale={0.8}>{"Use + beside a project"}</Text> }
                } else {
                    ui! { <Button id={id!(new_chat)} on_press={ChatMessage::NewChat} background={0x244a73} color={TEXT}>{"New"}</Button> }
                }}
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
                    let collapsed = state.collapsed_projects.contains(&project.key);
                    ui! { <Column key={project.key.clone()} fill_width shrink={0.0} gap={2.0}>
                        <Row fill_width gap={3.0}>
                        <Button key={format!("{}-header", project.key)} width={190.0}
                            on_press={ChatMessage::ToggleProjectCollapsed(project.key.clone())}
                            background={SIDEBAR} color={TEXT} height={34.0}
                            padding={Insets { top: 6.0, right: 8.0, bottom: 5.0, left: 8.0 }}
                            label_align={TextAlign::Start} ellipsis={true} fill_width>
                            {format!("{}  📁  {}", if collapsed { "▸" } else { "▾" }, project.name)}
                        </Button>
                        {if shell_hub && project.key != "other-tasks" && project.project_id.is_some() {
                            ui! { <Button on_press={ChatMessage::NewChatIn(project.key.clone().into(), project.project_id.clone().unwrap_or_default())}
                                background={0x244a73} color={TEXT} width={36.0}>{"+"}</Button> }
                        } else { ui! { <Spacer width={0.0} /> } }}
                        </Row>
                        {if collapsed { ui! { <Spacer height={0.0} /> } } else { ui! {
                          <Column fill_width gap={2.0}>
                          {visible_threads.iter().map(|thread| ui! {
                            <Button key={thread.id.0.clone()}
                                on_press={ChatMessage::SelectThread(thread.id.clone())}
                                background={if state.selected_thread.as_ref() == Some(&thread.id) { PANEL } else { SIDEBAR }}
                                color={TEXT} max_lines={2} radius={10.0}
                                padding={Insets { top: 10.0, right: 8.0, bottom: 7.0, left: 30.0 }}
                                label_align={TextAlign::Start} fill_width>
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
                                    background={SIDEBAR} color={MUTED}
                                    padding={Insets { top: 8.0, right: 8.0, bottom: 6.0, left: 30.0 }}
                                    label_align={TextAlign::Start} fill_width>{label}</Button>
                            }
                        } else {
                            ui! { <Spacer height={0.0} /> }
                        }}
                          </Column>
                        }}}
                    </Column> }
                })}
            </Column>
        </Column>
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use nickel_codex::{Project, Thread, ThreadId, ThreadRuntime, ThreadRuntimeStatus};

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
            thread("b", Some("/projects/sample-project")),
            thread("c", Some("/projects/nickel")),
            thread("d", None),
        ];
        let sections = project_sections(&threads, &ChatState::default(), false);
        assert_eq!(
            sections
                .iter()
                .map(|section| section.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Nickel", "Sample Project", "Other tasks"]
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
        let threads =
            (0..11)
                .map(|index| thread(&format!("task-{index}"), Some("/projects/nickel")))
                .chain((0..11).map(|index| {
                    thread(&format!("other-{index}"), Some("/projects/sample-project"))
                }))
                .collect::<Vec<_>>();
        let sections = project_sections(&threads, &ChatState::default(), false);
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

        state
            .expanded_projects
            .insert("/projects/sample-project".into());
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
            recent_thread("sample", Some("/projects/sample-project"), 30),
            recent_thread("nickel-new", Some("/projects/nickel"), 20),
            recent_thread("temporary", Some("/tmp/codex-worktree"), 100),
            recent_thread("not-temporary", Some("/tmp-project"), 40),
            thread("stable-a", Some("/projects/stable")),
            thread("stable-b", Some("/projects/stable")),
        ];

        let sections = project_sections(&threads, &ChatState::default(), false);
        assert_eq!(
            sections
                .iter()
                .map(|section| section.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Tmp Project", "Sample Project", "Nickel", "Stable"]
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
                .all(|section| section.name != "Codex Worktree")
        );
    }

    #[test]
    fn shell_hub_includes_empty_projects_and_excludes_active_threads() {
        let threads = vec![
            thread("idle", None),
            thread("active", Some("/projects/nickel")),
        ];
        let mut state = ChatState::default();
        state.projects.extend([
            Project {
                id: "nickel".into(),
                name: "Nickel".into(),
                roots: vec!["/projects/nickel".into()],
            },
            Project {
                id: "empty-project".into(),
                name: "Empty Project".into(),
                roots: vec!["/projects/empty-project".into()],
            },
        ]);
        state.thread_runtime.insert(
            ThreadId("idle".into()),
            ThreadRuntime {
                project_id: Some("nickel".into()),
                status: ThreadRuntimeStatus::Idle,
                ..ThreadRuntime::default()
            },
        );
        state.thread_runtime.insert(
            ThreadId("active".into()),
            ThreadRuntime {
                status: ThreadRuntimeStatus::Active,
                ..ThreadRuntime::default()
            },
        );
        let sections = project_sections(&threads, &state, true);
        assert!(
            sections
                .iter()
                .any(|section| section.name == "Empty Project")
        );
        assert_eq!(
            sections
                .iter()
                .flat_map(|section| section.threads.iter())
                .map(|thread| thread.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["idle"]
        );
    }
}
