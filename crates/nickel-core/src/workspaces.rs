use std::collections::{HashMap, HashSet};
use std::hash::Hash;

pub const MAX_WORKSPACES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workspace<WindowId> {
    pub id: WorkspaceId,
    pub windows: Vec<WindowId>,
    pub last_focused: Option<WindowId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceTransition<WindowId> {
    pub hide: Vec<WindowId>,
    pub show: Vec<WindowId>,
    pub focus: Option<WindowId>,
}

impl<WindowId> Default for WorkspaceTransition<WindowId> {
    fn default() -> Self {
        Self {
            hide: Vec::new(),
            show: Vec::new(),
            focus: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceError {
    UnknownWorkspace,
    LastWorkspace,
    LimitReached,
    UnknownWindow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceDirection {
    Previous,
    Next,
}

#[derive(Debug)]
pub struct Workspaces<WindowId> {
    ordered: Vec<Workspace<WindowId>>,
    membership: HashMap<WindowId, WorkspaceId>,
    focus_excluded: HashSet<WindowId>,
    active: WorkspaceId,
    active_output: Option<String>,
    next_id: u64,
}

impl<WindowId> Default for Workspaces<WindowId> {
    fn default() -> Self {
        Self {
            ordered: vec![Workspace {
                id: WorkspaceId(1),
                windows: Vec::new(),
                last_focused: None,
            }],
            membership: HashMap::new(),
            focus_excluded: HashSet::new(),
            active: WorkspaceId(1),
            active_output: None,
            next_id: 1,
        }
    }
}

impl<WindowId: Clone + Eq + Hash> Workspaces<WindowId> {
    pub fn ordered(&self) -> &[Workspace<WindowId>] {
        &self.ordered
    }

    pub fn active(&self) -> WorkspaceId {
        self.active
    }

    pub fn active_output(&self) -> Option<&str> {
        self.active_output.as_deref()
    }

    pub fn output_disconnected(&mut self, name: &str, fallback: Option<String>) {
        if self.active_output.as_deref() == Some(name) {
            self.active_output = fallback;
        }
    }

    pub fn neighbor(&self, direction: WorkspaceDirection) -> WorkspaceId {
        let active = self
            .ordered
            .iter()
            .position(|workspace| workspace.id == self.active)
            .expect("active workspace always exists");
        let index = match direction {
            WorkspaceDirection::Previous => active.saturating_sub(1),
            WorkspaceDirection::Next => (active + 1).min(self.ordered.len() - 1),
        };
        self.ordered[index].id
    }

    pub fn workspace_for(&self, window: &WindowId) -> Option<WorkspaceId> {
        self.membership.get(window).copied()
    }

    pub fn is_visible(&self, window: &WindowId) -> bool {
        self.workspace_for(window) == Some(self.active)
    }

    pub fn create(&mut self) -> Result<WorkspaceId, WorkspaceError> {
        if self.ordered.len() >= MAX_WORKSPACES {
            return Err(WorkspaceError::LimitReached);
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = WorkspaceId(self.next_id);
        self.ordered.push(Workspace {
            id,
            windows: Vec::new(),
            last_focused: None,
        });
        Ok(id)
    }

    pub fn add_window(&mut self, window: WindowId) {
        self.remove_window(&window);
        self.focus_excluded.remove(&window);
        let active = self
            .ordered
            .iter_mut()
            .find(|workspace| workspace.id == self.active)
            .expect("active workspace always exists");
        active.windows.push(window.clone());
        self.membership.insert(window, self.active);
    }

    pub fn remove_window(&mut self, window: &WindowId) {
        self.focus_excluded.remove(window);
        let Some(workspace) = self.membership.remove(window) else {
            return;
        };
        let workspace = self
            .ordered
            .iter_mut()
            .find(|candidate| candidate.id == workspace)
            .expect("workspace membership always references a live workspace");
        workspace.windows.retain(|candidate| candidate != window);
        if workspace.last_focused.as_ref() == Some(window) {
            workspace.last_focused = workspace.windows.last().cloned();
        }
    }

    pub fn focused(&mut self, window: &WindowId) {
        let Some(workspace) = self.workspace_for(window) else {
            return;
        };
        let workspace = self
            .ordered
            .iter_mut()
            .find(|candidate| candidate.id == workspace)
            .expect("workspace membership always references a live workspace");
        workspace.windows.retain(|candidate| candidate != window);
        workspace.windows.push(window.clone());
        workspace.last_focused = Some(window.clone());
        self.focus_excluded.remove(window);
    }

    pub fn unfocused(&mut self, window: &WindowId) {
        let Some(workspace) = self.workspace_for(window) else {
            return;
        };
        let workspace = self
            .ordered
            .iter_mut()
            .find(|candidate| candidate.id == workspace)
            .expect("workspace membership always references a live workspace");
        if workspace.last_focused.as_ref() == Some(window) {
            workspace.last_focused = None;
        }
        self.focus_excluded.insert(window.clone());
    }

    pub fn switch_to(
        &mut self,
        target: WorkspaceId,
        output: Option<String>,
    ) -> Result<WorkspaceTransition<WindowId>, WorkspaceError> {
        let target_index = self
            .ordered
            .iter()
            .position(|workspace| workspace.id == target)
            .ok_or(WorkspaceError::UnknownWorkspace)?;
        self.active_output = output;
        if target == self.active {
            return Ok(WorkspaceTransition::default());
        }
        let current_index = self
            .ordered
            .iter()
            .position(|workspace| workspace.id == self.active)
            .expect("active workspace always exists");
        let hide = self.ordered[current_index].windows.clone();
        let show = self.ordered[target_index].windows.clone();
        let focus = self.ordered[target_index]
            .last_focused
            .clone()
            .filter(|window| !self.focus_excluded.contains(window))
            .or_else(|| {
                show.iter()
                    .rev()
                    .find(|window| !self.focus_excluded.contains(*window))
                    .cloned()
            });
        self.active = target;
        Ok(WorkspaceTransition { hide, show, focus })
    }

    pub fn move_window(
        &mut self,
        window: &WindowId,
        target: WorkspaceId,
    ) -> Result<WorkspaceTransition<WindowId>, WorkspaceError> {
        let source = self
            .workspace_for(window)
            .ok_or(WorkspaceError::UnknownWindow)?;
        if !self.ordered.iter().any(|workspace| workspace.id == target) {
            return Err(WorkspaceError::UnknownWorkspace);
        }
        if source == target {
            return Ok(WorkspaceTransition::default());
        }
        let focus_excluded = self.focus_excluded.contains(window);
        self.remove_window(window);
        let target_workspace = self
            .ordered
            .iter_mut()
            .find(|workspace| workspace.id == target)
            .expect("target workspace was validated");
        target_workspace.windows.push(window.clone());
        if !focus_excluded {
            target_workspace.last_focused = Some(window.clone());
        }
        self.membership.insert(window.clone(), target);
        if focus_excluded {
            self.focus_excluded.insert(window.clone());
        }

        let mut transition = WorkspaceTransition::default();
        if source == self.active {
            transition.hide.push(window.clone());
            transition.focus = self
                .ordered
                .iter()
                .find(|workspace| workspace.id == source)
                .and_then(|workspace| {
                    workspace
                        .last_focused
                        .clone()
                        .filter(|candidate| !self.focus_excluded.contains(candidate))
                        .or_else(|| {
                            workspace
                                .windows
                                .iter()
                                .rev()
                                .find(|candidate| !self.focus_excluded.contains(*candidate))
                                .cloned()
                        })
                });
        } else if target == self.active {
            transition.show.push(window.clone());
            transition.focus = (!focus_excluded).then(|| window.clone());
        }
        Ok(transition)
    }

    pub fn remove(
        &mut self,
        workspace: WorkspaceId,
    ) -> Result<WorkspaceTransition<WindowId>, WorkspaceError> {
        if self.ordered.len() == 1 {
            return Err(WorkspaceError::LastWorkspace);
        }
        let removed_index = self
            .ordered
            .iter()
            .position(|candidate| candidate.id == workspace)
            .ok_or(WorkspaceError::UnknownWorkspace)?;
        let destination_index = if removed_index == 0 {
            1
        } else {
            removed_index - 1
        };
        let destination_id = self.ordered[destination_index].id;
        let removed = self.ordered.remove(removed_index);
        let destination_index = self
            .ordered
            .iter()
            .position(|candidate| candidate.id == destination_id)
            .expect("removal destination survives");
        for window in &removed.windows {
            self.membership.insert(window.clone(), destination_id);
        }
        self.ordered[destination_index]
            .windows
            .extend(removed.windows.iter().cloned());
        if let Some(focused) = removed
            .last_focused
            .clone()
            .filter(|window| !self.focus_excluded.contains(window))
        {
            self.ordered[destination_index].last_focused = Some(focused);
        }

        if workspace != self.active {
            return Ok(WorkspaceTransition::default());
        }
        self.active = destination_id;
        let show = self.ordered[destination_index]
            .windows
            .iter()
            .filter(|window| !removed.windows.contains(window))
            .cloned()
            .collect();
        let focus = removed
            .last_focused
            .filter(|window| !self.focus_excluded.contains(window))
            .or_else(|| {
                self.ordered[destination_index]
                    .last_focused
                    .clone()
                    .filter(|window| !self.focus_excluded.contains(window))
            })
            .or_else(|| {
                self.ordered[destination_index]
                    .windows
                    .iter()
                    .rev()
                    .find(|window| !self.focus_excluded.contains(*window))
                    .cloned()
            });
        Ok(WorkspaceTransition {
            hide: Vec::new(),
            show,
            focus,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkspaceError, WorkspaceId, Workspaces};

    #[test]
    fn stable_order_membership_and_focus_survive_switches() {
        let mut workspaces = Workspaces::default();
        workspaces.add_window("editor");
        workspaces.add_window("terminal");
        workspaces.focused(&"editor");
        let second = workspaces.create().unwrap();

        assert_eq!(
            workspaces.switch_to(second, Some("DP-1".into())).unwrap(),
            super::WorkspaceTransition {
                hide: vec!["terminal", "editor"],
                show: vec![],
                focus: None,
            }
        );
        workspaces.add_window("browser");
        assert_eq!(workspaces.active_output(), Some("DP-1"));
        assert_eq!(workspaces.workspace_for(&"browser"), Some(second));

        assert_eq!(
            workspaces
                .switch_to(WorkspaceId(1), Some("HDMI-A-1".into()))
                .unwrap(),
            super::WorkspaceTransition {
                hide: vec!["browser"],
                show: vec!["terminal", "editor"],
                focus: Some("editor"),
            }
        );
    }

    #[test]
    fn moving_visible_window_hides_it_and_restores_remaining_focus() {
        let mut workspaces = Workspaces::default();
        workspaces.add_window("editor");
        workspaces.add_window("terminal");
        workspaces.focused(&"editor");
        workspaces.focused(&"terminal");
        let second = workspaces.create().unwrap();
        assert_eq!(
            workspaces.move_window(&"terminal", second).unwrap(),
            super::WorkspaceTransition {
                hide: vec!["terminal"],
                show: vec![],
                focus: Some("editor"),
            }
        );
        assert!(!workspaces.is_visible(&"terminal"));
    }

    #[test]
    fn removing_active_workspace_merges_into_stable_neighbor() {
        let mut workspaces = Workspaces::default();
        workspaces.add_window("first");
        workspaces.focused(&"first");
        let second = workspaces.create().unwrap();
        workspaces.switch_to(second, None).unwrap();
        workspaces.add_window("second");
        workspaces.focused(&"second");
        let transition = workspaces.remove(second).unwrap();
        assert_eq!(workspaces.active(), WorkspaceId(1));
        assert_eq!(transition.show, vec!["first"]);
        assert_eq!(transition.focus, Some("second"));
        assert_eq!(workspaces.workspace_for(&"second"), Some(WorkspaceId(1)));
        assert_eq!(
            workspaces.remove(WorkspaceId(1)),
            Err(WorkspaceError::LastWorkspace)
        );
    }

    #[test]
    fn disconnected_active_output_uses_the_connected_fallback() {
        let mut workspaces = Workspaces::<u64>::default();
        workspaces
            .switch_to(WorkspaceId(1), Some("DP-2".into()))
            .unwrap();
        workspaces.output_disconnected("DP-2", Some("DP-1".into()));
        assert_eq!(workspaces.active_output(), Some("DP-1"));
        workspaces.output_disconnected("missing", None);
        assert_eq!(workspaces.active_output(), Some("DP-1"));
    }

    #[test]
    fn unfocusing_a_hidden_window_clears_only_its_focus_authority() {
        let mut workspaces = Workspaces::default();
        workspaces.add_window(1_u64);
        workspaces.add_window(2);
        workspaces.focused(&2);
        workspaces.unfocused(&2);
        assert_eq!(workspaces.ordered()[0].last_focused, None);
        workspaces.focused(&1);
        workspaces.unfocused(&2);
        assert_eq!(workspaces.ordered()[0].last_focused, Some(1));
    }

    #[test]
    fn unfocused_windows_remain_visible_but_are_not_restored_as_focus() {
        let mut workspaces = Workspaces::default();
        workspaces.add_window("editor");
        workspaces.focused(&"editor");
        workspaces.unfocused(&"editor");
        let second = workspaces.create().unwrap();

        workspaces.switch_to(second, None).unwrap();
        let transition = workspaces.switch_to(WorkspaceId(1), None).unwrap();

        assert_eq!(transition.show, vec!["editor"]);
        assert_eq!(transition.focus, None);
    }

    #[test]
    fn moving_an_unfocused_window_does_not_make_it_a_focus_candidate() {
        let mut workspaces = Workspaces::default();
        workspaces.add_window("editor");
        workspaces.focused(&"editor");
        workspaces.unfocused(&"editor");
        let second = workspaces.create().unwrap();

        workspaces.move_window(&"editor", second).unwrap();
        let transition = workspaces.switch_to(second, None).unwrap();

        assert_eq!(transition.show, vec!["editor"]);
        assert_eq!(transition.focus, None);
    }
}
