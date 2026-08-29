//! Production-authoritative fluent scenarios for shell behavior tests.

use crate::{
    focus::{FocusRequest, FocusTransactions},
    hotkeys::{Hotkey, HotkeyAction, HotkeyController, KeyEdge},
    launcher::{
        LauncherActivation, LauncherActivationSource, LauncherPointerTarget,
        LauncherSemanticTarget, LauncherTransition, LauncherVisibility,
    },
    output_layout::OutputLayout,
    task_switcher::{SwitchWindow, TaskSwitchEffect, TaskSwitcher},
    window_input::{
        PointerPosition, WindowGeometry, WindowPointerEffect, WindowSurface, hit_test,
        reduce_pointer_press, resolve_semantic_target,
    },
    workspaces::{WorkspaceDirection, WorkspaceId, WorkspaceTransition, Workspaces},
};
use std::{collections::HashMap, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    AltTab,
    AltShiftTab,
    AltBacktick,
    AltShiftBacktick,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Surface {
    Launcher,
    Flip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClickTarget {
    PanelLauncher,
    LauncherBackground,
    Desktop,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SurfaceIdentity(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LauncherEffect {
    ShowSurface(SurfaceIdentity),
    HideSurface(SurfaceIdentity),
    RequestFocus(FocusRequest<SurfaceIdentity>),
    RestoreWindowFocus(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordedEffect {
    Launcher(LauncherEffect),
    Task(TaskSwitchEffect<String>),
    Workspace(WorkspaceEffect),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceEffect {
    HideWindow(String),
    ShowWindow(String),
    ActivateWindow(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectAcknowledgement {
    pub effect: String,
    pub acknowledged: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityRecord {
    pub field: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq)]
struct WindowFact {
    name: String,
    application_id: String,
    active: bool,
    geometry: Option<WindowGeometry>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordingPlatform {
    effects: Vec<TaskSwitchEffect<String>>,
    active_window: Option<String>,
    visible_flip_session: Option<u64>,
    launcher_effects: Vec<LauncherEffect>,
    visible_launcher: Option<SurfaceIdentity>,
    launcher_output: Option<String>,
    acknowledgements: Vec<EffectAcknowledgement>,
    ordered_effects: Vec<RecordedEffect>,
}

impl RecordingPlatform {
    fn apply(&mut self, effect: TaskSwitchEffect<String>) {
        match &effect {
            TaskSwitchEffect::ShowFlip { session } => {
                self.visible_flip_session = Some(*session);
            }
            TaskSwitchEffect::HideFlip { session }
                if self.visible_flip_session == Some(*session) =>
            {
                self.visible_flip_session = None;
            }
            TaskSwitchEffect::ActivateWindow(window) => {
                self.active_window = Some(window.clone());
            }
            TaskSwitchEffect::RequestPreviews(_) | TaskSwitchEffect::SelectPreview(_) => {}
            TaskSwitchEffect::HideFlip { .. } => {}
        }
        self.ordered_effects
            .push(RecordedEffect::Task(effect.clone()));
        self.effects.push(effect);
    }

    pub fn effects(&self) -> &[TaskSwitchEffect<String>] {
        &self.effects
    }

    pub fn launcher_effects(&self) -> &[LauncherEffect] {
        &self.launcher_effects
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScenarioBudget {
    pub events: usize,
    pub effects: usize,
    pub redraws: usize,
}

impl Default for ScenarioBudget {
    fn default() -> Self {
        Self {
            events: 32,
            effects: 32,
            redraws: 8,
        }
    }
}

pub struct Scenario {
    name: String,
    windows: Vec<WindowFact>,
    hotkeys: HotkeyController,
    switcher: TaskSwitcher<String>,
    platform: RecordingPlatform,
    trace: Vec<String>,
    budget: ScenarioBudget,
    consumed_events: usize,
    consumed_effects: usize,
    redraws: usize,
    now: Duration,
    launcher: LauncherVisibility,
    launcher_identity: SurfaceIdentity,
    focus: FocusTransactions<SurfaceIdentity>,
    captured_surfaces: HashMap<String, SurfaceIdentity>,
    captured_focus: HashMap<String, FocusRequest<SurfaceIdentity>>,
    outputs: OutputLayout,
    task_reduce: TaskReducer,
    window_target_resolver: WindowTargetResolver,
    actions: Vec<HotkeyAction>,
    workspaces: Workspaces<String>,
    workspace_names: HashMap<String, WorkspaceId>,
    authority: Vec<AuthorityRecord>,
    state_before_event: String,
}

type TaskReducer = fn(
    &mut TaskSwitcher<String>,
    HotkeyAction,
    &[SwitchWindow<String>],
) -> Vec<TaskSwitchEffect<String>>;

type WindowTargetResolver = fn(&[WindowSurface<String>], &str) -> Option<PointerPosition>;

fn production_task_reduce(
    switcher: &mut TaskSwitcher<String>,
    action: HotkeyAction,
    windows: &[SwitchWindow<String>],
) -> Vec<TaskSwitchEffect<String>> {
    switcher.apply(action, windows)
}

fn production_window_target_resolver(
    surfaces: &[WindowSurface<String>],
    name: &str,
) -> Option<PointerPosition> {
    resolve_semantic_target(surfaces, &name.to_owned())
}

pub struct WindowCursor {
    scenario: Scenario,
    current: usize,
}

pub fn scenario(name: impl Into<String>) -> Scenario {
    Scenario {
        name: name.into(),
        windows: Vec::new(),
        hotkeys: HotkeyController::default(),
        switcher: TaskSwitcher::default(),
        platform: RecordingPlatform::default(),
        trace: Vec::new(),
        budget: ScenarioBudget::default(),
        consumed_events: 0,
        consumed_effects: 0,
        redraws: 0,
        now: Duration::ZERO,
        launcher: LauncherVisibility::Hidden,
        launcher_identity: SurfaceIdentity(1),
        focus: FocusTransactions::default(),
        captured_surfaces: HashMap::new(),
        captured_focus: HashMap::new(),
        outputs: OutputLayout::default(),
        task_reduce: production_task_reduce,
        window_target_resolver: production_window_target_resolver,
        actions: Vec::new(),
        workspaces: Workspaces::default(),
        workspace_names: HashMap::from([("main".into(), WorkspaceId(1))]),
        authority: Vec::new(),
        state_before_event: "initial scenario state".into(),
    }
}

impl Scenario {
    pub fn budget(mut self, budget: ScenarioBudget) -> Self {
        self.budget = budget;
        self
    }

    pub fn window(mut self, name: impl Into<String>) -> WindowCursor {
        let name = name.into();
        self.workspaces.add_window(name.clone());
        self.windows.push(WindowFact {
            name,
            application_id: String::new(),
            active: false,
            geometry: None,
        });
        let current = self.windows.len() - 1;
        WindowCursor {
            scenario: self,
            current,
        }
    }

    pub fn output(
        mut self,
        name: impl Into<String>,
        width: i32,
        height: i32,
        priority: u8,
    ) -> Self {
        self.outputs.connect(name.into(), width, height, priority);
        self
    }

    pub fn press(mut self, key: Key) -> Self {
        self.trace.push(format!("press {key:?}"));
        let sequence: &[(Hotkey, KeyEdge)] = match key {
            Key::AltTab => &[
                (Hotkey::Alt, KeyEdge::Pressed),
                (Hotkey::Tab, KeyEdge::Pressed),
                (Hotkey::Tab, KeyEdge::Released),
                (Hotkey::Alt, KeyEdge::Released),
            ],
            Key::AltShiftTab => &[
                (Hotkey::Alt, KeyEdge::Pressed),
                (Hotkey::Shift, KeyEdge::Pressed),
                (Hotkey::Tab, KeyEdge::Pressed),
                (Hotkey::Tab, KeyEdge::Released),
                (Hotkey::Shift, KeyEdge::Released),
                (Hotkey::Alt, KeyEdge::Released),
            ],
            Key::AltBacktick => &[
                (Hotkey::Alt, KeyEdge::Pressed),
                (Hotkey::Grave, KeyEdge::Pressed),
                (Hotkey::Grave, KeyEdge::Released),
                (Hotkey::Alt, KeyEdge::Released),
            ],
            Key::AltShiftBacktick => &[
                (Hotkey::Alt, KeyEdge::Pressed),
                (Hotkey::Shift, KeyEdge::Pressed),
                (Hotkey::Grave, KeyEdge::Pressed),
                (Hotkey::Grave, KeyEdge::Released),
                (Hotkey::Shift, KeyEdge::Released),
                (Hotkey::Alt, KeyEdge::Released),
            ],
        };
        for (key, edge) in sequence {
            self.consume_event(format!("{key:?} {edge:?}"));
            let outcome = self.hotkeys.handle(*key, *edge);
            if let Some(action) = outcome.action {
                self.apply_hotkey_action(action);
            }
        }
        self
    }

    pub fn key_edge(mut self, key: Hotkey, edge: KeyEdge) -> Self {
        self.consume_event(format!("{key:?} {edge:?}"));
        let outcome = self.hotkeys.handle(key, edge);
        if let Some(action) = outcome.action {
            self.apply_hotkey_action(action);
        }
        self
    }

    pub fn click(mut self, target: ClickTarget) -> Self {
        self.consume_event(format!("pointer click {target:?}"));
        match target {
            ClickTarget::PanelLauncher => self.toggle_launcher(None),
            ClickTarget::LauncherBackground => {
                let _ = self.launcher.pointer_press(LauncherPointerTarget::Launcher);
            }
            ClickTarget::Desktop => {
                if self.launcher.pointer_press(LauncherPointerTarget::Other) {
                    self.hide_launcher(true);
                }
            }
        }
        self
    }

    pub fn click_window(mut self, window: &str) -> Self {
        self.consume_event(format!("pointer click window {window:?}"));
        let surfaces = self.production_window_surfaces();
        let position = (self.window_target_resolver)(&surfaces, window).unwrap_or_else(|| {
            panic!(
                "scenario {:?} has no pointer geometry for window {window:?}",
                self.name
            )
        });
        let resolved = hit_test(&surfaces, position);
        if self.launcher.pointer_press(LauncherPointerTarget::Other) {
            self.hide_launcher(false);
        }
        for effect in reduce_pointer_press(resolved) {
            match effect {
                WindowPointerEffect::ActivateWindow(window) => {
                    let effect = TaskSwitchEffect::ActivateWindow(window);
                    self.consumed_effects += 1;
                    self.authority.push(AuthorityRecord {
                        field: "window.active".into(),
                        path: format!(
                            "semantic named window -> production WindowSurface geometry -> hit_test({position:?}) -> reduce_pointer_press -> {effect:?} -> RecordingPlatform"
                        ),
                    });
                    self.platform.acknowledgements.push(EffectAcknowledgement {
                        effect: format!("{effect:?}"),
                        acknowledged: true,
                    });
                    self.platform.apply(effect);
                }
            }
        }
        self.check_budget();
        self
    }

    /// Applies a production window-lifecycle removal while the scenario is active.
    pub fn remove_window(mut self, window: &str) -> Self {
        self.consume_event(format!("window removed {window:?}"));
        self.workspaces.remove_window(&window.to_owned());
        let before = self.windows.len();
        self.windows.retain(|candidate| candidate.name != window);
        assert_eq!(
            self.windows.len(),
            before.saturating_sub(1),
            "scenario {:?} has no window {window:?}",
            self.name
        );
        let effects = self.switcher.remove_candidate(&window.to_owned());
        self.apply_task_effects(
            effects,
            format!("production window lifecycle removal {window:?} -> TaskSwitcher"),
        );
        self
    }

    pub fn activate(mut self, source: LauncherActivationSource, target: ClickTarget) -> Self {
        self.consume_event(format!("{source:?} activate {target:?}"));
        let target = match target {
            ClickTarget::PanelLauncher => LauncherSemanticTarget::PanelButton,
            ClickTarget::LauncherBackground => LauncherSemanticTarget::Surface,
            ClickTarget::Desktop => LauncherSemanticTarget::Desktop,
        };
        let transition = self
            .launcher
            .activate(LauncherActivation { source, target });
        self.apply_launcher_transition(transition, None);
        self
    }

    pub fn click_panel_launcher_on(mut self, output: &str) -> Self {
        self.consume_event(format!("pointer click PanelLauncher on {output:?}"));
        assert!(
            self.outputs
                .outputs()
                .iter()
                .any(|candidate| candidate.name == output),
            "scenario {:?} has no output {output:?}",
            self.name
        );
        self.toggle_launcher(Some(output.to_owned()));
        self
    }

    pub fn disconnect_output(mut self, output: &str) -> Self {
        self.consume_event(format!("output disconnected {output:?}"));
        self.outputs.disconnect(output);
        self
    }

    pub fn create_workspace(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        assert!(
            !self.workspace_names.contains_key(&name),
            "scenario {:?} already has workspace {name:?}",
            self.name
        );
        let id = self
            .workspaces
            .create()
            .expect("scenario workspace limit exceeded");
        self.workspace_names.insert(name, id);
        self
    }

    pub fn switch_workspace(mut self, name: &str, output: &str) -> Self {
        self.consume_event(format!("switch workspace {name:?} on {output:?}"));
        assert!(
            self.outputs
                .outputs()
                .iter()
                .any(|item| item.name == output),
            "scenario {:?} has no output {output:?}",
            self.name
        );
        let id = *self
            .workspace_names
            .get(name)
            .unwrap_or_else(|| panic!("scenario {:?} has no workspace {name:?}", self.name));
        let transition = self
            .workspaces
            .switch_to(id, Some(output.to_owned()))
            .expect("named workspace was validated");
        self.apply_workspace_transition(transition, "Workspaces::switch_to");
        self
    }

    pub fn move_window_to_workspace(mut self, window: &str, workspace: &str) -> Self {
        self.consume_event(format!("move window {window:?} to workspace {workspace:?}"));
        let id = *self
            .workspace_names
            .get(workspace)
            .unwrap_or_else(|| panic!("scenario {:?} has no workspace {workspace:?}", self.name));
        let transition = self
            .workspaces
            .move_window(&window.to_owned(), id)
            .unwrap_or_else(|_| panic!("scenario {:?} has no window {window:?}", self.name));
        self.apply_workspace_transition(transition, "Workspaces::move_window");
        self
    }

    pub fn remove_workspace(mut self, name: &str) -> Self {
        self.consume_event(format!("remove workspace {name:?}"));
        let id = self
            .workspace_names
            .remove(name)
            .unwrap_or_else(|| panic!("scenario {:?} has no workspace {name:?}", self.name));
        let transition = self
            .workspaces
            .remove(id)
            .expect("scenario must retain one workspace");
        self.apply_workspace_transition(transition, "Workspaces::remove");
        self
    }

    pub fn expect_workspace(self, expected: &str) -> Self {
        let actual = self
            .workspace_names
            .iter()
            .find_map(|(name, id)| (*id == self.workspaces.active()).then(|| name.clone()));
        self.assert(
            actual.as_deref() == Some(expected),
            format!("expected active workspace {expected:?}, got {actual:?}"),
        )
    }

    pub fn expect_window_visible(self, window: &str, visible: bool) -> Self {
        let actual = self.workspaces.is_visible(&window.to_owned());
        self.assert(
            actual == visible,
            format!("expected window {window:?} visibility {visible}, got {actual}"),
        )
    }

    pub fn expect_output_position(self, output: &str, x: i32, y: i32) -> Self {
        let actual = self
            .outputs
            .outputs()
            .iter()
            .find(|candidate| candidate.name == output)
            .map(|candidate| (candidate.x, candidate.y));
        self.assert(
            actual == Some((x, y)),
            format!("expected output {output:?} at ({x}, {y}), got {actual:?}"),
        )
    }

    pub fn expect_launcher_output(self, expected: &str) -> Self {
        let actual = self.platform.launcher_output.clone();
        let condition = actual.as_deref() == Some(expected);
        self.assert(
            condition,
            format!("expected launcher on {expected:?}, got {actual:?}"),
        )
    }

    pub fn acknowledge_current_focus(mut self) -> Self {
        self.consume_event("acknowledge current focus".into());
        if let Some(request) = self.focus.requested().cloned() {
            let _ = self.focus.acknowledge(&request);
        }
        self
    }

    pub fn expect_visible_windows(self, expected: &[&str]) -> Self {
        let actual = self
            .production_window_facts()
            .into_iter()
            .map(|window| window.id)
            .collect::<Vec<_>>();
        let expected = expected
            .iter()
            .map(|window| (*window).to_owned())
            .collect::<Vec<_>>();
        self.assert(
            actual == expected,
            format!("expected visible window feed {expected:?}, got {actual:?}"),
        )
    }

    pub fn lose_focus(mut self, request: FocusRequest<SurfaceIdentity>) -> Self {
        self.consume_event(format!("focus lost {:?}", request.transaction));
        if self.focus.loses_current(&request)
            && self.launcher.pointer_press(LauncherPointerTarget::Other)
        {
            self.hide_launcher(true);
        }
        self
    }

    pub fn current_focus_request(&self) -> Option<FocusRequest<SurfaceIdentity>> {
        self.focus.requested().cloned()
    }

    pub fn capture_focus(mut self, label: impl Into<String>) -> Self {
        let request = self.focus.requested().cloned().unwrap_or_else(|| {
            panic!("scenario {:?} has no requested focus to capture", self.name)
        });
        self.captured_focus.insert(label.into(), request);
        self
    }

    pub fn lose_captured_focus(mut self, label: &str) -> Self {
        let request = self.captured_focus.get(label).cloned().unwrap_or_else(|| {
            panic!(
                "scenario {:?} has no captured focus named {label:?}",
                self.name
            )
        });
        self.consume_event(format!("focus lost {:?}", request.transaction));
        if self.focus.loses_current(&request)
            && self.launcher.pointer_press(LauncherPointerTarget::Other)
        {
            self.hide_launcher(true);
        }
        self
    }

    pub fn capture_surface(mut self, label: impl Into<String>, surface: Surface) -> Self {
        let identity = self.surface_identity(surface).unwrap_or_else(|| {
            panic!(
                "scenario {:?} cannot capture hidden surface {surface:?}",
                self.name
            )
        });
        self.captured_surfaces.insert(label.into(), identity);
        self
    }

    pub fn expect_same_surface(self, label: &str, surface: Surface) -> Self {
        let captured = self.captured_surfaces.get(label).copied();
        let current = self.surface_identity(surface);
        let condition = captured.is_some() && captured == current;
        self.assert(
            condition,
            format!("expected {surface:?} identity {captured:?}, got {current:?}"),
        )
    }

    pub fn expect_new_surface(self, label: &str, surface: Surface) -> Self {
        let captured = self.captured_surfaces.get(label).copied();
        let current = self.surface_identity(surface);
        self.assert(
            captured.is_some() && current.is_some() && captured != current,
            format!("expected a new {surface:?} identity after {captured:?}, got {current:?}"),
        )
    }

    pub fn expect_visible(self, surface: Surface) -> Self {
        let visible = self.surface_identity(surface).is_some();
        self.assert(visible, format!("expected {surface:?} visible"))
    }

    pub fn expect_hidden(self, surface: Surface) -> Self {
        let hidden = self.surface_identity(surface).is_none();
        self.assert(hidden, format!("expected {surface:?} hidden"))
    }

    pub fn expect_stable_for(mut self, duration: Duration) -> Self {
        let launcher = self.launcher;
        let task_session = self.switcher.session();
        let launcher_effects = self.platform.launcher_effects.len();
        let task_effects = self.platform.effects.len();
        let redraws = self.redraws;
        self.now += duration;
        let stable = self.launcher == launcher
            && self.switcher.session() == task_session
            && self.platform.launcher_effects.len() == launcher_effects
            && self.platform.effects.len() == task_effects
            && self.redraws == redraws;
        self.assert(
            stable,
            format!("state changed while stable for {duration:?}"),
        )
    }

    pub fn expect_active(self, window: &str) -> Self {
        let condition = self.platform.active_window.as_deref() == Some(window);
        let detail = format!(
            "expected active window {window:?}, got {:?}",
            self.platform.active_window
        );
        self.assert(condition, detail)
    }

    pub fn expect_flip_hidden(self) -> Self {
        let condition = self.platform.visible_flip_session.is_none();
        let detail = format!(
            "expected Flip hidden, session {:?} remains visible",
            self.platform.visible_flip_session
        );
        self.assert(condition, detail)
    }

    pub fn expect_effects(self, expected: &[TaskSwitchEffect<String>]) -> Self {
        let condition = self.platform.effects == expected;
        let detail = format!(
            "effect sequence differed\nexpected: {expected:#?}\nactual: {:#?}",
            self.platform.effects
        );
        self.assert(condition, detail)
    }

    pub fn expect_actions(self, expected: &[HotkeyAction]) -> Self {
        let condition = self.actions == expected;
        let detail = format!(
            "hotkey action sequence differed\nexpected: {expected:#?}\nactual: {:#?}",
            self.actions
        );
        self.assert(condition, detail)
    }

    pub fn expect_launcher_effects(self, expected: &[LauncherEffect]) -> Self {
        let condition = self.platform.launcher_effects == expected;
        let detail = format!(
            "launcher effect sequence differed\nexpected: {expected:#?}\nactual: {:#?}",
            self.platform.launcher_effects
        );
        self.assert(condition, detail)
    }

    pub fn expect_ordered_effects(self, expected: &[RecordedEffect]) -> Self {
        let condition = self.platform.ordered_effects == expected;
        let detail = format!(
            "ordered effect sequence differed\nexpected: {expected:#?}\nactual: {:#?}",
            self.platform.ordered_effects
        );
        self.assert(condition, detail)
    }

    pub fn expect_authority_path(self, field: &str, required: &[&str]) -> Self {
        let matching = self.authority.iter().any(|record| {
            record.field == field && required.iter().all(|part| record.path.contains(part))
        });
        let detail = format!(
            "expected authority field {field:?} containing {required:?}, got {:#?}",
            self.authority
        );
        self.assert(matching, detail)
    }

    pub fn expect_within_budget(self) -> Self {
        let condition = self.consumed_events <= self.budget.events
            && self.consumed_effects <= self.budget.effects
            && self.redraws <= self.budget.redraws;
        self.assert(
            condition,
            "scenario exhausted its event, effect, or redraw budget".into(),
        )
    }

    pub fn platform(&self) -> &RecordingPlatform {
        &self.platform
    }

    fn apply_hotkey_action(&mut self, action: HotkeyAction) {
        self.actions.push(action);
        let workspace_transition = match action {
            HotkeyAction::SwitchWorkspacePrevious | HotkeyAction::SwitchWorkspaceNext => {
                let direction = if action == HotkeyAction::SwitchWorkspacePrevious {
                    WorkspaceDirection::Previous
                } else {
                    WorkspaceDirection::Next
                };
                let target = self.workspaces.neighbor(direction);
                let output = self
                    .workspaces
                    .active_output()
                    .map(str::to_owned)
                    .or_else(|| {
                        self.outputs
                            .outputs()
                            .first()
                            .map(|output| output.name.clone())
                    });
                Some(self.workspaces.switch_to(target, output))
            }
            HotkeyAction::MoveWindowToPreviousWorkspace
            | HotkeyAction::MoveWindowToNextWorkspace => {
                let direction = if action == HotkeyAction::MoveWindowToPreviousWorkspace {
                    WorkspaceDirection::Previous
                } else {
                    WorkspaceDirection::Next
                };
                self.platform.active_window.clone().map(|window| {
                    let target = self.workspaces.neighbor(direction);
                    self.workspaces.move_window(&window, target)
                })
            }
            _ => None,
        };
        if let Some(transition) = workspace_transition {
            self.apply_workspace_transition(
                transition.expect("directional workspace target must remain valid"),
                "HotkeyController -> Workspaces directional reducer",
            );
            return;
        }
        let windows = self.production_window_facts();
        let effects = (self.task_reduce)(&mut self.switcher, action, &windows);
        self.apply_task_effects(
            effects,
            format!("semantic key -> HotkeyController -> {action:?} -> TaskSwitcher"),
        );
    }

    fn apply_workspace_transition(
        &mut self,
        transition: WorkspaceTransition<String>,
        authority: &str,
    ) {
        let effects = transition
            .hide
            .into_iter()
            .map(WorkspaceEffect::HideWindow)
            .chain(transition.show.into_iter().map(WorkspaceEffect::ShowWindow))
            .chain(
                transition
                    .focus
                    .into_iter()
                    .map(WorkspaceEffect::ActivateWindow),
            );
        for effect in effects {
            self.consumed_effects += 1;
            self.authority.push(AuthorityRecord {
                field: match effect {
                    WorkspaceEffect::HideWindow(_) | WorkspaceEffect::ShowWindow(_) => {
                        "window.workspace.visible"
                    }
                    WorkspaceEffect::ActivateWindow(_) => "window.active",
                }
                .into(),
                path: format!("semantic workspace action -> {authority} -> {effect:?}"),
            });
            if let WorkspaceEffect::ActivateWindow(window) = &effect {
                self.platform.active_window = Some(window.clone());
            }
            self.platform
                .ordered_effects
                .push(RecordedEffect::Workspace(effect));
        }
        self.check_budget();
    }

    fn apply_task_effects(
        &mut self,
        effects: Vec<TaskSwitchEffect<String>>,
        authority_path: String,
    ) {
        for effect in effects {
            self.consumed_effects += 1;
            if matches!(
                effect,
                TaskSwitchEffect::ShowFlip { .. }
                    | TaskSwitchEffect::SelectPreview(_)
                    | TaskSwitchEffect::HideFlip { .. }
            ) {
                self.redraws += 1;
            }
            self.authority.push(AuthorityRecord {
                field: match &effect {
                    TaskSwitchEffect::ShowFlip { .. } | TaskSwitchEffect::HideFlip { .. } => {
                        "surface.flip.visible"
                    }
                    TaskSwitchEffect::RequestPreviews(_) => "surface.flip.previews",
                    TaskSwitchEffect::SelectPreview(_) => "surface.flip.selection",
                    TaskSwitchEffect::ActivateWindow(_) => "window.active",
                }
                .into(),
                path: format!("{authority_path} -> {effect:?} -> RecordingPlatform"),
            });
            self.platform.acknowledgements.push(EffectAcknowledgement {
                effect: format!("{effect:?}"),
                acknowledged: true,
            });
            self.platform.apply(effect);
        }
        self.check_budget();
    }

    fn toggle_launcher(&mut self, output: Option<String>) {
        let transition = self.launcher.activate(LauncherActivation {
            source: LauncherActivationSource::Pointer,
            target: LauncherSemanticTarget::PanelButton,
        });
        self.apply_launcher_transition(transition, output);
    }

    fn apply_launcher_transition(
        &mut self,
        transition: LauncherTransition,
        output: Option<String>,
    ) {
        if transition == LauncherTransition::Shown {
            self.platform.visible_launcher = Some(self.launcher_identity);
            self.platform.launcher_output = output;
            self.record_launcher_effect(LauncherEffect::ShowSurface(self.launcher_identity));
            let request = self.focus.request(self.launcher_identity);
            self.record_launcher_effect(LauncherEffect::RequestFocus(request));
        } else if transition == LauncherTransition::Hidden {
            self.hide_launcher(true);
        }
    }

    fn hide_launcher(&mut self, restore_window_focus: bool) {
        self.platform.visible_launcher = None;
        self.platform.launcher_output = None;
        self.record_launcher_effect(LauncherEffect::HideSurface(self.launcher_identity));
        if restore_window_focus && let Some(window) = self.platform.active_window.clone() {
            self.record_launcher_effect(LauncherEffect::RestoreWindowFocus(window));
        }
    }

    fn record_launcher_effect(&mut self, effect: LauncherEffect) {
        self.consumed_effects += 1;
        self.redraws += usize::from(matches!(
            effect,
            LauncherEffect::ShowSurface(_) | LauncherEffect::HideSurface(_)
        ));
        self.authority.push(AuthorityRecord {
            field: match &effect {
                LauncherEffect::ShowSurface(_) | LauncherEffect::HideSurface(_) => {
                    "surface.launcher.visible"
                }
                LauncherEffect::RequestFocus(_) => "focus.requested",
                LauncherEffect::RestoreWindowFocus(_) => "window.focus.restored",
            }
            .into(),
            path: format!(
                "semantic activation -> LauncherVisibility -> {effect:?} -> RecordingPlatform"
            ),
        });
        self.platform.acknowledgements.push(EffectAcknowledgement {
            effect: format!("{effect:?}"),
            acknowledged: true,
        });
        if let LauncherEffect::RestoreWindowFocus(window) = &effect {
            self.platform.active_window = Some(window.clone());
        }
        self.platform
            .ordered_effects
            .push(RecordedEffect::Launcher(effect.clone()));
        self.platform.launcher_effects.push(effect);
        self.check_budget();
    }

    fn surface_identity(&self, surface: Surface) -> Option<SurfaceIdentity> {
        match surface {
            Surface::Launcher => self.platform.visible_launcher,
            Surface::Flip => self.platform.visible_flip_session.map(SurfaceIdentity),
        }
    }

    fn production_window_facts(&self) -> Vec<SwitchWindow<String>> {
        let mut windows = self
            .windows
            .iter()
            .filter(|window| self.workspaces.is_visible(&window.name))
            .filter(|window| window.active)
            .collect::<Vec<_>>();
        windows.extend(
            self.windows
                .iter()
                .filter(|window| self.workspaces.is_visible(&window.name))
                .filter(|window| !window.active),
        );
        windows
            .into_iter()
            .map(|window| SwitchWindow {
                id: window.name.clone(),
                application_id: window.application_id.clone(),
                active: window.active,
            })
            .collect()
    }

    fn production_window_surfaces(&self) -> Vec<WindowSurface<String>> {
        self.windows
            .iter()
            .filter(|window| self.workspaces.is_visible(&window.name))
            .filter_map(|window| {
                window.geometry.map(|geometry| WindowSurface {
                    id: window.name.clone(),
                    geometry,
                })
            })
            .collect()
    }

    fn consume_event(&mut self, event: String) {
        self.state_before_event = self.diagnostic_state();
        self.consumed_events += 1;
        self.trace.push(event);
        self.check_budget();
    }

    fn check_budget(&self) {
        assert!(
            self.consumed_events <= self.budget.events
                && self.consumed_effects <= self.budget.effects
                && self.redraws <= self.budget.redraws,
            "{} exceeded budget {:?}; consumed events={}, effects={}, redraws={}\ntrace:\n{}\nauthority:\n{}",
            self.name,
            self.budget,
            self.consumed_events,
            self.consumed_effects,
            self.redraws,
            self.numbered_trace(),
            self.authority_trace()
        );
    }

    fn assert(mut self, condition: bool, detail: String) -> Self {
        if !condition {
            self.trace.push(format!("assertion failed: {detail}"));
            panic!(
                "scenario {:?} failed: {detail}\nconsumed events={}, effects={}, redraws={}\nbefore last event: {}\nafter: {}\ntrace:\n{}\nauthority:\n{}\nacknowledgements: {:#?}",
                self.name,
                self.consumed_events,
                self.consumed_effects,
                self.redraws,
                self.state_before_event,
                self.diagnostic_state(),
                self.numbered_trace(),
                self.authority_trace(),
                self.platform.acknowledgements
            );
        }
        self
    }

    fn numbered_trace(&self) -> String {
        self.trace
            .iter()
            .enumerate()
            .map(|(index, event)| format!("{}. {event}", index + 1))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn authority_trace(&self) -> String {
        self.authority
            .iter()
            .map(|record| format!("{} <- {}", record.field, record.path))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn diagnostic_state(&self) -> String {
        format!(
            "active={:?}, visible_launcher={:?}, visible_flip={:?}, focus_requested={:?}, focus_acknowledged={:?}, launcher_output={:?}, virtual_time={:?}, pending_timers=[]",
            self.platform.active_window,
            self.platform.visible_launcher,
            self.platform.visible_flip_session,
            self.focus.requested(),
            self.focus.acknowledged(),
            self.platform.launcher_output,
            self.now,
        )
    }
}

impl WindowCursor {
    pub fn app(mut self, application_id: impl Into<String>) -> Self {
        self.scenario.windows[self.current].application_id = application_id.into();
        self
    }

    pub fn active(mut self) -> Self {
        for window in &mut self.scenario.windows {
            window.active = false;
        }
        self.scenario.windows[self.current].active = true;
        self.scenario.platform.active_window =
            Some(self.scenario.windows[self.current].name.clone());
        self.scenario
            .workspaces
            .focused(&self.scenario.windows[self.current].name);
        self
    }

    pub fn bounds(mut self, x: f64, y: f64, width: f64, height: f64) -> Self {
        self.scenario.windows[self.current].geometry = Some(WindowGeometry {
            x,
            y,
            width,
            height,
        });
        self
    }

    pub fn window(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.scenario.workspaces.add_window(name.clone());
        self.scenario.windows.push(WindowFact {
            name,
            application_id: String::new(),
            active: false,
            geometry: None,
        });
        self.current = self.scenario.windows.len() - 1;
        self
    }

    pub fn press(self, key: Key) -> Scenario {
        self.scenario.press(key)
    }

    pub fn key_edge(self, key: Hotkey, edge: KeyEdge) -> Scenario {
        self.scenario.key_edge(key, edge)
    }

    pub fn click(self, target: ClickTarget) -> Scenario {
        self.scenario.click(target)
    }

    pub fn click_window(self, window: &str) -> Scenario {
        self.scenario.click_window(window)
    }

    pub fn create_workspace(self, name: impl Into<String>) -> Scenario {
        self.scenario.create_workspace(name)
    }

    pub fn switch_workspace(self, name: &str, output: &str) -> Scenario {
        self.scenario.switch_workspace(name, output)
    }

    pub fn move_window_to_workspace(self, window: &str, workspace: &str) -> Scenario {
        self.scenario.move_window_to_workspace(window, workspace)
    }

    pub fn expect_visible_windows(self, expected: &[&str]) -> Scenario {
        self.scenario.expect_visible_windows(expected)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Key, ScenarioBudget, Surface, scenario};
    use crate::hotkeys::{Hotkey, HotkeyAction, KeyEdge};
    use crate::task_switcher::TaskSwitchEffect;
    use crate::task_switcher::{SwitchWindow, TaskSwitcher};
    use crate::window_input::{PointerPosition, WindowSurface};

    #[test]
    fn flip_commits_exact_effects_and_allocates_a_new_session() {
        let first = scenario("modifier release commits one flip session")
            .window("a")
            .app("one")
            .active()
            .window("b")
            .app("two")
            .key_edge(Hotkey::Alt, KeyEdge::Pressed)
            .key_edge(Hotkey::Tab, KeyEdge::Pressed)
            .capture_surface("first", Surface::Flip)
            .expect_visible(Surface::Flip)
            .key_edge(Hotkey::Tab, KeyEdge::Released)
            .key_edge(Hotkey::Alt, KeyEdge::Released);

        let second = first
            .expect_actions(&[HotkeyAction::SwitchNext, HotkeyAction::CommitSwitch])
            .expect_effects(&[
                TaskSwitchEffect::ShowFlip { session: 1 },
                TaskSwitchEffect::RequestPreviews(vec!["a".into(), "b".into()]),
                TaskSwitchEffect::SelectPreview("b".into()),
                TaskSwitchEffect::HideFlip { session: 1 },
                TaskSwitchEffect::ActivateWindow("b".into()),
            ])
            .expect_active("b")
            .expect_hidden(Surface::Flip)
            .key_edge(Hotkey::Alt, KeyEdge::Pressed)
            .key_edge(Hotkey::Tab, KeyEdge::Pressed)
            .expect_new_surface("first", Surface::Flip)
            .key_edge(Hotkey::Tab, KeyEdge::Released)
            .key_edge(Hotkey::Alt, KeyEdge::Released);

        second
            .expect_actions(&[
                HotkeyAction::SwitchNext,
                HotkeyAction::CommitSwitch,
                HotkeyAction::SwitchNext,
                HotkeyAction::CommitSwitch,
            ])
            .expect_effects(&[
                TaskSwitchEffect::ShowFlip { session: 1 },
                TaskSwitchEffect::RequestPreviews(vec!["a".into(), "b".into()]),
                TaskSwitchEffect::SelectPreview("b".into()),
                TaskSwitchEffect::HideFlip { session: 1 },
                TaskSwitchEffect::ActivateWindow("b".into()),
                TaskSwitchEffect::ShowFlip { session: 2 },
                TaskSwitchEffect::RequestPreviews(vec!["a".into(), "b".into()]),
                TaskSwitchEffect::SelectPreview("b".into()),
                TaskSwitchEffect::HideFlip { session: 2 },
                TaskSwitchEffect::ActivateWindow("b".into()),
            ])
            .expect_active("b")
            .expect_hidden(Surface::Flip);
    }

    #[test]
    #[should_panic(expected = "exceeded budget")]
    fn hard_event_budget_fails_during_dispatch() {
        scenario("budget admission")
            .budget(ScenarioBudget {
                events: 1,
                effects: 32,
                redraws: 8,
            })
            .window("a")
            .active()
            .window("b")
            .press(Key::AltTab);
    }

    #[test]
    #[should_panic(expected = "exceeded budget")]
    fn hard_effect_budget_fails_during_dispatch() {
        scenario("effect budget admission")
            .budget(ScenarioBudget {
                events: 32,
                effects: 1,
                redraws: 8,
            })
            .click(super::ClickTarget::PanelLauncher);
    }

    #[test]
    #[should_panic(expected = "exceeded budget")]
    fn hard_redraw_budget_fails_during_dispatch() {
        scenario("redraw budget admission")
            .budget(ScenarioBudget {
                events: 32,
                effects: 32,
                redraws: 0,
            })
            .click(super::ClickTarget::PanelLauncher);
    }

    #[test]
    fn deterministic_stability_advances_time_without_spurious_activity() {
        scenario("stable launcher interval")
            .click(super::ClickTarget::PanelLauncher)
            .expect_stable_for(Duration::from_millis(250))
            .expect_visible(Surface::Launcher)
            .expect_within_budget();
    }

    #[test]
    #[should_panic(expected = "expected active window")]
    fn admission_catches_a_substituted_incorrect_task_reducer() {
        fn incorrect(
            _: &mut TaskSwitcher<String>,
            _: HotkeyAction,
            _: &[SwitchWindow<String>],
        ) -> Vec<TaskSwitchEffect<String>> {
            Vec::new()
        }

        let mut test = scenario("anti-cheating admission");
        test.task_reduce = incorrect;
        test.window("a")
            .active()
            .window("b")
            .press(Key::AltTab)
            .expect_active("b");
    }

    #[test]
    #[should_panic(expected = "expected active window")]
    fn admission_catches_a_substituted_incorrect_window_target_resolver() {
        fn incorrect(_: &[WindowSurface<String>], _: &str) -> Option<PointerPosition> {
            Some(PointerPosition { x: 250.0, y: 50.0 })
        }

        let mut test = scenario("window target anti-cheating admission");
        test.window_target_resolver = incorrect;
        test.window("editor")
            .bounds(0.0, 0.0, 100.0, 100.0)
            .window("terminal")
            .bounds(200.0, 0.0, 100.0, 100.0)
            .active()
            .click_window("editor")
            .expect_active("editor");
    }

    #[test]
    fn failure_diagnostic_names_scenario_trace_authority_and_acknowledgements() {
        let failure = std::panic::catch_unwind(|| {
            scenario("diagnostic oracle")
                .window("a")
                .active()
                .window("b")
                .press(Key::AltTab)
                .expect_active("missing");
        })
        .expect_err("incorrect expectation must fail");
        let message = failure
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| failure.downcast_ref::<&str>().copied())
            .expect("panic has textual diagnostic");
        assert!(message.contains("diagnostic oracle"));
        assert!(message.contains("Alt Pressed"));
        assert!(message.contains("authority:"));
        assert!(message.contains("acknowledgements:"));
        assert!(message.contains("TaskSwitcher"));
        assert!(message.contains("before last event:"));
        assert!(message.contains("visible_flip="));
        assert!(message.contains("pending_timers=[]"));
    }
}
