use crate::hotkeys::HotkeyAction;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwitchDirection {
    Forward,
    Previous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwitchScope {
    AllWindows,
    ActiveApplication,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwitchWindow<Id> {
    pub id: Id,
    pub application_id: String,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskSwitchEffect<Id> {
    ShowFlip { session: u64 },
    RequestPreviews(Vec<Id>),
    SelectPreview(Id),
    HideFlip { session: u64 },
    ActivateWindow(Id),
}

#[derive(Debug)]
pub struct TaskSwitcher<Id> {
    next_session: u64,
    session: Option<u64>,
    scope: Option<SwitchScope>,
    candidates: Vec<Id>,
    selected: usize,
}

impl<Id> Default for TaskSwitcher<Id> {
    fn default() -> Self {
        Self {
            next_session: 0,
            session: None,
            scope: None,
            candidates: Vec::new(),
            selected: 0,
        }
    }
}

impl<Id: Clone + Eq> TaskSwitcher<Id> {
    pub fn session(&self) -> Option<u64> {
        self.session
    }

    pub fn selected(&self) -> Option<&Id> {
        self.candidates.get(self.selected)
    }

    pub fn candidates(&self) -> &[Id] {
        &self.candidates
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn apply(
        &mut self,
        action: HotkeyAction,
        windows_mru: &[SwitchWindow<Id>],
    ) -> Vec<TaskSwitchEffect<Id>> {
        match action {
            HotkeyAction::SwitchNext => self.step(
                windows_mru,
                SwitchScope::AllWindows,
                SwitchDirection::Forward,
            ),
            HotkeyAction::SwitchPrevious => self.step(
                windows_mru,
                SwitchScope::AllWindows,
                SwitchDirection::Previous,
            ),
            HotkeyAction::SwitchGroupNext => self.step(
                windows_mru,
                SwitchScope::ActiveApplication,
                SwitchDirection::Forward,
            ),
            HotkeyAction::SwitchGroupPrevious => self.step(
                windows_mru,
                SwitchScope::ActiveApplication,
                SwitchDirection::Previous,
            ),
            HotkeyAction::CommitSwitch => self.commit(),
            _ => Vec::new(),
        }
    }

    pub fn remove_candidate(&mut self, removed: &Id) -> Vec<TaskSwitchEffect<Id>> {
        let Some(removed_index) = self
            .candidates
            .iter()
            .position(|candidate| candidate == removed)
        else {
            return Vec::new();
        };
        let selected = self.selected().cloned();
        self.candidates.remove(removed_index);

        if self.candidates.is_empty() {
            self.scope = None;
            self.selected = 0;
            return self
                .session
                .take()
                .map(|session| vec![TaskSwitchEffect::HideFlip { session }])
                .unwrap_or_default();
        }

        if removed_index < self.selected || self.selected >= self.candidates.len() {
            self.selected = self.selected.saturating_sub(1);
        }
        let mut effects = vec![TaskSwitchEffect::RequestPreviews(self.candidates.clone())];
        if self.selected() != selected.as_ref()
            && let Some(selected) = self.selected().cloned()
        {
            effects.push(TaskSwitchEffect::SelectPreview(selected));
        }
        effects
    }

    fn step(
        &mut self,
        windows_mru: &[SwitchWindow<Id>],
        scope: SwitchScope,
        direction: SwitchDirection,
    ) -> Vec<TaskSwitchEffect<Id>> {
        let mut effects = Vec::new();
        if self.session.is_none() {
            let active_application = windows_mru
                .iter()
                .find(|window| window.active)
                .map(|window| window.application_id.as_str());
            self.candidates = windows_mru
                .iter()
                .filter(|window| {
                    scope == SwitchScope::AllWindows
                        || active_application == Some(window.application_id.as_str())
                })
                .map(|window| window.id.clone())
                .collect();
            if self.candidates.is_empty() {
                return effects;
            }
            self.next_session = self.next_session.wrapping_add(1).max(1);
            self.session = Some(self.next_session);
            self.scope = Some(scope);
            self.selected = match direction {
                SwitchDirection::Forward => usize::from(self.candidates.len() > 1),
                SwitchDirection::Previous => self.candidates.len().saturating_sub(1),
            };
            effects.push(TaskSwitchEffect::ShowFlip {
                session: self.next_session,
            });
            effects.push(TaskSwitchEffect::RequestPreviews(self.candidates.clone()));
        } else if !self.candidates.is_empty() {
            self.selected = match direction {
                SwitchDirection::Forward => (self.selected + 1) % self.candidates.len(),
                SwitchDirection::Previous => self
                    .selected
                    .checked_sub(1)
                    .unwrap_or(self.candidates.len() - 1),
            };
        }
        if let Some(selected) = self.selected().cloned() {
            effects.push(TaskSwitchEffect::SelectPreview(selected));
        }
        effects
    }

    fn commit(&mut self) -> Vec<TaskSwitchEffect<Id>> {
        let Some(session) = self.session.take() else {
            return Vec::new();
        };
        let selected = self.selected().cloned();
        self.scope = None;
        self.candidates.clear();
        self.selected = 0;
        let mut effects = vec![TaskSwitchEffect::HideFlip { session }];
        if let Some(selected) = selected {
            effects.push(TaskSwitchEffect::ActivateWindow(selected));
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::{SwitchWindow, TaskSwitchEffect, TaskSwitcher};
    use crate::hotkeys::HotkeyAction;

    fn window(id: &'static str, app: &'static str, active: bool) -> SwitchWindow<&'static str> {
        SwitchWindow {
            id,
            application_id: app.into(),
            active,
        }
    }

    #[test]
    fn group_switch_uses_active_application_and_orders_commit_effects() {
        let windows = [
            window("chrome-b", "chrome", true),
            window("code", "vscode", false),
            window("chrome-a", "chrome", false),
        ];
        let mut switcher = TaskSwitcher::default();
        assert_eq!(
            switcher.apply(HotkeyAction::SwitchGroupNext, &windows),
            [
                TaskSwitchEffect::ShowFlip { session: 1 },
                TaskSwitchEffect::RequestPreviews(vec!["chrome-b", "chrome-a"]),
                TaskSwitchEffect::SelectPreview("chrome-a"),
            ]
        );
        assert_eq!(
            switcher.apply(HotkeyAction::CommitSwitch, &windows),
            [
                TaskSwitchEffect::HideFlip { session: 1 },
                TaskSwitchEffect::ActivateWindow("chrome-a"),
            ]
        );
    }

    #[test]
    fn candidate_removal_updates_previews_selection_and_empty_session() {
        let windows = [
            window("current", "editor", true),
            window("selected", "browser", false),
            window("other", "terminal", false),
        ];
        let mut switcher = TaskSwitcher::default();
        let _ = switcher.apply(HotkeyAction::SwitchNext, &windows);

        assert_eq!(
            switcher.remove_candidate(&"selected"),
            [
                TaskSwitchEffect::RequestPreviews(vec!["current", "other"]),
                TaskSwitchEffect::SelectPreview("other"),
            ]
        );
        assert_eq!(
            switcher.remove_candidate(&"current"),
            [TaskSwitchEffect::RequestPreviews(vec!["other"])]
        );
        assert_eq!(
            switcher.remove_candidate(&"other"),
            [TaskSwitchEffect::HideFlip { session: 1 }]
        );
        assert_eq!(switcher.session(), None);
        assert!(switcher.apply(HotkeyAction::CommitSwitch, &[]).is_empty());
    }
}
