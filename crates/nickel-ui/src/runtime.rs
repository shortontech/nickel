use std::{
    any::Any,
    error::Error,
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};

use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle},
    window::{CursorIcon, Window, WindowAttributes, WindowId},
};

use crate::{
    AccessibilityNode, ActionKind, Color, ControllerAction, ControllerFamily, ControllerInput,
    DamageRegion, EffectiveHitRoute, FocusedInputDispatcher, FrameRequest,
    FrameResourceDiagnostics, InputCommand, InputContext, InputModality, InputSource,
    InteractionIntent, Invalidation, LayoutDiagnostic, OverlayId, OverlayMenu, PointerIcon, Rect,
    SemanticAction, SemanticActionError, SemanticNodeSnapshot, SemanticQueryError,
    SemanticSelector, SoftwareRenderer, UiEvent, UiFrame, UiId, UiStateStore, View,
};

#[derive(Debug, Default)]
struct PresentScheduler {
    dirty: bool,
    pending: bool,
}

impl PresentScheduler {
    fn invalidate(&mut self) {
        self.dirty = true;
    }

    fn request_present(&mut self) -> bool {
        if !self.dirty || self.pending {
            return false;
        }
        self.pending = true;
        true
    }

    fn begin_present(&mut self) -> bool {
        self.pending = false;
        std::mem::take(&mut self.dirty)
    }
}

fn queue_continuous_input(
    pending: &mut Vec<nickel_input::InputEvent>,
    event: nickel_input::InputEvent,
) {
    let nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion {
        device,
        order,
        position,
        delta,
    }) = event
    else {
        pending.push(event);
        return;
    };
    if let Some(nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion {
        device: queued_device,
        order: queued_order,
        position: queued_position,
        delta: queued_delta,
    })) = pending.last_mut()
        && *queued_device == device
    {
        *queued_order = order;
        *queued_position = position;
        *queued_delta = match (*queued_delta, delta) {
            (Some(previous), Some(next)) => Some(nickel_input::Vector {
                x: previous.x + next.x,
                y: previous.y + next.y,
            }),
            (None, next) => next,
            (previous, None) => previous,
        };
        return;
    }
    pending.push(nickel_input::InputEvent::Pointer(
        nickel_input::PointerEvent::Motion {
            device,
            order,
            position,
            delta,
        },
    ));
}

#[cfg(target_os = "linux")]
fn file_uri_list(paths: &[std::path::PathBuf]) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    let mut payload = Vec::new();
    for path in paths {
        payload.extend_from_slice(b"file://");
        for byte in path.as_os_str().as_bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
                payload.push(*byte);
            } else {
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                payload.extend_from_slice(&[
                    b'%',
                    HEX[(byte >> 4) as usize],
                    HEX[(byte & 15) as usize],
                ]);
            }
        }
        payload.extend_from_slice(b"\r\n");
    }
    payload
}

#[cfg(target_os = "windows")]
fn start_windows_file_drag(paths: &[std::path::PathBuf]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::{
        System::{
            Com::IDataObject,
            Ole::{DROPEFFECT_COPY, DROPEFFECT_MOVE, IDropSource},
        },
        UI::Shell::{ILCreateFromPathW, ILFree, SHCreateDataObject, SHDoDragDrop},
    };
    use windows::core::PCWSTR;
    let wide = paths
        .iter()
        .map(|path| {
            path.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let pidls = wide
        .iter()
        .map(|path| unsafe { ILCreateFromPathW(PCWSTR(path.as_ptr())) })
        .collect::<Vec<_>>();
    if pidls.iter().any(|pidl| pidl.is_null()) {
        return Err("could not create shell drag items".into());
    }
    let pointers = pidls
        .iter()
        .map(|pidl| *pidl as *const _)
        .collect::<Vec<_>>();
    let result = unsafe {
        let data: IDataObject = SHCreateDataObject(None, Some(&pointers), None::<&IDataObject>)
            .map_err(|error| error.to_string())?;
        SHDoDragDrop(
            None,
            &data,
            None::<&IDropSource>,
            DROPEFFECT_COPY | DROPEFFECT_MOVE,
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    };
    for pidl in pidls {
        unsafe { ILFree(Some(pidl.cast())) }
    }
    result
}

fn wait_duration(
    now: Instant,
    deadlines: impl IntoIterator<Item = Option<Instant>>,
) -> Option<Duration> {
    deadlines
        .into_iter()
        .flatten()
        .min()
        .map(|deadline| deadline.saturating_duration_since(now))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControllerPollSchedule {
    deadline: Instant,
}

impl ControllerPollSchedule {
    pub const CONNECTED_INTERVAL: Duration = Duration::from_millis(16);
    pub const DISCONNECTED_INTERVAL: Duration = Duration::from_millis(250);

    pub fn new(now: Instant) -> Self {
        Self { deadline: now }
    }

    pub fn deadline(self) -> Instant {
        self.deadline
    }

    pub fn is_due(self, now: Instant) -> bool {
        now >= self.deadline
    }

    pub fn mark_polled(&mut self, now: Instant, connected: bool) {
        self.deadline = now
            + if connected {
                Self::CONNECTED_INTERVAL
            } else {
                Self::DISCONNECTED_INTERVAL
            };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Submit,
    Newline,
    Escape,
    Reload,
    Back,
    Forward,
    DocumentStart,
    DocumentEnd,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileDragEvent {
    Hovered(std::path::PathBuf),
    HoverCancelled,
    Dropped(std::path::PathBuf),
}

/// An application request to begin a native outbound file drag.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboundFileDrag {
    pub paths: Vec<std::path::PathBuf>,
}

/// Immutable environmental inputs for a declarative application view.
///
/// The host owns this state and supplies it before every resolve, so responsive
/// applications do not need a parallel resize callback or cached window size.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewContext {
    pub viewport: Rect,
    pub modality: InputModality,
    /// Host-owned keyboard/accessibility focus retained across declarative rebuilds.
    pub focused: Option<UiId>,
    /// Host-owned controller selection retained across declarative rebuilds.
    pub controller_target: Option<UiId>,
    /// Semantic actions accepted by the currently selected controller target.
    pub available_semantic_actions: Vec<ActionKind>,
    /// Depth of the active controller navigation scope.
    pub navigation_depth: usize,
    /// The topmost transient overlay, when controller navigation is trapped there.
    pub open_overlay: Option<OverlayId>,
}

/// Declarative content rendered above an application's ordinary view.
///
/// The host remains the sole owner of frame resolution, interaction state,
/// semantics, hit testing, and paint-list construction. Applications only
/// describe transient layers derived from their model.
#[derive(Clone, Debug)]
pub enum FrameOverlay<Message> {
    Menu(OverlayMenu<Message>),
    Surface(crate::TransientSurface),
    ContentSurface {
        surface: crate::TransientSurface,
        content: Box<crate::ui::Element<Message>>,
    },
    SelectionMarquee {
        rect: Rect,
        fill: Option<Color>,
        stroke: Color,
        width: f32,
    },
}

/// A named, interactive transient surface anchored to ordinary declarative UI.
/// Layout, focus trapping, dismissal, and focus return remain host-owned.
#[derive(Clone, Debug)]
pub struct Popover<Message> {
    surface: crate::TransientSurface,
    content: Box<crate::ui::Element<Message>>,
}

impl<Message> Popover<Message> {
    pub fn new(
        id: impl Into<UiId>,
        anchor: crate::OverlayAnchor,
        name: impl Into<String>,
        size: crate::Size,
        style: crate::OverlayStyle,
        content: impl crate::Component<Message>,
    ) -> Self {
        Self {
            surface: crate::TransientSurface::popover(id, anchor, size, style)
                .accessible_name(name),
            content: Box::new(content.into_element()),
        }
    }

    pub fn placement(mut self, placement: crate::OverlayPlacement) -> Self {
        self.surface = self.surface.placement(placement);
        self
    }

    pub fn collision(mut self, collision: crate::CollisionPolicy) -> Self {
        self.surface = self.surface.collision(collision);
        self
    }

    pub fn focus(mut self, focus: crate::OverlayFocusPolicy) -> Self {
        self.surface = self.surface.focus(focus);
        self
    }

    pub fn dismiss(mut self, dismiss: crate::DismissPolicy) -> Self {
        self.surface = self.surface.dismiss(dismiss);
        self
    }

    pub fn focus_return(mut self, target: impl Into<UiId>) -> Self {
        self.surface = self.surface.focus_return(target);
        self
    }

    pub fn direction(mut self, direction: crate::ReadingDirection) -> Self {
        self.surface = self.surface.direction(direction);
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.surface = self.surface.scale(scale);
        self
    }
}

/// A named, non-focus-stealing transient hint. Tooltips share the host's
/// collision and lifecycle machinery without pretending to be popovers.
#[derive(Clone, Debug)]
pub struct Tooltip<Message> {
    surface: crate::TransientSurface,
    content: Box<crate::ui::Element<Message>>,
}

impl<Message> Tooltip<Message> {
    pub fn new(
        id: impl Into<UiId>,
        anchor: crate::OverlayAnchor,
        name: impl Into<String>,
        size: crate::Size,
        style: crate::OverlayStyle,
        content: impl crate::Component<Message>,
    ) -> Self {
        Self {
            surface: crate::TransientSurface::tooltip(id, anchor, size, style)
                .accessible_name(name),
            content: Box::new(content.into_element()),
        }
    }

    pub fn placement(mut self, placement: crate::OverlayPlacement) -> Self {
        self.surface = self.surface.placement(placement);
        self
    }

    pub fn collision(mut self, collision: crate::CollisionPolicy) -> Self {
        self.surface = self.surface.collision(collision);
        self
    }

    pub fn dismiss(mut self, dismiss: crate::DismissPolicy) -> Self {
        self.surface = self.surface.dismiss(dismiss);
        self
    }

    pub fn direction(mut self, direction: crate::ReadingDirection) -> Self {
        self.surface = self.surface.direction(direction);
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.surface = self.surface.scale(scale);
        self
    }
}

impl<Message> From<Popover<Message>> for FrameOverlay<Message> {
    fn from(popover: Popover<Message>) -> Self {
        Self::ContentSurface {
            surface: popover.surface,
            content: popover.content,
        }
    }
}

impl<Message> From<Tooltip<Message>> for FrameOverlay<Message> {
    fn from(tooltip: Tooltip<Message>) -> Self {
        Self::ContentSurface {
            surface: tooltip.surface,
            content: tooltip.content,
        }
    }
}

impl<Message> FrameOverlay<Message> {
    pub fn surface(
        surface: crate::TransientSurface,
        content: impl crate::Component<Message>,
    ) -> Self {
        Self::ContentSurface {
            surface,
            content: Box::new(content.into_element()),
        }
    }
}

impl ViewContext {
    pub const fn new(viewport: Rect, modality: InputModality) -> Self {
        Self {
            viewport,
            modality,
            focused: None,
            controller_target: None,
            available_semantic_actions: Vec::new(),
            navigation_depth: 0,
            open_overlay: None,
        }
    }

    fn from_host(viewport: Rect, state: &UiStateStore, tree: Option<&UiFrame<impl Clone>>) -> Self {
        Self {
            viewport,
            modality: state.input_modality(),
            focused: state.focused().cloned(),
            controller_target: state.navigation().controller_selected().cloned(),
            available_semantic_actions: tree
                .map(|tree| tree.available_semantic_actions(state))
                .unwrap_or_default(),
            navigation_depth: tree.map(|tree| tree.navigation_depth(state)).unwrap_or(0),
            open_overlay: state.open_overlay_id().cloned(),
        }
    }
}

pub trait Application: Sized {
    type Message: Clone;

    fn update(&mut self, message: Self::Message);

    fn message_evidence(&self, _message: &Self::Message) -> MessageEvidence {
        MessageEvidence {
            type_name: std::any::type_name::<Self::Message>(),
            label: None,
        }
    }

    /// Drains application-owned effects produced by updates, completions, or
    /// polling so adapters, scenarios, and telemetry observe the same effects.
    fn take_effect_evidence(&mut self) -> Vec<EffectEvidence> {
        Vec::new()
    }

    /// Drains a semantic focus request produced by a domain update.
    ///
    /// The host resolves the stable application id against the rebuilt tree,
    /// including component-generated ancestor prefixes, and performs focus as
    /// an ordinary UI transition. Applications therefore never need to retain
    /// a frame or mutate [`UiStateStore`] to reconcile focus after rebuilding.
    fn take_focus_request(&mut self) -> Option<UiId> {
        None
    }

    /// Applies an application/domain completion injected by a host adapter or
    /// deterministic scenario. Implementations downcast the typed payload and
    /// return whether the completion changed declarative state.
    fn complete(&mut self, completion: Completion) -> Result<bool, CompletionFailure> {
        Err(CompletionFailure {
            id: completion.id,
            kind: CompletionFailureKind::Unhandled,
            detail: "application has no matching completion subscription".into(),
        })
    }

    fn view(&self, context: ViewContext) -> impl View<Self::Message>;

    fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        Vec::new()
    }

    /// Poll application-owned background work without introducing another UI runtime.
    /// Return `true` when new state requires a redraw.
    fn poll(&mut self) -> bool {
        false
    }

    /// Declares the cadence for application-owned completion polling.
    /// `None` means the application is event-driven and must not be woken.
    fn poll_interval(&self) -> Option<Duration> {
        None
    }

    /// Handle application-level keyboard semantics before ordinary component activation.
    fn shortcut(&mut self, _shortcut: Shortcut) -> bool {
        false
    }

    /// Offers normalized RGBA clipboard pixels to the focused application.
    /// Returning true makes image data win over simultaneous clipboard text.
    fn paste_clipboard_image(&mut self, _width: u32, _height: u32, _rgba: &[u8]) -> bool {
        false
    }

    /// Receives native file drag offers without exposing platform MIME or COM
    /// details to application policy.
    fn file_drag_event(&mut self, _event: FileDragEvent) -> bool {
        false
    }

    /// Takes a pending outbound file drag. Hosts call this synchronously while
    /// handling the pointer press/motion that supplied the native drag serial.
    fn take_outbound_file_drag(&mut self) -> Option<OutboundFileDrag> {
        None
    }

    /// Reports the controller presentation currently driving this host.
    /// Applications can retain it when controller-specific legends are part
    /// of their declarative view.
    fn controller_family_changed(&mut self, _family: ControllerFamily) -> bool {
        false
    }

    /// Reports the physical pixels per logical pixel used by the host.
    fn scale_factor_changed(&mut self, _scale_factor: f32) -> bool {
        false
    }

    fn title(&self) -> &str {
        "Nickel UI"
    }

    fn initial_size(&self) -> (u32, u32) {
        (800, 600)
    }
}

fn controller_ui_event(action: ControllerAction) -> Option<UiEvent> {
    match action {
        ControllerAction::Up => Some(UiEvent::ControllerUp),
        ControllerAction::Down => Some(UiEvent::ControllerDown),
        ControllerAction::Left => Some(UiEvent::ControllerLeft),
        ControllerAction::Right => Some(UiEvent::ControllerRight),
        ControllerAction::Confirm => Some(UiEvent::ControllerActivate),
        ControllerAction::Cancel => Some(UiEvent::ControllerBack),
        ControllerAction::PreviousPane => Some(UiEvent::ControllerPreviousPane),
        ControllerAction::NextPane => Some(UiEvent::ControllerNextPane),
        ControllerAction::Launcher => None,
        ControllerAction::ContextMenu => Some(UiEvent::ControllerContextMenu),
    }
}

pub struct UiHost<A: Application> {
    application: A,
    state: UiStateStore,
    tree: UiFrame<A::Message>,
    bounds: Rect,
    scale_factor: f32,
    input_dispatcher: FocusedInputDispatcher,
    frame_generation: u64,
    pointer_icon: PointerIcon,
    overlay_failures: Vec<OverlayDeclarationFailure>,
    next_application_deadline: Option<Instant>,
}

#[derive(Clone, Default)]
struct OverlayInteractionSnapshot {
    focused: Option<UiId>,
    hovered: Option<UiId>,
    pressed: Option<UiId>,
    captured: Option<UiId>,
    controller_selected: Option<UiId>,
}

impl OverlayInteractionSnapshot {
    fn capture<Message: Clone>(state: &UiStateStore, tree: &UiFrame<Message>) -> Self {
        let owned = |id: Option<&UiId>| id.filter(|id| tree.contains_target(id)).cloned();
        let controller_selected = if let Some(overlay) = state.open_overlay_id() {
            state
                .navigation()
                .controller_selected()
                .filter(|id| tree.is_descendant_or_self(overlay.as_ui_id(), id))
                .cloned()
        } else {
            owned(state.navigation().controller_selected())
        };
        Self {
            focused: owned(state.focused()),
            hovered: owned(state.hovered()),
            pressed: owned(state.pressed()),
            captured: owned(state.captured()),
            controller_selected,
        }
    }

    fn restore<Message: Clone>(self, state: &mut UiStateStore, tree: &UiFrame<Message>) {
        let valid = |id: Option<UiId>| id.filter(|id| tree.contains_target(id));
        if self.focused.is_some() {
            state.set_focus(valid(self.focused));
        }
        if self.hovered.is_some() {
            state.set_hovered(valid(self.hovered));
        }
        if self.pressed.is_some() {
            state.set_pressed(valid(self.pressed));
        }
        if self.captured.is_some() {
            state.set_capture(valid(self.captured));
        }
        if self.controller_selected.is_some() {
            state
                .navigation_mut()
                .set_controller_selected(valid(self.controller_selected));
        }
    }

    fn restore_before_overlay(&self, state: &mut UiStateStore) {
        if let Some(focused) = &self.focused {
            state.set_focus(Some(focused.clone()));
        }
        if let Some(hovered) = &self.hovered {
            state.set_hovered(Some(hovered.clone()));
        }
        if let Some(pressed) = &self.pressed {
            state.set_pressed(Some(pressed.clone()));
        }
        if let Some(captured) = &self.captured {
            state.set_capture(Some(captured.clone()));
        }
        if let Some(selected) = &self.controller_selected {
            state
                .navigation_mut()
                .set_controller_selected(Some(selected.clone()));
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlayDeclarationFailure {
    pub overlay: OverlayId,
    pub anchor: UiId,
    pub error: SemanticActionError,
}

pub enum HostEvent {
    Ui(UiEvent),
    Controller(ControllerAction),
    Shortcut(Shortcut),
    Semantic {
        target: UiId,
        action: SemanticAction,
    },
    Accessibility {
        target: UiId,
        action: SemanticAction,
    },
    ControllerSemantic {
        target: UiId,
        action: SemanticAction,
    },
    Normalized {
        input: nickel_input::InputEvent,
        clipboard_text: Option<String>,
    },
    Poll,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalAction {
    ToggleLauncher,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdapterOutcome {
    pub changed: bool,
    pub consume: bool,
    pub exit: bool,
}

impl AdapterOutcome {
    pub const fn changed() -> Self {
        Self {
            changed: true,
            consume: false,
            exit: false,
        }
    }

    pub const fn consumed(changed: bool) -> Self {
        Self {
            changed,
            consume: true,
            exit: false,
        }
    }

    pub const fn exit() -> Self {
        Self {
            changed: false,
            consume: true,
            exit: true,
        }
    }
}

/// Read-only native services supplied to an application-specific host adapter.
/// Rendering, normalized input, clipboard routing, and controller navigation
/// remain owned by [`UiHost`].
pub struct HostServices<'a> {
    window: &'a Window,
}

impl<'a> HostServices<'a> {
    pub fn window(&self) -> &'a Window {
        self.window
    }
}

/// Injects platform-specific effects into the canonical Nickel UI runtime
/// without creating an application-owned event loop.
pub trait HostAdapter<A: Application> {
    fn poll_interval(&self) -> Option<Duration> {
        None
    }

    /// Declares the next adapter wakeup. Adapters without pending work return
    /// `None`, allowing the platform event loop to sleep indefinitely.
    fn next_deadline(&self, _now: Instant) -> Option<Instant> {
        None
    }

    fn started(
        &mut self,
        _host: &mut UiHost<A>,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        Ok(AdapterOutcome::default())
    }

    fn event(
        &mut self,
        _host: &mut UiHost<A>,
        _event: &WindowEvent,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        Ok(AdapterOutcome::default())
    }

    /// Observes canonical normalized input at the same dispatch boundary as
    /// [`UiHost`]. Continuous pointer input is therefore delivered only after
    /// the runtime has coalesced it for the next presentation.
    fn normalized_input(
        &mut self,
        _host: &mut UiHost<A>,
        _input: &nickel_input::InputEvent,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        Ok(AdapterOutcome::default())
    }

    fn poll(
        &mut self,
        _host: &mut UiHost<A>,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        Ok(AdapterOutcome::default())
    }

    fn global_action(
        &mut self,
        _host: &mut UiHost<A>,
        _action: GlobalAction,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        Ok(AdapterOutcome::default())
    }

    fn stopped(
        &mut self,
        _host: &mut UiHost<A>,
        _services: HostServices<'_>,
    ) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}

#[derive(Default)]
pub struct DefaultHostAdapter;

impl<A: Application> HostAdapter<A> for DefaultHostAdapter {}

#[derive(Default)]
pub struct HostBatch {
    /// Monotonic time supplied by an adapter or deterministic harness.
    pub now: Option<Instant>,
    /// The embedding host mutated application-owned view data directly.
    pub application_changed: bool,
    pub surface_size: Option<(u32, u32)>,
    pub scale_factor: Option<f32>,
    pub window_focused: Option<bool>,
    pub completions: Vec<Completion>,
    /// Failures observed by the transport while servicing this batch.  They
    /// are evidence, not application input: reporting one must not mutate or
    /// short-circuit the canonical UI transition.
    pub failures: Vec<HostFailure>,
    pub events: Vec<HostEvent>,
}

pub struct Completion {
    pub id: &'static str,
    payload: Box<dyn Any + Send>,
}

impl Completion {
    pub fn new<T: Any + Send>(id: &'static str, payload: T) -> Self {
        Self {
            id,
            payload: Box::new(payload),
        }
    }

    pub fn downcast<T: Any + Send>(self) -> Result<T, Self> {
        match self.payload.downcast::<T>() {
            Ok(value) => Ok(*value),
            Err(payload) => Err(Self {
                id: self.id,
                payload,
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionFailure {
    pub id: &'static str,
    pub kind: CompletionFailureKind,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionFailureKind {
    Unhandled,
    TypeMismatch,
    Rejected,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostInspection {
    pub frame_generation: u64,
    pub semantic_generation: u64,
    pub input: InputContext,
    pub window_focused: bool,
    pub scale_factor: f32,
    pub pointer_icon: PointerIcon,
    pub keyboard_focus: Option<UiId>,
    /// The semantic target currently owned by production pointer hit testing.
    pub pointer_hover: Option<UiId>,
    pub pointer_capture: Option<UiId>,
    pub controller_target: Option<UiId>,
    pub controller_scope: Option<UiId>,
    pub navigation_depth: usize,
    pub available_semantic_actions: Vec<ActionKind>,
    pub controller_editing: bool,
    pub open_overlay: Option<OverlayId>,
    pub modality: InputModality,
    pub diagnostics: Vec<LayoutDiagnostic>,
    pub resources: FrameResourceDiagnostics,
    pub overlay_failures: Vec<OverlayDeclarationFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageEvidence {
    pub type_name: &'static str,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectEvidence {
    pub type_name: &'static str,
    pub label: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostFailureStage {
    Presenter,
    Clipboard,
    Ime,
    Accessibility,
    Controller,
    DomainService,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostFailure {
    pub surface: String,
    pub stage: HostFailureStage,
    pub optional: bool,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HostChangeToken {
    pub frame_generation: u64,
    pub semantic_generation: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HostEventOutcome {
    pub changed: bool,
    pub invalidation: Invalidation,
    pub messages: Vec<MessageEvidence>,
    pub effects: Vec<EffectEvidence>,
    pub failures: Vec<HostFailure>,
    pub completion_failures: Vec<CompletionFailure>,
    pub pointer_icon: PointerIcon,
    pub text_input_active: bool,
    pub accessibility_generation: u64,
    pub change_token: HostChangeToken,
    pub next_deadline: Option<Instant>,
    pub telemetry: HostTelemetry,
    pub clipboard_text: Option<String>,
    pub semantic_failures: Vec<SemanticActionFailure>,
    pub global_actions: Vec<GlobalAction>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HostTelemetry {
    pub events_processed: usize,
    pub completions_processed: usize,
    pub rebuilt: bool,
    /// Time from beginning the host step through application message dispatch.
    pub input_to_message_us: u64,
    /// Time from beginning the host step through the resolved frame.
    pub input_to_frame_us: u64,
    /// Time spent resolving declarative layout during this step.
    pub layout_us: u64,
    /// Time spent constructing the application's declarative view/paint list.
    pub paint_list_us: u64,
    /// Explicit host/application deadline wakeups processed by this step.
    pub scheduled_wakeups: usize,
    /// Retained bytes owned by the resolved frame after this step.
    pub retained_frame_bytes: usize,
    /// Allocation counting is not available in the shared runtime unless a
    /// concrete adapter installs an allocator-visible counter.
    pub allocation_count: Option<u64>,
}

impl Default for HostEventOutcome {
    fn default() -> Self {
        Self {
            changed: false,
            invalidation: Invalidation::None,
            messages: Vec::new(),
            effects: Vec::new(),
            failures: Vec::new(),
            completion_failures: Vec::new(),
            pointer_icon: PointerIcon::Default,
            text_input_active: false,
            accessibility_generation: 0,
            change_token: HostChangeToken::default(),
            next_deadline: None,
            telemetry: HostTelemetry::default(),
            clipboard_text: None,
            semantic_failures: Vec::new(),
            global_actions: Vec::new(),
        }
    }
}

impl HostEventOutcome {
    fn merge(&mut self, mut other: Self) {
        self.changed |= other.changed;
        self.invalidation = self.invalidation.merge(other.invalidation);
        self.messages.append(&mut other.messages);
        self.effects.append(&mut other.effects);
        self.failures.append(&mut other.failures);
        if other.clipboard_text.is_some() {
            self.clipboard_text = other.clipboard_text;
        }
        self.semantic_failures.append(&mut other.semantic_failures);
        self.completion_failures
            .append(&mut other.completion_failures);
        self.global_actions.append(&mut other.global_actions);
        self.pointer_icon = other.pointer_icon;
        self.text_input_active = other.text_input_active;
        self.accessibility_generation = self
            .accessibility_generation
            .max(other.accessibility_generation);
        self.change_token.frame_generation = self
            .change_token
            .frame_generation
            .max(other.change_token.frame_generation);
        self.change_token.semantic_generation = self
            .change_token
            .semantic_generation
            .max(other.change_token.semantic_generation);
        self.next_deadline = match (self.next_deadline, other.next_deadline) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (deadline @ Some(_), None) | (None, deadline @ Some(_)) => deadline,
            (None, None) => None,
        };
        self.telemetry.events_processed = self
            .telemetry
            .events_processed
            .saturating_add(other.telemetry.events_processed);
        self.telemetry.completions_processed = self
            .telemetry
            .completions_processed
            .saturating_add(other.telemetry.completions_processed);
        self.telemetry.rebuilt |= other.telemetry.rebuilt;
        self.telemetry.input_to_message_us = self
            .telemetry
            .input_to_message_us
            .saturating_add(other.telemetry.input_to_message_us);
        self.telemetry.input_to_frame_us = self
            .telemetry
            .input_to_frame_us
            .saturating_add(other.telemetry.input_to_frame_us);
        self.telemetry.layout_us = self
            .telemetry
            .layout_us
            .saturating_add(other.telemetry.layout_us);
        self.telemetry.paint_list_us = self
            .telemetry
            .paint_list_us
            .saturating_add(other.telemetry.paint_list_us);
        self.telemetry.scheduled_wakeups = self
            .telemetry
            .scheduled_wakeups
            .saturating_add(other.telemetry.scheduled_wakeups);
        self.telemetry.retained_frame_bytes = self
            .telemetry
            .retained_frame_bytes
            .max(other.telemetry.retained_frame_bytes);
        self.telemetry.allocation_count = match (
            self.telemetry.allocation_count,
            other.telemetry.allocation_count,
        ) {
            (Some(left), Some(right)) => Some(left.saturating_add(right)),
            _ => None,
        };
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticActionFailure {
    pub target: UiId,
    pub error: SemanticActionError,
}

impl<A: Application> UiHost<A> {
    pub fn paste_clipboard_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> bool {
        if self.application.paste_clipboard_image(width, height, rgba) {
            self.rebuild();
            true
        } else {
            false
        }
    }
    pub fn set_controller_family(&mut self, family: ControllerFamily) -> bool {
        if self.application.controller_family_changed(family) {
            self.rebuild();
            true
        } else {
            false
        }
    }
    pub fn set_scale_factor(&mut self, scale_factor: f32) -> bool {
        self.step(HostBatch {
            scale_factor: Some(scale_factor),
            ..HostBatch::default()
        })
        .changed
    }
    pub fn new(application: A, width: u32, height: u32) -> Self {
        Self::new_at(application, width, height, Instant::now())
    }

    /// Constructs a host against an explicit monotonic origin for deterministic
    /// scheduling tests and embedded adapters that already own the clock.
    pub fn new_at(application: A, width: u32, height: u32, now: Instant) -> Self {
        let bounds = Rect::new(0.0, 0.0, width as f32, height as f32);
        let mut state = UiStateStore::default();
        let context = ViewContext::from_host(bounds, &state, None::<&UiFrame<A::Message>>);
        let mut tree = UiFrame::resolve(
            application.view(context.clone()),
            FrameRequest::new(bounds, &mut state),
        );
        let overlay_failures =
            apply_frame_overlays(&mut tree, &mut state, application.frame_overlays(context));
        let next_application_deadline = application.poll_interval().map(|interval| now + interval);
        Self {
            application,
            state,
            tree,
            bounds,
            scale_factor: 1.0,
            input_dispatcher: FocusedInputDispatcher::default(),
            frame_generation: 1,
            pointer_icon: PointerIcon::Default,
            overlay_failures,
            next_application_deadline,
        }
    }

    pub fn application_mut(&mut self) -> &mut A {
        &mut self.application
    }

    pub fn application(&self) -> &A {
        &self.application
    }

    pub fn commands(&self) -> &[crate::PaintCommand] {
        self.tree.commands()
    }

    pub fn render_software(&self, renderer: &mut SoftwareRenderer) -> DamageRegion {
        renderer.render(self.tree.commands())
    }

    pub fn pointer_icon_at(&self, point: crate::Point) -> PointerIcon {
        self.tree.pointer_icon_at(point)
    }

    pub fn input_context(&self) -> crate::InputContext {
        crate::InputContext {
            text_focused: self
                .state
                .focused()
                .is_some_and(|id| self.tree.is_text_input(id)),
            navigation_active: self.state.navigation().controller_selected().is_some(),
            selection_owned: self.state.selection_owner().is_some(),
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.tree.selected_text(&self.state)
    }

    pub fn semantic_nodes(&self) -> Vec<SemanticNodeSnapshot> {
        self.tree.semantic_nodes()
    }

    pub fn query(&self, selector: &SemanticSelector) -> Vec<SemanticNodeSnapshot> {
        self.tree.query(selector)
    }

    pub fn query_unique(
        &self,
        selector: &SemanticSelector,
    ) -> Result<SemanticNodeSnapshot, SemanticQueryError> {
        self.tree.query_unique(selector)
    }

    pub fn semantic_targets_for_message(&self, message: &A::Message) -> Vec<crate::SemanticTarget>
    where
        A::Message: PartialEq,
    {
        self.tree.semantic_targets_for_message(message)
    }

    /// Returns the typed application message bound to a semantic target.
    ///
    /// Semantic IDs are opaque identity tokens. Adapters that need to route
    /// hover or other non-activating interactions can use this lookup instead
    /// of recovering application meaning by parsing an ID's text.
    pub fn message_for_semantic_target(&self, target: &UiId) -> Option<&A::Message> {
        self.tree.message_for_id(target)
    }

    pub fn unique_semantic_target_for_message(
        &self,
        message: &A::Message,
    ) -> Result<crate::SemanticTarget, SemanticQueryError>
    where
        A::Message: PartialEq,
    {
        self.tree.unique_semantic_target_for_message(message)
    }

    pub fn accessibility_nodes(&self) -> &[AccessibilityNode] {
        self.tree.accessibility_nodes()
    }

    pub fn resolved_grid_columns(&self) -> Option<usize> {
        self.tree.resolved_grid_columns()
    }

    /// Scrolls the canonical view state just enough to reveal a message-bound
    /// item. The host owns both geometry and scroll state; applications only
    /// name the item and its scroll surface.
    pub fn ensure_message_visible(&mut self, item: &A::Message, scroll: &A::Message) -> bool
    where
        A::Message: PartialEq,
    {
        let Ok(item_target) = self.tree.unique_semantic_target_for_message(item) else {
            return false;
        };
        let item_rect = item_target.bounds;
        let Some(viewport) = self.tree.scroll_viewport(scroll) else {
            return false;
        };
        let Some(extent) = self.tree.scroll_extent(scroll) else {
            return false;
        };
        let Some(target) = self
            .tree
            .semantic_targets_for_message(scroll)
            .into_iter()
            .next()
        else {
            return false;
        };
        let delta = if item_rect.origin.y < viewport.origin.y {
            item_rect.origin.y - viewport.origin.y
        } else {
            let item_bottom = item_rect.origin.y + item_rect.size.height;
            let viewport_bottom = viewport.origin.y + viewport.size.height;
            (item_bottom - viewport_bottom).max(0.0)
        };
        let maximum = (extent.content.height - extent.viewport.height).max(0.0);
        let changed = self.state.scroll_by(target.id, delta, maximum) != crate::Invalidation::None;
        if changed {
            self.rebuild();
        }
        changed
    }

    pub fn reset_scroll(&mut self, scroll: &A::Message) -> bool
    where
        A::Message: PartialEq,
    {
        let Some(extent) = self.tree.scroll_extent(scroll) else {
            return false;
        };
        let Some(target) = self
            .tree
            .semantic_targets_for_message(scroll)
            .into_iter()
            .next()
        else {
            return false;
        };
        let maximum = (extent.content.height - extent.viewport.height).max(0.0);
        let current = self
            .state
            .state(&target.id)
            .map_or(extent.offset, |state| state.scroll_offset);
        let changed =
            self.state.scroll_by(target.id, -current, maximum) != crate::Invalidation::None;
        if changed {
            self.rebuild();
        }
        changed
    }

    pub fn resolve_effective_target(
        &self,
        target: &UiId,
        action: ActionKind,
    ) -> Result<EffectiveHitRoute, SemanticActionError> {
        self.tree.resolve_effective_target(target, action)
    }

    pub fn perform_semantic_action(
        &mut self,
        target: UiId,
        action: SemanticAction,
    ) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Semantic { target, action }],
            ..HostBatch::default()
        })
    }

    pub fn perform_accessibility_action(
        &mut self,
        target: UiId,
        action: SemanticAction,
    ) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Accessibility { target, action }],
            ..HostBatch::default()
        })
    }

    pub fn perform_controller_semantic_action(
        &mut self,
        target: UiId,
        action: SemanticAction,
    ) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::ControllerSemantic { target, action }],
            ..HostBatch::default()
        })
    }

    pub fn open_transient(&mut self, id: OverlayId, invocation_target: UiId) -> bool {
        let changed = self.state.open_overlay(id, invocation_target) != Invalidation::None;
        if changed {
            self.rebuild();
        }
        changed
    }

    /// Requests focus through the frame reducer without changing the user's
    /// current input modality. Adapters use this after an application update
    /// resolves a new semantic descendant.
    pub fn request_focus(&mut self, target: UiId) -> HostEventOutcome {
        let transition = self
            .tree
            .transition(
                &mut self.state,
                InputSource::System,
                InteractionIntent::Event(UiEvent::AccessibilityFocus(target)),
            )
            .expect("system focus is an ordinary frame event");
        let changed =
            transition.invalidation != crate::Invalidation::None || !transition.messages.is_empty();
        for message in transition.messages {
            self.application.update(message);
        }
        let mut outcome = HostEventOutcome {
            changed,
            clipboard_text: transition.clipboard_text,
            ..HostEventOutcome::default()
        };
        if changed {
            self.rebuild();
        }
        outcome.effects = self.application.take_effect_evidence();
        outcome.pointer_icon = self.pointer_icon;
        outcome.text_input_active = self.input_context().text_focused;
        outcome.accessibility_generation = self.frame_generation;
        outcome.change_token = HostChangeToken {
            frame_generation: self.frame_generation,
            semantic_generation: self.frame_generation,
        };
        outcome
    }

    pub fn inspect(&self) -> HostInspection {
        HostInspection {
            frame_generation: self.frame_generation,
            semantic_generation: self.frame_generation,
            input: self.input_context(),
            window_focused: self.state.window_focused(),
            scale_factor: self.scale_factor,
            pointer_icon: self.pointer_icon,
            keyboard_focus: self.state.focused().cloned(),
            pointer_hover: self.state.hovered().cloned(),
            pointer_capture: self.state.captured().cloned(),
            controller_target: self.state.navigation().controller_selected().cloned(),
            controller_scope: self.state.navigation().controller_scope().cloned(),
            navigation_depth: self.tree.navigation_depth(&self.state),
            available_semantic_actions: self.tree.available_semantic_actions(&self.state),
            controller_editing: self.state.navigation().controller_editing(),
            open_overlay: self.state.open_overlay_id().cloned(),
            modality: self.state.input_modality(),
            diagnostics: self.tree.diagnostics().to_vec(),
            resources: self.tree.resource_diagnostics(),
            overlay_failures: self.overlay_failures.clone(),
        }
    }

    pub fn step(&mut self, batch: HostBatch) -> HostEventOutcome {
        let step_started = Instant::now();
        let now = batch.now.unwrap_or_else(Instant::now);
        let mut combined = HostEventOutcome {
            failures: batch.failures,
            ..HostEventOutcome::default()
        };
        combined.telemetry.events_processed = batch.events.len();
        combined.telemetry.completions_processed = batch.completions.len();
        if batch.application_changed {
            combined.changed = true;
            combined.invalidation = Invalidation::Layout;
        }
        combined.telemetry.scheduled_wakeups = batch
            .events
            .iter()
            .filter(|event| matches!(event, HostEvent::Poll))
            .count();
        if let Some((width, height)) = batch.surface_size {
            let next = Rect::new(0.0, 0.0, width as f32, height as f32);
            if next != self.bounds {
                self.bounds = next;
                combined.changed = true;
                combined.invalidation = Invalidation::Layout;
            }
        }
        if let Some(scale_factor) = batch.scale_factor
            && scale_factor.is_finite()
            && scale_factor > 0.0
            && scale_factor != self.scale_factor
        {
            self.scale_factor = scale_factor;
            combined.changed = true;
            combined.invalidation = combined.invalidation.merge(Invalidation::Layout);
            if self.application.scale_factor_changed(scale_factor) {
                combined.changed = true;
                combined.invalidation = combined.invalidation.merge(Invalidation::Paint);
            }
        }
        if let Some(focused) = batch.window_focused {
            let focus = self.dispatch_ui_event(if focused {
                UiEvent::FocusGained
            } else {
                UiEvent::FocusLost
            });
            combined.changed |= focus.changed;
            combined.invalidation = combined.invalidation.merge(focus.invalidation);
        }
        for completion in batch.completions {
            match self.application.complete(completion) {
                Ok(changed) => {
                    combined.changed |= changed;
                    if changed {
                        combined.invalidation = combined.invalidation.merge(Invalidation::Layout);
                    }
                }
                Err(failure) => combined.completion_failures.push(failure),
            }
        }
        for event in batch.events {
            let outcome = match event {
                HostEvent::Ui(event) => self.dispatch_ui_event(event),
                HostEvent::Controller(action) => self.dispatch_controller_action(action),
                HostEvent::Shortcut(shortcut) => {
                    let changed = self.application.shortcut(shortcut);
                    HostEventOutcome {
                        changed,
                        invalidation: if changed {
                            Invalidation::Layout
                        } else {
                            Invalidation::None
                        },
                        ..HostEventOutcome::default()
                    }
                }
                HostEvent::Semantic { target, action } => {
                    self.dispatch_semantic_action(target, action, InputSource::Programmatic)
                }
                HostEvent::Accessibility { target, action } => {
                    self.dispatch_semantic_action(target, action, InputSource::Accessibility)
                }
                HostEvent::ControllerSemantic { target, action } => {
                    self.dispatch_semantic_action(target, action, InputSource::Controller)
                }
                HostEvent::Normalized {
                    input,
                    clipboard_text,
                } => self.dispatch_input(&input, clipboard_text.as_deref()),
                HostEvent::Poll => {
                    let changed = self.application.poll();
                    self.next_application_deadline = self
                        .application
                        .poll_interval()
                        .map(|interval| now + interval);
                    HostEventOutcome {
                        changed,
                        invalidation: if changed {
                            Invalidation::Layout
                        } else {
                            Invalidation::None
                        },
                        ..HostEventOutcome::default()
                    }
                }
            };
            combined.merge(outcome);
        }
        combined.telemetry.input_to_message_us = elapsed_us(step_started);
        if combined.changed {
            let (paint_list_us, layout_us) = self.rebuild_timed();
            combined.telemetry.paint_list_us = paint_list_us;
            combined.telemetry.layout_us = layout_us;
            combined.telemetry.rebuilt = true;
        }
        if let Some(requested) = self.application.take_focus_request()
            && let Some(target) = self.tree.resolve_stable_target(&requested)
        {
            let focus = self
                .tree
                .transition(
                    &mut self.state,
                    InputSource::System,
                    InteractionIntent::Event(UiEvent::AccessibilityFocus(target)),
                )
                .expect("host focus requests are ordinary frame events");
            if focus.invalidation != Invalidation::None || !focus.messages.is_empty() {
                combined.changed = true;
                combined.invalidation = combined.invalidation.merge(focus.invalidation);
                for message in focus.messages {
                    self.application.update(message);
                }
                let (paint_list_us, layout_us) = self.rebuild_timed();
                combined.telemetry.paint_list_us = combined
                    .telemetry
                    .paint_list_us
                    .saturating_add(paint_list_us);
                combined.telemetry.layout_us =
                    combined.telemetry.layout_us.saturating_add(layout_us);
                combined.telemetry.rebuilt = true;
            }
        }
        combined.effects = self.application.take_effect_evidence();
        self.next_application_deadline = match (
            self.next_application_deadline,
            self.application.poll_interval(),
        ) {
            (_, None) => None,
            (Some(deadline), Some(_)) => Some(deadline),
            (None, Some(interval)) => Some(now + interval),
        };
        combined.pointer_icon = self.pointer_icon;
        combined.text_input_active = self.input_context().text_focused;
        combined.accessibility_generation = self.frame_generation;
        combined.change_token = HostChangeToken {
            frame_generation: self.frame_generation,
            semantic_generation: self.frame_generation,
        };
        combined.next_deadline = self.next_application_deadline;
        combined.telemetry.retained_frame_bytes =
            self.tree.resource_diagnostics().estimated_retained_bytes;
        combined.telemetry.input_to_frame_us = elapsed_us(step_started);
        combined
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.next_application_deadline
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.step(HostBatch {
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
    }

    pub fn poll(&mut self) -> bool {
        self.step(HostBatch {
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        })
        .changed
    }

    pub fn handle_event(&mut self, event: UiEvent) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Ui(event)],
            ..HostBatch::default()
        })
    }

    fn dispatch_ui_event(&mut self, event: UiEvent) -> HostEventOutcome {
        if let UiEvent::PointerMoved(point)
        | UiEvent::PointerPressed(point)
        | UiEvent::PointerReleased(point) = &event
        {
            self.pointer_icon = self.tree.pointer_icon_at(*point);
        }
        let source = event.input_source();
        let outcome = self
            .tree
            .transition(&mut self.state, source, InteractionIntent::Event(event))
            .expect("ordinary UI events cannot fail semantic resolution");
        let invalidation = outcome.invalidation;
        let changed = invalidation != Invalidation::None || !outcome.messages.is_empty();
        let messages = outcome
            .messages
            .iter()
            .map(|message| self.application.message_evidence(message))
            .collect();
        for message in outcome.messages {
            self.application.update(message);
        }
        let clipboard_text = outcome.clipboard_text;
        HostEventOutcome {
            changed,
            invalidation,
            messages,
            clipboard_text,
            semantic_failures: Vec::new(),
            global_actions: Vec::new(),
            completion_failures: Vec::new(),
            ..HostEventOutcome::default()
        }
    }

    fn dispatch_semantic_action(
        &mut self,
        target: UiId,
        action: SemanticAction,
        source: InputSource,
    ) -> HostEventOutcome {
        match self.tree.transition(
            &mut self.state,
            source,
            InteractionIntent::Invoke {
                target: target.clone(),
                action,
            },
        ) {
            Ok(outcome) => {
                let invalidation = outcome.invalidation;
                let changed = invalidation != Invalidation::None || !outcome.messages.is_empty();
                let messages = outcome
                    .messages
                    .iter()
                    .map(|message| self.application.message_evidence(message))
                    .collect();
                for message in outcome.messages {
                    self.application.update(message);
                }
                HostEventOutcome {
                    changed,
                    invalidation,
                    messages,
                    clipboard_text: outcome.clipboard_text,
                    semantic_failures: Vec::new(),
                    global_actions: Vec::new(),
                    completion_failures: Vec::new(),
                    ..HostEventOutcome::default()
                }
            }
            Err(error) => HostEventOutcome {
                semantic_failures: vec![SemanticActionFailure { target, error }],
                ..HostEventOutcome::default()
            },
        }
    }

    pub fn handle_controller_action(&mut self, action: ControllerAction) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        })
    }

    fn dispatch_controller_action(&mut self, action: ControllerAction) -> HostEventOutcome {
        if !self.state.window_focused() {
            return HostEventOutcome::default();
        }
        if action == ControllerAction::Launcher {
            return HostEventOutcome {
                global_actions: vec![GlobalAction::ToggleLauncher],
                ..HostEventOutcome::default()
            };
        }
        controller_ui_event(action)
            .map(|event| self.dispatch_ui_event(event))
            .unwrap_or_default()
    }

    pub fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        self.step(HostBatch {
            events: vec![HostEvent::Shortcut(shortcut)],
            ..HostBatch::default()
        })
        .changed
    }

    /// Dispatch a normalized event through the same focused-input contract used by standalone
    /// Nickel UI applications. Embedded hosts provide clipboard text only when paste is allowed;
    /// copy and cut return replacement clipboard text in the outcome.
    pub fn handle_input(
        &mut self,
        input: &nickel_input::InputEvent,
        clipboard_text: Option<&str>,
    ) -> HostEventOutcome {
        self.step(HostBatch {
            events: vec![HostEvent::Normalized {
                input: input.clone(),
                clipboard_text: clipboard_text.map(ToOwned::to_owned),
            }],
            ..HostBatch::default()
        })
    }

    fn dispatch_input(
        &mut self,
        input: &nickel_input::InputEvent,
        clipboard_text: Option<&str>,
    ) -> HostEventOutcome {
        let context = self.input_context();
        let commands = self.input_dispatcher.dispatch_with_context(input, context);
        let mut combined = HostEventOutcome::default();
        for command in commands {
            let event = match command {
                InputCommand::Ui(event) => Some(event),
                InputCommand::Application { shortcut, fallback } => {
                    if self.application.shortcut(shortcut) {
                        combined.changed = true;
                        combined.invalidation = combined.invalidation.merge(Invalidation::Layout);
                        None
                    } else {
                        fallback
                    }
                }
                InputCommand::Copy => Some(UiEvent::TextCopy),
                InputCommand::Cut => Some(UiEvent::TextCut),
                InputCommand::Paste => clipboard_text.map(|text| UiEvent::TextPaste(text.into())),
            };
            let Some(event) = event else {
                continue;
            };
            let outcome = self.dispatch_ui_event(event);
            combined.merge(outcome);
        }
        combined
    }

    fn rebuild(&mut self) {
        let _ = self.rebuild_timed();
    }

    fn rebuild_timed(&mut self) -> (u64, u64) {
        let context = ViewContext::from_host(self.bounds, &self.state, Some(&self.tree));
        let overlay_interaction = OverlayInteractionSnapshot::capture(&self.state, &self.tree);
        let paint_started = Instant::now();
        let view = self.application.view(context.clone());
        let overlays = self.application.frame_overlays(context);
        let paint_list_us = elapsed_us(paint_started);
        let layout_started = Instant::now();
        self.tree = UiFrame::resolve(view, FrameRequest::new(self.bounds, &mut self.state));
        // Base resolution cannot retain transient descendants because their
        // topology is declared next. Restore interaction ownership before
        // overlay emission so paint and semantics observe the same state.
        overlay_interaction.restore_before_overlay(&mut self.state);
        self.overlay_failures = apply_frame_overlays(&mut self.tree, &mut self.state, overlays);
        overlay_interaction.restore(&mut self.state, &self.tree);
        self.tree.reconcile_transient_focus(&mut self.state);
        self.tree.finalize_transient_layers(&self.state);
        self.frame_generation = self.frame_generation.wrapping_add(1);
        (paint_list_us, elapsed_us(layout_started))
    }

    pub fn shutdown(&mut self) {
        self.state.destroy();
    }
}

fn elapsed_us(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

fn apply_frame_overlays<Message: Clone>(
    frame: &mut UiFrame<Message>,
    state: &mut UiStateStore,
    overlays: Vec<FrameOverlay<Message>>,
) -> Vec<OverlayDeclarationFailure> {
    let mut failures = Vec::new();
    for overlay in overlays {
        match overlay {
            FrameOverlay::Menu(menu) => {
                let id = menu.id.clone();
                let anchor = menu.anchor.id().clone();
                if let Err(error) = frame.present_menu(state, menu) {
                    failures.push(OverlayDeclarationFailure {
                        overlay: id,
                        anchor,
                        error,
                    });
                }
            }
            FrameOverlay::Surface(surface) => {
                let id = surface.id.clone();
                let anchor = surface.anchor.id().clone();
                if let Err(error) = frame.present_transient_surface(state, surface) {
                    failures.push(OverlayDeclarationFailure {
                        overlay: id,
                        anchor,
                        error,
                    });
                }
            }
            FrameOverlay::ContentSurface { surface, content } => {
                let id = surface.id.clone();
                let anchor = surface.anchor.id().clone();
                if let Err(error) = frame.present_transient_content(state, surface, *content) {
                    failures.push(OverlayDeclarationFailure {
                        overlay: id,
                        anchor,
                        error,
                    });
                }
            }
            FrameOverlay::SelectionMarquee {
                rect,
                fill,
                stroke,
                width,
            } => {
                frame.selection_marquee_layer(rect, fill, stroke, width);
            }
        }
    }
    frame.finalize_transient_layers(state);
    failures
}

pub fn run<A: Application>(application: A) -> Result<(), Box<dyn Error>> {
    run_with_adapter(application, DefaultHostAdapter)
}

pub fn run_with_adapter<A: Application>(
    application: A,
    adapter: impl HostAdapter<A>,
) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    let display = event_loop.owned_display_handle();
    let mut runtime = ApplicationRuntime::new(application, adapter, display);
    event_loop.run_app(&mut runtime)?;
    if let Some(error) = runtime.error {
        Err(error)
    } else {
        Ok(())
    }
}

struct ApplicationRuntime<A: Application, H: HostAdapter<A>> {
    host: Option<UiHost<A>>,
    application: Option<A>,
    adapter: H,
    display: OwnedDisplayHandle,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Arc<Window>>>,
    renderer: Option<SoftwareRenderer>,
    input: nickel_input::winit::Adapter,
    clipboard: Option<arboard::Clipboard>,
    controller: ControllerInput,
    controller_schedule: ControllerPollSchedule,
    next_caret_blink: Instant,
    next_adapter_poll: Option<Instant>,
    scheduler: PresentScheduler,
    pointer_icon: PointerIcon,
    scale: f32,
    stopped: bool,
    error: Option<Box<dyn Error>>,
    pending_continuous_input: Vec<nickel_input::InputEvent>,
}

impl<A: Application, H: HostAdapter<A>> ApplicationRuntime<A, H> {
    fn new(application: A, adapter: H, display: OwnedDisplayHandle) -> Self {
        let now = Instant::now();
        let next_adapter_poll = adapter.poll_interval().map(|interval| now + interval);
        Self {
            host: None,
            application: Some(application),
            adapter,
            display,
            window: None,
            surface: None,
            renderer: None,
            input: nickel_input::winit::Adapter::default(),
            clipboard: arboard::Clipboard::new().ok(),
            controller: ControllerInput::new(),
            controller_schedule: ControllerPollSchedule::new(now),
            next_caret_blink: now + Duration::from_millis(500),
            next_adapter_poll,
            scheduler: PresentScheduler::default(),
            pointer_icon: PointerIcon::Default,
            scale: 1.0,
            stopped: false,
            error: None,
            pending_continuous_input: Vec::new(),
        }
    }

    fn apply_input_outcome(&mut self, window: &Window, outcome: HostEventOutcome) {
        window.set_ime_allowed(outcome.text_input_active);
        if let Some(text) = outcome.clipboard_text
            && let Some(clipboard) = &mut self.clipboard
        {
            let _ = clipboard.set_text(text);
        }
        if let Some(host) = &self.host {
            let next_icon = host.inspect().pointer_icon;
            if next_icon != self.pointer_icon {
                window.set_cursor(match next_icon {
                    PointerIcon::Default => CursorIcon::Default,
                    PointerIcon::Hand => CursorIcon::Pointer,
                    PointerIcon::Text => CursorIcon::Text,
                });
                self.pointer_icon = next_icon;
            }
        }
        if outcome.changed {
            self.next_caret_blink = Instant::now() + Duration::from_millis(500);
            self.scheduler.invalidate();
        }
    }

    fn dispatch_normalized_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: &Window,
        inputs: Vec<nickel_input::InputEvent>,
        clipboard_text: Option<String>,
    ) {
        let mut events = Vec::with_capacity(inputs.len());
        let mut adapter_changed = false;
        let mut adapter_exit = false;
        for input in inputs {
            let adapted = self.host.as_mut().map(|host| {
                self.adapter
                    .normalized_input(host, &input, HostServices { window })
            });
            let consume = match adapted {
                Some(Ok(outcome)) => {
                    let consume = outcome.consume;
                    adapter_changed |= outcome.changed;
                    adapter_exit |= outcome.exit;
                    consume
                }
                Some(Err(error)) => {
                    self.fail(event_loop, error);
                    return;
                }
                None => return,
            };
            if !consume {
                events.push(HostEvent::Normalized {
                    input,
                    clipboard_text: clipboard_text.clone(),
                });
            }
        }
        if adapter_exit {
            event_loop.exit();
        }
        if events.is_empty() && !adapter_changed {
            return;
        }
        let Some(host) = &mut self.host else { return };
        let outcome = host.step(HostBatch {
            events,
            application_changed: adapter_changed,
            ..HostBatch::default()
        });
        self.apply_input_outcome(window, outcome);
        self.start_pending_file_drag(window);
    }

    fn start_pending_file_drag(&mut self, window: &Window) {
        #[cfg(target_os = "windows")]
        let _ = window;
        let Some(drag) = self
            .host
            .as_mut()
            .and_then(|host| host.application_mut().take_outbound_file_drag())
        else {
            return;
        };
        #[cfg(target_os = "linux")]
        {
            use winit::platform::wayland::WindowExtWayland;
            let payload = file_uri_list(&drag.paths);
            if let Err(error) = window.start_file_drag(payload) {
                tracing::warn!(%error, "could not begin native Wayland file drag");
            }
        }
        #[cfg(target_os = "windows")]
        if let Err(error) = start_windows_file_drag(&drag.paths) {
            tracing::warn!(%error, "could not begin native Windows file drag");
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let _ = (drag, window);
    }

    fn flush_pending_continuous_input(&mut self, event_loop: &ActiveEventLoop, window: &Window) {
        if self.pending_continuous_input.is_empty() {
            return;
        }
        let inputs = std::mem::take(&mut self.pending_continuous_input);
        self.dispatch_normalized_input(event_loop, window, inputs, None);
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl Into<Box<dyn Error>>) {
        self.error = Some(error.into());
        event_loop.exit();
    }

    fn apply_adapter_outcome(&mut self, event_loop: &ActiveEventLoop, outcome: AdapterOutcome) {
        if outcome.changed
            && let Some(host) = &mut self.host
        {
            host.rebuild();
            self.scheduler.invalidate();
        }
        if outcome.exit {
            event_loop.exit();
        }
    }

    fn present(&mut self) -> Result<(), Box<dyn Error>> {
        let (Some(host), Some(surface), Some(renderer), Some(window)) = (
            self.host.as_ref(),
            self.surface.as_mut(),
            self.renderer.as_mut(),
            self.window.as_ref(),
        ) else {
            return Ok(());
        };
        let size = window.inner_size();
        let width = NonZeroU32::new(size.width.max(1)).expect("clamped non-zero width");
        let height = NonZeroU32::new(size.height.max(1)).expect("clamped non-zero height");
        surface.resize(width, height)?;
        renderer.resize(width.get(), height.get(), self.scale);
        if renderer.render(host.commands()).is_empty() {
            return Ok(());
        }
        let mut buffer = surface.buffer_mut()?;
        for (target, pixel) in buffer.iter_mut().zip(renderer.pixels()) {
            *target = u32::from(pixel.r) << 16 | u32::from(pixel.g) << 8 | u32::from(pixel.b);
        }
        buffer.present()?;
        Ok(())
    }

    fn tick(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let Some(window) = self.window.clone() else {
            return;
        };
        if now >= self.next_caret_blink {
            self.next_caret_blink = now + Duration::from_millis(500);
            if self
                .host
                .as_mut()
                .is_some_and(|host| host.handle_event(UiEvent::CaretBlink).changed)
            {
                self.scheduler.invalidate();
            }
        }
        if self
            .host
            .as_ref()
            .and_then(UiHost::next_deadline)
            .is_some_and(|deadline| now >= deadline)
            && self.host.as_mut().is_some_and(|host| {
                host.step(HostBatch {
                    now: Some(now),
                    events: vec![HostEvent::Poll],
                    ..HostBatch::default()
                })
                .changed
            })
        {
            self.scheduler.invalidate();
        }
        let adapter_due = self
            .next_adapter_poll
            .is_some_and(|deadline| now >= deadline)
            || self
                .adapter
                .next_deadline(now)
                .is_some_and(|deadline| now >= deadline);
        if adapter_due {
            let outcome = self
                .host
                .as_mut()
                .map(|host| self.adapter.poll(host, HostServices { window: &window }));
            match outcome {
                Some(Ok(outcome)) => self.apply_adapter_outcome(event_loop, outcome),
                Some(Err(error)) => {
                    self.fail(event_loop, error);
                    return;
                }
                None => {}
            }
            self.next_adapter_poll = self.adapter.poll_interval().map(|interval| now + interval);
        }
        if self.controller_schedule.is_due(now) {
            let focused = self
                .host
                .as_ref()
                .is_some_and(|host| host.inspect().window_focused);
            let actions = self.controller.poll(now, focused);
            if let Some(family) = self.controller.active_family()
                && self
                    .host
                    .as_mut()
                    .is_some_and(|host| host.set_controller_family(family))
            {
                self.scheduler.invalidate();
            }
            for action in actions {
                let Some(host) = self.host.as_mut() else {
                    break;
                };
                let outcome = host.handle_controller_action(action);
                if outcome.changed {
                    self.scheduler.invalidate();
                }
                for action in outcome.global_actions {
                    match self
                        .adapter
                        .global_action(host, action, HostServices { window: &window })
                    {
                        Ok(outcome) => {
                            if outcome.changed {
                                host.rebuild();
                                self.scheduler.invalidate();
                            }
                            if outcome.exit {
                                event_loop.exit();
                            }
                        }
                        Err(error) => {
                            self.fail(event_loop, error);
                            return;
                        }
                    }
                }
            }
            self.controller_schedule
                .mark_polled(now, self.controller.connected());
        }
        if self.scheduler.request_present() {
            window.request_redraw();
        }
        let deadline = wait_duration(
            now,
            [
                Some(self.next_caret_blink),
                self.host.as_ref().and_then(UiHost::next_deadline),
                self.next_adapter_poll,
                self.adapter.next_deadline(now),
                Some(self.controller_schedule.deadline()),
            ],
        )
        .map(|wait| now + wait);
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        if let (Some(host), Some(window)) = (self.host.as_mut(), self.window.as_ref()) {
            if let Err(error) = self.adapter.stopped(host, HostServices { window }) {
                self.error.get_or_insert(error);
            }
            host.shutdown();
        }
        self.surface = None;
        self.window = None;
    }
}

impl<A: Application, H: HostAdapter<A>> ApplicationHandler for ApplicationRuntime<A, H> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let application = self
            .application
            .as_ref()
            .expect("application before first resume");
        let (width, height) = application.initial_size();
        let attributes = WindowAttributes::default()
            .with_title(application.title())
            .with_inner_size(LogicalSize::new(width, height))
            .with_resizable(true);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        self.scale = window.scale_factor() as f32;
        self.input.set_scale_factor(window.scale_factor());
        let logical = window.inner_size().to_logical::<u32>(window.scale_factor());
        let mut host = UiHost::new(
            self.application.take().expect("application available"),
            logical.width,
            logical.height,
        );
        host.set_scale_factor(self.scale);
        match self
            .adapter
            .started(&mut host, HostServices { window: &window })
        {
            Ok(outcome) => {
                if outcome.changed {
                    host.rebuild();
                }
                if outcome.exit {
                    event_loop.exit();
                }
            }
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        }
        let context = match softbuffer::Context::new(self.display.clone()) {
            Ok(context) => context,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        let surface = match softbuffer::Surface::new(&context, window.clone()) {
            Ok(surface) => surface,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        let size = window.inner_size();
        self.renderer = Some(SoftwareRenderer::new_pixel_buffer(
            size.width,
            size.height,
            self.scale,
        ));
        self.surface = Some(surface);
        self.host = Some(host);
        self.window = Some(window.clone());
        self.scheduler.invalidate();
        if self.scheduler.request_present() {
            window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if window.id() != window_id {
            return;
        }
        let adapted = self.host.as_mut().map(|host| {
            self.adapter
                .event(host, &event, HostServices { window: &window })
        });
        let consume = match adapted {
            Some(Ok(outcome)) => {
                let consume = outcome.consume;
                self.apply_adapter_outcome(event_loop, outcome);
                consume
            }
            Some(Err(error)) => {
                self.fail(event_loop, error);
                return;
            }
            None => return,
        };
        if consume {
            return;
        }
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                self.flush_pending_continuous_input(event_loop, &window);
                if self.scheduler.begin_present()
                    && let Err(error) = self.present()
                {
                    self.fail(event_loop, error);
                }
                return;
            }
            WindowEvent::Resized(size) => {
                let logical = size.to_logical::<u32>(window.scale_factor());
                if let Some(host) = &mut self.host {
                    host.resize(logical.width, logical.height);
                }
                self.scheduler.invalidate();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = *scale_factor as f32;
                self.input.set_scale_factor(*scale_factor);
                let logical = window.inner_size().to_logical::<u32>(*scale_factor);
                if let Some(host) = &mut self.host {
                    host.resize(logical.width, logical.height);
                    host.set_scale_factor(self.scale);
                }
                self.scheduler.invalidate();
            }
            WindowEvent::Occluded(true) => {
                if let Some(host) = &mut self.host {
                    host.handle_event(UiEvent::Suspended);
                }
                if let Some(renderer) = &mut self.renderer {
                    renderer.suspend();
                }
                return;
            }
            WindowEvent::Occluded(false) => self.scheduler.invalidate(),
            WindowEvent::HoveredFile(path) => {
                if self.host.as_mut().is_some_and(|host| {
                    host.application_mut()
                        .file_drag_event(FileDragEvent::Hovered(path.clone()))
                }) {
                    self.scheduler.invalidate();
                }
            }
            WindowEvent::HoveredFileCancelled => {
                if self.host.as_mut().is_some_and(|host| {
                    host.application_mut()
                        .file_drag_event(FileDragEvent::HoverCancelled)
                }) {
                    self.scheduler.invalidate();
                }
            }
            WindowEvent::DroppedFile(path) => {
                if self.host.as_mut().is_some_and(|host| {
                    host.application_mut()
                        .file_drag_event(FileDragEvent::Dropped(path.clone()))
                }) {
                    self.scheduler.invalidate();
                }
            }
            _ => {}
        }
        for normalized in self.input.normalize(nickel_input::DeviceId(0), &event) {
            if matches!(
                normalized,
                nickel_input::InputEvent::Pointer(
                    nickel_input::PointerEvent::Axis { .. }
                        | nickel_input::PointerEvent::Motion { .. }
                )
            ) {
                queue_continuous_input(&mut self.pending_continuous_input, normalized);
                self.scheduler.invalidate();
                continue;
            }
            self.flush_pending_continuous_input(event_loop, &window);
            let image_pasted = if is_clipboard_paste(&normalized) {
                self.clipboard
                    .as_mut()
                    .and_then(|clipboard| clipboard.get_image().ok())
                    .and_then(|image| {
                        let width = u32::try_from(image.width).ok()?;
                        let height = u32::try_from(image.height).ok()?;
                        self.host.as_mut().map(|host| {
                            host.paste_clipboard_image(width, height, image.bytes.as_ref())
                        })
                    })
                    .unwrap_or(false)
            } else {
                false
            };
            if image_pasted {
                self.scheduler.invalidate();
                continue;
            }
            let clipboard_text = self
                .clipboard
                .as_mut()
                .and_then(|clipboard| clipboard.get_text().ok());
            self.dispatch_normalized_input(event_loop, &window, vec![normalized], clipboard_text);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.tick(event_loop);
    }
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.stop();
    }
}

fn is_clipboard_paste(input: &nickel_input::InputEvent) -> bool {
    matches!(input, nickel_input::InputEvent::Key(key)
        if key.edge == nickel_input::KeyEdge::Pressed
        && matches!(key.logical, nickel_input::LogicalKey::Character(ref value) if value.eq_ignore_ascii_case("v"))
        && (key.modifiers.aggregate(nickel_input::AggregateModifier::Control)
            || key.modifiers.aggregate(nickel_input::AggregateModifier::Super)))
}

#[cfg(test)]
mod tests {
    use nickel_input::{
        DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, KeyEvent, KeyLocation, LogicalKey,
        Modifier, ModifierState, NamedKey, PhysicalKey, Point, PointerButton, PointerEvent,
        TextEvent, TouchEvent, TouchId, Vector,
    };
    use std::time::{Duration, Instant};

    use super::{
        Application, Completion, CompletionFailure, CompletionFailureKind, ControllerPollSchedule,
        EffectEvidence, FrameOverlay, GlobalAction, HostBatch, HostEvent, HostFailure,
        HostFailureStage, MessageEvidence, PresentScheduler, Shortcut, UiHost, ViewContext,
        queue_continuous_input, wait_duration,
    };
    use crate::{
        ActionKind, Button, Container, ControllerAction, InputModality, Invalidation,
        NavigationEntry, NavigationScope, OverlayId, SemanticAction, SemanticActionError,
        SemanticRole, SemanticValueInput, TextField, UiEvent, UiId, UiStateStore,
    };

    #[derive(Clone)]
    enum Message {
        Changed(String),
    }

    #[derive(Default)]
    struct InputApplication {
        text: String,
        submits: usize,
    }

    struct MultilineInputApplication {
        text: String,
    }

    struct SecureInputApplication {
        text: String,
    }

    #[derive(Default)]
    struct ControllerApplication;

    impl Application for ControllerApplication {
        type Message = ();

        fn update(&mut self, (): Self::Message) {}

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            Button::new((), "Activate")
        }
    }

    struct NavigationApplication;

    impl Application for NavigationApplication {
        type Message = ();

        fn update(&mut self, (): Self::Message) {}

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            Container::new()
                .id("scope")
                .navigation_scope(NavigationScope::group().entry(NavigationEntry::Last))
                .children([
                    Button::new((), "First").id("first"),
                    Button::new((), "Last").id("last"),
                ])
        }
    }

    #[derive(Default)]
    struct ResponsiveApplication;

    impl Application for ResponsiveApplication {
        type Message = ();

        fn update(&mut self, (): Self::Message) {}

        fn view(&self, context: ViewContext) -> impl crate::View<Self::Message> {
            let label = if context.modality == crate::InputModality::Controller {
                "Controller"
            } else if context.viewport.size.width < 200.0 {
                "Narrow"
            } else {
                "Wide"
            };
            Button::new((), label)
        }
    }

    #[derive(Default)]
    struct CompletionApplication(u32);

    impl Application for CompletionApplication {
        type Message = ();

        fn update(&mut self, (): Self::Message) {}

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            Button::new((), self.0.to_string())
        }

        fn complete(&mut self, completion: Completion) -> Result<bool, CompletionFailure> {
            let id = completion.id;
            let value = completion
                .downcast::<u32>()
                .map_err(|_| CompletionFailure {
                    id,
                    kind: CompletionFailureKind::TypeMismatch,
                    detail: "expected u32".into(),
                })?;
            self.0 = value;
            Ok(true)
        }
    }

    #[derive(Default)]
    struct EffectApplication {
        pending: Vec<EffectEvidence>,
    }

    impl Application for EffectApplication {
        type Message = ();

        fn update(&mut self, (): Self::Message) {
            self.pending.push(EffectEvidence {
                type_name: "test.effect",
                label: Some("activated".into()),
            });
        }

        fn take_effect_evidence(&mut self) -> Vec<EffectEvidence> {
            std::mem::take(&mut self.pending)
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            Button::new((), "Effect")
        }
    }

    impl Application for InputApplication {
        type Message = Message;

        fn update(&mut self, message: Self::Message) {
            match message {
                Message::Changed(text) => self.text = text,
            }
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            TextField::on_change(&self.text, Message::Changed)
        }

        fn shortcut(&mut self, shortcut: Shortcut) -> bool {
            if shortcut != Shortcut::Submit {
                return false;
            }
            self.submits += 1;
            true
        }
    }

    impl Application for MultilineInputApplication {
        type Message = Message;

        fn update(&mut self, message: Self::Message) {
            let Message::Changed(text) = message;
            self.text = text;
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            TextField::on_change(&self.text, Message::Changed).wrap(true)
        }
    }

    impl Application for SecureInputApplication {
        type Message = Message;

        fn update(&mut self, message: Self::Message) {
            let Message::Changed(text) = message;
            self.text = text;
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            TextField::on_change_masked(&self.text, '•', Message::Changed)
        }
    }

    fn key(order: u64, repeat: bool) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(order),
            physical: PhysicalKey::Code(KeyCode::Enter),
            logical: LogicalKey::Named(NamedKey::Enter),
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat,
            modifiers: ModifierState::default(),
        })
    }

    fn command_key(order: u64, physical: KeyCode, logical: &str) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(order),
            physical: PhysicalKey::Code(physical),
            logical: LogicalKey::Character(logical.into()),
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat: false,
            modifiers: ModifierState::from_sides([Modifier::ControlLeft]),
        })
    }

    fn focus_event() -> InputEvent {
        InputEvent::Pointer(PointerEvent::Button {
            device: DeviceId(2),
            order: EventOrder(1),
            button: PointerButton::Primary,
            edge: KeyEdge::Pressed,
            position: Some(Point { x: 4.0, y: 4.0 }),
        })
    }

    #[test]
    fn idle_frames_do_not_present_and_present_requests_coalesce() {
        let mut scheduler = PresentScheduler::default();
        assert!(!scheduler.request_present());
        scheduler.invalidate();
        scheduler.invalidate();
        scheduler.invalidate();
        assert!(scheduler.request_present());
        assert!(!scheduler.request_present());
        scheduler.invalidate();
        assert!(!scheduler.request_present());
        assert!(scheduler.begin_present());
        assert!(!scheduler.begin_present());
    }

    #[test]
    fn continuous_pointer_motion_keeps_latest_position_and_total_delta() {
        let mut pending = Vec::new();
        for (order, position, delta) in [
            (1, Point { x: 10.0, y: 20.0 }, Vector { x: 1.0, y: 2.0 }),
            (2, Point { x: 14.0, y: 26.0 }, Vector { x: 4.0, y: 6.0 }),
        ] {
            queue_continuous_input(
                &mut pending,
                InputEvent::Pointer(PointerEvent::Motion {
                    device: DeviceId(7),
                    order: EventOrder(order),
                    position,
                    delta: Some(delta),
                }),
            );
        }

        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0],
            InputEvent::Pointer(PointerEvent::Motion {
                device: DeviceId(7),
                order: EventOrder(2),
                position: Point { x: 14.0, y: 26.0 },
                delta: Some(Vector { x: 5.0, y: 8.0 }),
            })
        );
    }

    #[test]
    fn host_telemetry_reports_phases_and_explicit_allocation_unavailability() {
        let mut host = UiHost::new(ControllerApplication, 320, 200);
        let outcome = host.step(HostBatch {
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        assert_eq!(outcome.telemetry.scheduled_wakeups, 1);
        assert!(outcome.telemetry.input_to_frame_us >= outcome.telemetry.input_to_message_us);
        assert!(outcome.telemetry.retained_frame_bytes > 0);
        assert_eq!(outcome.telemetry.allocation_count, None);
    }

    #[test]
    fn ui_host_dispatches_declared_scope_entry_through_the_production_frame() {
        let mut host = UiHost::new(NavigationApplication, 320, 200);
        host.handle_event(UiEvent::ControllerDown);
        host.handle_event(UiEvent::ControllerActivate);
        let selected = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.controller_selected)
            .expect("declared entry selects a semantic target");
        assert!(selected.id.as_str().ends_with("/last"));
        assert_eq!(selected.navigation_depth, 1);
    }

    #[test]
    fn host_batch_drains_typed_effects_and_reports_change_token() {
        let mut host = UiHost::new(EffectApplication::default(), 160, 48);
        let outcome = host.perform_semantic_action(
            UiId::from("root"),
            SemanticAction::Invoke(ActionKind::Activate),
        );
        assert_eq!(outcome.effects.len(), 1);
        assert_eq!(outcome.effects[0].type_name, "test.effect");
        assert_eq!(
            outcome.change_token.frame_generation,
            host.inspect().frame_generation
        );
        assert_eq!(
            outcome.change_token.semantic_generation,
            host.inspect().semantic_generation
        );

        let idle = host.step(HostBatch::default());
        assert!(
            idle.effects.is_empty(),
            "effects must be drained exactly once"
        );
        assert_eq!(idle.change_token, outcome.change_token);
    }

    #[test]
    fn event_wait_uses_the_earliest_declared_deadline_and_can_sleep_indefinitely() {
        let now = Instant::now();
        assert_eq!(wait_duration(now, [None, None]), None);
        assert_eq!(
            wait_duration(
                now,
                [
                    Some(now + Duration::from_millis(250)),
                    Some(now + Duration::from_millis(17)),
                    Some(now + Duration::from_secs(1)),
                ],
            ),
            Some(Duration::from_millis(17))
        );
    }

    #[test]
    fn application_poll_deadline_is_host_owned_and_advances_from_batch_time() {
        struct PollApplication;
        impl Application for PollApplication {
            type Message = ();
            fn update(&mut self, (): Self::Message) {}
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                crate::Container::new()
            }
            fn poll_interval(&self) -> Option<Duration> {
                Some(Duration::from_millis(40))
            }
        }

        let origin = Instant::now();
        let mut host = UiHost::new_at(PollApplication, 100, 40, origin);
        assert_eq!(
            host.next_deadline(),
            Some(origin + Duration::from_millis(40))
        );
        let polled_at = origin + Duration::from_millis(45);
        let outcome = host.step(HostBatch {
            now: Some(polled_at),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        assert_eq!(
            outcome.next_deadline,
            Some(polled_at + Duration::from_millis(40))
        );
        assert_eq!(host.next_deadline(), outcome.next_deadline);
    }

    #[test]
    fn application_work_started_by_message_arms_a_new_poll_deadline() {
        struct DeferredApplication {
            pending: bool,
        }
        impl Application for DeferredApplication {
            type Message = ();
            fn update(&mut self, (): Self::Message) {
                self.pending = true;
            }
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new((), "Start work")
            }
            fn poll_interval(&self) -> Option<Duration> {
                self.pending.then_some(Duration::from_millis(16))
            }
        }

        let origin = Instant::now();
        let mut host = UiHost::new_at(DeferredApplication { pending: false }, 100, 40, origin);
        assert_eq!(host.next_deadline(), None);
        let button = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Start work"))
            .expect("button is a semantic target");
        host.perform_semantic_action(button.id, SemanticAction::Invoke(ActionKind::Activate));

        assert!(host.next_deadline().is_some());
    }

    #[test]
    fn controller_poll_schedule_has_one_shared_bounded_cadence() {
        let now = Instant::now();
        let mut schedule = ControllerPollSchedule::new(now);
        assert!(schedule.is_due(now));
        schedule.mark_polled(now, true);
        assert_eq!(
            schedule.deadline(),
            now + ControllerPollSchedule::CONNECTED_INTERVAL
        );
        schedule.mark_polled(now, false);
        assert_eq!(
            schedule.deadline(),
            now + ControllerPollSchedule::DISCONNECTED_INTERVAL
        );
    }

    #[test]
    fn ui_host_batches_changes_into_one_frame_and_idle_steps_do_nothing() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        let initial = host.inspect();
        assert_eq!(initial.frame_generation, 1);
        assert_eq!(initial.resources.retained_build_scratch_bytes, 0);

        let idle = host.step(HostBatch::default());
        assert!(!idle.changed);
        assert_eq!(host.inspect().frame_generation, 1);

        let changed = host.step(HostBatch {
            events: vec![
                HostEvent::Shortcut(Shortcut::Submit),
                HostEvent::Shortcut(Shortcut::Submit),
            ],
            ..HostBatch::default()
        });
        assert!(changed.changed);
        assert_eq!(changed.invalidation, Invalidation::Layout);
        assert_eq!(host.application_mut().submits, 2);
        assert_eq!(host.inspect().frame_generation, 2);
        assert_eq!(host.inspect().resources.retained_build_scratch_bytes, 0);
    }

    #[test]
    fn explicit_application_change_rebuilds_at_an_unchanged_surface_size() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        host.application_mut().text = "loaded after construction".into();

        let outcome = host.step(HostBatch {
            application_changed: true,
            surface_size: Some((320, 48)),
            ..HostBatch::default()
        });

        assert!(outcome.changed);
        assert!(outcome.telemetry.rebuilt);
        assert_eq!(host.inspect().frame_generation, 2);
        assert!(host.commands().iter().any(|command| {
            matches!(command, crate::PaintCommand::Text { text, .. } if text == "loaded after construction")
        }));
    }

    #[test]
    fn typed_completions_are_applied_in_the_same_batch_before_one_rebuild() {
        let mut host = UiHost::new(CompletionApplication::default(), 160, 48);
        let outcome = host.step(HostBatch {
            completions: vec![Completion::new("loaded-count", 7_u32)],
            ..HostBatch::default()
        });
        assert!(outcome.changed);
        assert_eq!(outcome.invalidation, Invalidation::Layout);
        assert!(outcome.completion_failures.is_empty());
        assert_eq!(host.application().0, 7);
        assert_eq!(host.inspect().frame_generation, 2);

        let rejected = host.step(HostBatch {
            completions: vec![Completion::new("loaded-count", "wrong type")],
            ..HostBatch::default()
        });
        assert!(!rejected.changed);
        assert_eq!(
            rejected.completion_failures[0].kind,
            CompletionFailureKind::TypeMismatch
        );
    }

    #[test]
    fn declared_dialog_surface_renders_in_host_stack_and_cancel_restores_focus() {
        struct DialogApplication {
            confirmations: usize,
        }
        impl Application for DialogApplication {
            type Message = bool;
            fn update(&mut self, confirmed: Self::Message) {
                self.confirmations += usize::from(confirmed);
            }
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new(false, "Open").id("anchor")
            }
            fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
                vec![FrameOverlay::surface(
                    crate::TransientSurface::dialog(
                        "dialog",
                        crate::OverlayAnchor::Node(UiId::from("anchor")),
                        crate::Size::new(120.0, 80.0),
                        crate::OverlayStyle {
                            background: 0x111111,
                            foreground: 0xffffff,
                            border: 0x888888,
                            selected: 0x333333,
                            radius: 8,
                        },
                    ),
                    Button::new(true, "Confirm"),
                )]
            }
        }
        let mut host = UiHost::new(DialogApplication { confirmations: 0 }, 320, 200);
        host.request_focus(UiId::from("root/anchor"));
        assert!(host.open_transient(OverlayId::new("dialog"), UiId::from("root/anchor")));
        assert!(
            host.semantic_nodes()
                .iter()
                .any(|node| node.role == Some(SemanticRole::Dialog))
        );
        let confirm = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Confirm"))
            .expect("dialog content shares the semantic frame");
        host.handle_event(UiEvent::FocusNext);
        assert_eq!(host.inspect().keyboard_focus, Some(confirm.id.clone()));
        host.handle_event(UiEvent::FocusNext);
        assert_eq!(host.inspect().keyboard_focus, Some(confirm.id.clone()));
        let activated =
            host.perform_semantic_action(confirm.id, SemanticAction::Invoke(ActionKind::Activate));
        assert!(activated.changed);
        assert_eq!(host.application().confirmations, 1);
        host.handle_event(UiEvent::ControllerBack);
        assert!(host.inspect().open_overlay.is_none());
        assert_eq!(
            host.inspect().keyboard_focus,
            Some(UiId::from("root/anchor"))
        );
    }

    #[test]
    fn public_popover_and_tooltip_use_named_canonical_transient_surfaces() {
        struct PopoverApplication;
        impl Application for PopoverApplication {
            type Message = ();
            fn update(&mut self, (): Self::Message) {}
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new((), "Details").id("anchor")
            }
            fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
                vec![
                    super::Popover::new(
                        "details-popover",
                        crate::OverlayAnchor::InvocationTarget(UiId::from("anchor")),
                        "Application details",
                        crate::Size::new(80.0, 40.0),
                        crate::OverlayStyle {
                            background: 0x111111,
                            foreground: 0xffffff,
                            border: 0x888888,
                            selected: 0x333333,
                            radius: 8,
                        },
                        Button::new((), "Close"),
                    )
                    .placement(crate::OverlayPlacement::Before)
                    .direction(crate::ReadingDirection::RightToLeft)
                    .scale(1.5)
                    .into(),
                ]
            }
        }

        let mut host = UiHost::new(PopoverApplication, 320, 200);
        let anchor = host
            .query_unique(&crate::SemanticSelector::Role(SemanticRole::Button))
            .expect("popover anchor");
        let anchor_id = anchor.id.clone();
        host.request_focus(anchor_id.clone());
        assert!(host.open_transient(OverlayId::new("details-popover"), anchor.id));
        let popover = host
            .query_unique(&crate::SemanticSelector::RoleAndName {
                role: SemanticRole::Popover,
                name: "Application details".into(),
            })
            .expect("named popover");
        assert_eq!(popover.bounds.size, crate::Size::new(120.0, 60.0));
        assert!(host.accessibility_nodes().iter().any(|node| {
            node.role.as_deref() == Some("popover")
                && node.label.as_deref() == Some("Application details")
        }));
        host.handle_event(UiEvent::FocusNext);
        assert_ne!(host.inspect().keyboard_focus.as_ref(), Some(&anchor_id));
        host.handle_event(UiEvent::ControllerBack);
        assert!(host.inspect().open_overlay.is_none());
        assert_eq!(host.inspect().keyboard_focus.as_ref(), Some(&anchor_id));

        struct TooltipApplication;
        impl Application for TooltipApplication {
            type Message = ();
            fn update(&mut self, (): Self::Message) {}
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new((), "Help").id("anchor")
            }
            fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
                vec![
                    super::Tooltip::new(
                        "help-tooltip",
                        crate::OverlayAnchor::InvocationTarget(UiId::from("anchor")),
                        "Explains this control",
                        crate::Size::new(100.0, 32.0),
                        crate::OverlayStyle {
                            background: 0x111111,
                            foreground: 0xffffff,
                            border: 0x888888,
                            selected: 0x333333,
                            radius: 8,
                        },
                        crate::Text::new("Keyboard shortcut: F1"),
                    )
                    .placement(crate::OverlayPlacement::Above)
                    .into(),
                ]
            }
        }

        let mut host = UiHost::new(TooltipApplication, 320, 200);
        let anchor = host
            .query_unique(&crate::SemanticSelector::Role(SemanticRole::Button))
            .expect("tooltip anchor");
        let anchor_id = anchor.id.clone();
        assert!(host.request_focus(anchor_id.clone()).changed);
        assert!(host.open_transient(OverlayId::new("help-tooltip"), anchor.id));
        assert!(
            host.query_unique(&crate::SemanticSelector::RoleAndName {
                role: SemanticRole::Tooltip,
                name: "Explains this control".into(),
            })
            .is_ok()
        );
        assert!(host.accessibility_nodes().iter().any(|node| {
            node.role.as_deref() == Some("tooltip")
                && node.label.as_deref() == Some("Explains this control")
        }));
        assert_eq!(host.inspect().keyboard_focus.as_ref(), Some(&anchor_id));
    }

    #[test]
    fn ui_host_semantic_actions_update_rebuild_and_report_failures_transactionally() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        let text_field = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.role == Some(SemanticRole::TextField))
            .expect("text field semantics");
        let changed = host.perform_semantic_action(
            text_field.id.clone(),
            SemanticAction::SetValue(SemanticValueInput::Text("semantic".into())),
        );
        assert!(changed.changed);
        assert!(changed.semantic_failures.is_empty());
        assert_eq!(host.application_mut().text, "semantic");
        assert_eq!(host.inspect().frame_generation, 2);

        let accessible = host.perform_accessibility_action(
            text_field.id.clone(),
            SemanticAction::SetValue(SemanticValueInput::Text("accessible".into())),
        );
        assert_eq!(accessible.messages.len(), 1);
        assert_eq!(host.application().text, "accessible");
        assert_eq!(host.inspect().modality, InputModality::Accessibility);

        let rejected = host
            .perform_semantic_action(text_field.id, SemanticAction::Invoke(ActionKind::Activate));
        assert!(!rejected.changed);
        assert_eq!(rejected.semantic_failures.len(), 1);
        assert_eq!(
            rejected.semantic_failures[0].error,
            SemanticActionError::ActionUnavailable
        );
        assert_eq!(host.inspect().frame_generation, 3);

        let missing = host.perform_semantic_action(
            UiId::from("missing"),
            SemanticAction::Invoke(ActionKind::Activate),
        );
        assert_eq!(
            missing.semantic_failures[0].error,
            SemanticActionError::MissingTarget
        );
    }

    #[test]
    fn an_unfocused_caret_tick_does_not_invalidate_the_window() {
        let mut state = UiStateStore::default();

        assert_eq!(state.toggle_caret(), Invalidation::None);
    }

    #[test]
    fn embedded_controller_dispatch_respects_window_focus() {
        let mut host = UiHost::new(ControllerApplication, 320, 48);
        host.handle_event(crate::UiEvent::FocusGained);
        assert!(
            host.handle_controller_action(ControllerAction::Down)
                .changed
        );
        host.handle_event(crate::UiEvent::FocusLost);
        assert!(
            !host
                .handle_controller_action(ControllerAction::Down)
                .changed
        );
        host.handle_event(crate::UiEvent::FocusGained);
        assert!(
            host.handle_controller_action(ControllerAction::Down)
                .changed
        );
    }

    #[test]
    fn focused_button_is_not_reported_as_a_text_editor() {
        let mut host = UiHost::new(ControllerApplication, 320, 48);
        let button = host.semantic_nodes()[0].id.clone();

        assert!(host.request_focus(button).changed);
        assert!(!host.input_context().text_focused);
    }

    #[test]
    fn controller_host_event_is_equivalent_to_the_canonical_ui_transition() {
        let mut controller = UiHost::new(ControllerApplication, 320, 48);
        let mut semantic = UiHost::new(ControllerApplication, 320, 48);
        controller.handle_event(crate::UiEvent::FocusGained);
        semantic.handle_event(crate::UiEvent::FocusGained);

        let controller_outcome = controller.step(HostBatch {
            events: vec![HostEvent::Controller(ControllerAction::Down)],
            ..HostBatch::default()
        });
        let semantic_outcome = semantic.step(HostBatch {
            events: vec![HostEvent::Ui(crate::UiEvent::ControllerDown)],
            ..HostBatch::default()
        });

        assert_eq!(controller_outcome.changed, semantic_outcome.changed);
        assert_eq!(
            controller_outcome.invalidation,
            semantic_outcome.invalidation
        );
        assert_eq!(controller.inspect(), semantic.inspect());

        let activation = controller.handle_controller_action(ControllerAction::Confirm);
        assert_eq!(activation.messages.len(), 1);
        assert_eq!(activation.messages[0].type_name, "()");
    }

    #[test]
    fn normalized_touch_and_direct_pointer_adapters_produce_the_same_host_trace() {
        let mut touch = UiHost::new(ControllerApplication, 160, 48);
        let mut pointer = UiHost::new(ControllerApplication, 160, 48);
        let point = crate::Point { x: 40.0, y: 20.0 };
        let pointer_outcome = pointer.step(HostBatch {
            events: vec![
                HostEvent::Ui(UiEvent::PointerMoved(point)),
                HostEvent::Ui(UiEvent::PointerPressed(point)),
                HostEvent::Ui(UiEvent::PointerReleased(point)),
            ],
            ..HostBatch::default()
        });
        let touch_outcome = touch.step(HostBatch {
            events: vec![
                HostEvent::Normalized {
                    input: InputEvent::Touch(TouchEvent::Started {
                        device: DeviceId(4),
                        order: EventOrder(1),
                        contact: TouchId(1),
                        position: Point { x: 40.0, y: 20.0 },
                    }),
                    clipboard_text: None,
                },
                HostEvent::Normalized {
                    input: InputEvent::Touch(TouchEvent::Ended {
                        device: DeviceId(4),
                        order: EventOrder(2),
                        contact: TouchId(1),
                        position: Point { x: 40.0, y: 20.0 },
                    }),
                    clipboard_text: None,
                },
            ],
            ..HostBatch::default()
        });
        assert_eq!(touch_outcome.messages, pointer_outcome.messages);
        assert_eq!(touch_outcome.invalidation, pointer_outcome.invalidation);
        assert_eq!(touch.inspect(), pointer.inspect());
    }

    #[test]
    fn resize_and_modality_are_supplied_before_the_declarative_rebuild() {
        let mut host = UiHost::new(ResponsiveApplication, 320, 48);
        assert_eq!(host.semantic_nodes()[0].name.as_deref(), Some("Wide"));

        host.resize(160, 48);
        assert_eq!(host.inspect().frame_generation, 2);
        assert_eq!(host.semantic_nodes()[0].name.as_deref(), Some("Narrow"));

        let outcome = host.handle_controller_action(ControllerAction::Down);
        assert!(outcome.changed);
        assert_eq!(host.inspect().modality, crate::InputModality::Controller);
        assert_eq!(host.semantic_nodes()[0].name.as_deref(), Some("Controller"));

        let target = host.semantic_nodes()[0].id.clone();
        let generation = host.inspect().frame_generation;
        assert!(host.request_focus(target.clone()).changed);
        let inspection = host.inspect();
        assert_eq!(inspection.keyboard_focus, Some(target));
        assert_eq!(inspection.modality, crate::InputModality::Controller);
        assert_eq!(inspection.frame_generation, generation + 1);
    }

    #[test]
    fn guide_action_is_reported_globally_without_entering_application_dispatch() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        let outcome = host.handle_controller_action(ControllerAction::Launcher);

        assert!(!outcome.changed);
        assert_eq!(outcome.global_actions, [GlobalAction::ToggleLauncher]);
        assert_eq!(host.inspect().frame_generation, 1);
    }

    #[test]
    fn embedded_host_dispatches_normalized_text_ime_and_submit_once() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        assert!(host.handle_input(&focus_event(), None).changed);
        assert!(host.input_context().text_focused);
        assert!(
            !host
                .handle_input(
                    &InputEvent::FocusGained {
                        order: EventOrder(2),
                    },
                    None,
                )
                .changed
        );
        assert!(host.input_context().text_focused);

        let preedit = InputEvent::Text(TextEvent::Preedit {
            device: DeviceId(1),
            order: EventOrder(3),
            text: "世".into(),
            selection: Some((0, 3)),
        });
        assert!(host.handle_input(&preedit, None).changed);
        assert!(host.application_mut().text.is_empty());

        let commit = InputEvent::Text(TextEvent::Commit {
            device: DeviceId(1),
            order: EventOrder(4),
            text: "世界".into(),
        });
        assert!(host.handle_input(&commit, None).changed);
        assert_eq!(host.application_mut().text, "世界");

        assert!(host.handle_input(&key(5, false), None).changed);
        assert_eq!(host.application_mut().submits, 1);
        assert!(!host.handle_input(&key(6, true), None).changed);
        assert_eq!(host.application_mut().submits, 1);
    }

    #[test]
    fn embedded_host_owns_one_clipboard_command_path() {
        let mut host = UiHost::new(InputApplication::default(), 320, 48);
        host.handle_input(&focus_event(), None);
        host.handle_input(
            &InputEvent::Text(TextEvent::Commit {
                device: DeviceId(1),
                order: EventOrder(2),
                text: "copy me".into(),
            }),
            None,
        );
        host.handle_input(&command_key(3, KeyCode::KeyA, "a"), None);

        let copied = host.handle_input(&command_key(4, KeyCode::KeyC, "c"), None);
        assert_eq!(copied.clipboard_text.as_deref(), Some("copy me"));
        assert_eq!(host.application_mut().text, "copy me");

        let cut = host.handle_input(&command_key(5, KeyCode::KeyX, "x"), None);
        assert_eq!(cut.clipboard_text.as_deref(), Some("copy me"));
        assert!(host.application_mut().text.is_empty());

        assert!(
            host.handle_input(&command_key(6, KeyCode::KeyV, "v"), Some("pasted"))
                .changed
        );
        assert_eq!(host.application_mut().text, "pasted");
    }

    #[test]
    fn consumed_select_all_text_does_not_reach_single_multiline_or_secure_fields() {
        fn exercise<A: Application<Message = Message>>(
            host: &mut UiHost<A>,
            text: impl for<'a> FnOnce(&'a mut A) -> &'a str,
        ) {
            host.handle_input(&focus_event(), None);
            assert!(host.input_context().text_focused);
            host.handle_input(&command_key(2, KeyCode::KeyA, "a"), None);
            let leaked = host.handle_input(
                &InputEvent::Text(TextEvent::Commit {
                    device: DeviceId(1),
                    order: EventOrder(2),
                    text: "a".into(),
                }),
                None,
            );
            assert!(!leaked.changed);
            assert_eq!(text(host.application_mut()), "unchanged");
        }

        let mut single = UiHost::new(
            InputApplication {
                text: "unchanged".into(),
                submits: 0,
            },
            320,
            48,
        );
        exercise(&mut single, |application| &application.text);

        let mut multiline = UiHost::new(
            MultilineInputApplication {
                text: "unchanged".into(),
            },
            320,
            96,
        );
        exercise(&mut multiline, |application| &application.text);

        let mut secure = UiHost::new(
            SecureInputApplication {
                text: "unchanged".into(),
            },
            320,
            48,
        );
        exercise(&mut secure, |application| &application.text);
    }

    #[derive(Clone, Copy, Debug)]
    enum ReplayPath {
        Headless,
        StandaloneAdapter,
        EmbeddedAdapter,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct ReplayProof {
        messages: Vec<MessageEvidence>,
        semantics: Vec<crate::SemanticNodeSnapshot>,
        paint: Vec<crate::PaintCommand>,
        accessibility: Vec<crate::AccessibilityNode>,
        inspection: super::HostInspection,
        deadline_offset: Option<Duration>,
    }

    struct ReplayApplication {
        text: String,
    }

    impl Application for ReplayApplication {
        type Message = Message;

        fn update(&mut self, message: Self::Message) {
            match message {
                Message::Changed(text) => self.text = text,
            }
        }

        fn message_evidence(&self, message: &Self::Message) -> MessageEvidence {
            let Message::Changed(text) = message;
            MessageEvidence {
                type_name: "replay.text.changed",
                label: Some(text.clone()),
            }
        }

        fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
            TextField::on_change(&self.text, Message::Changed)
        }

        fn poll_interval(&self) -> Option<Duration> {
            Some(Duration::from_millis(40))
        }
    }

    fn replay_adapter_path(path: ReplayPath) -> ReplayProof {
        let origin = Instant::now();
        let mut host = UiHost::new_at(
            ReplayApplication {
                text: String::new(),
            },
            320,
            48,
            origin,
        );
        let editor = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.role == Some(SemanticRole::TextField))
            .expect("editor semantic target")
            .id;
        let events = vec![
            HostEvent::Normalized {
                input: focus_event(),
                clipboard_text: None,
            },
            HostEvent::Normalized {
                input: InputEvent::Text(TextEvent::Preedit {
                    device: DeviceId(1),
                    order: EventOrder(2),
                    text: "世".into(),
                    selection: Some((0, 3)),
                }),
                clipboard_text: None,
            },
            HostEvent::Normalized {
                input: InputEvent::Text(TextEvent::Commit {
                    device: DeviceId(1),
                    order: EventOrder(3),
                    text: "world".into(),
                }),
                clipboard_text: None,
            },
            HostEvent::Normalized {
                input: command_key(4, KeyCode::KeyA, "a"),
                clipboard_text: None,
            },
            HostEvent::Normalized {
                input: command_key(5, KeyCode::KeyC, "c"),
                clipboard_text: None,
            },
            HostEvent::Normalized {
                input: command_key(6, KeyCode::KeyX, "x"),
                clipboard_text: None,
            },
            HostEvent::Normalized {
                input: command_key(7, KeyCode::KeyV, "v"),
                clipboard_text: Some("pasted".into()),
            },
            HostEvent::Normalized {
                input: InputEvent::Touch(TouchEvent::Started {
                    device: DeviceId(4),
                    order: EventOrder(8),
                    contact: TouchId(1),
                    position: Point { x: 8.0, y: 8.0 },
                }),
                clipboard_text: None,
            },
            HostEvent::Normalized {
                input: InputEvent::Touch(TouchEvent::Ended {
                    device: DeviceId(4),
                    order: EventOrder(9),
                    contact: TouchId(1),
                    position: Point { x: 8.0, y: 8.0 },
                }),
                clipboard_text: None,
            },
            HostEvent::Controller(ControllerAction::Down),
            HostEvent::Accessibility {
                target: editor,
                action: SemanticAction::SetValue(SemanticValueInput::Text("accessible".into())),
            },
            HostEvent::Shortcut(Shortcut::Submit),
            HostEvent::Controller(ControllerAction::Launcher),
            HostEvent::Ui(UiEvent::FocusLost),
            HostEvent::Poll,
        ];
        let batch = HostBatch {
            now: Some(origin + Duration::from_millis(45)),
            surface_size: Some((480, 72)),
            scale_factor: Some(1.5),
            window_focused: Some(true),
            events,
            ..HostBatch::default()
        };

        // These are deliberately transport-only paths. Standalone windows,
        // embedded shell surfaces, and headless scenarios all surrender the
        // normalized batch to the same host transition authority.
        let outcome = match path {
            ReplayPath::Headless => host.step(batch),
            ReplayPath::StandaloneAdapter => {
                let adapter_batch = batch;
                host.step(adapter_batch)
            }
            ReplayPath::EmbeddedAdapter => {
                let embedded_batch = batch;
                host.step(embedded_batch)
            }
        };
        ReplayProof {
            messages: outcome.messages,
            semantics: host.semantic_nodes(),
            paint: host.commands().to_vec(),
            accessibility: host.accessibility_nodes().to_vec(),
            inspection: host.inspect(),
            deadline_offset: outcome
                .next_deadline
                .map(|deadline| deadline.duration_since(origin)),
        }
    }

    #[test]
    fn one_normalized_trace_is_identical_across_all_host_adapter_paths() {
        let headless = replay_adapter_path(ReplayPath::Headless);
        let standalone = replay_adapter_path(ReplayPath::StandaloneAdapter);
        let embedded = replay_adapter_path(ReplayPath::EmbeddedAdapter);

        assert_eq!(standalone, headless);
        assert_eq!(embedded, headless);
        assert_eq!(headless.deadline_offset, Some(Duration::from_millis(85)));
        assert_eq!(headless.inspection.scale_factor, 1.5);
        assert_eq!(headless.inspection.modality, InputModality::Accessibility);
        assert!(!headless.inspection.window_focused);
        assert!(headless.messages.iter().any(|message| {
            message.type_name == "replay.text.changed"
                && message.label.as_deref() == Some("accessible")
        }));
    }

    #[test]
    fn adapter_faults_are_typed_and_do_not_prevent_overlay_dismissal_or_mutate_state() {
        struct FaultApplication;
        impl Application for FaultApplication {
            type Message = ();
            fn update(&mut self, (): Self::Message) {}
            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                Button::new((), "Anchor").id("anchor")
            }
            fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
                vec![FrameOverlay::surface(
                    crate::TransientSurface::dialog(
                        "fault-dialog",
                        crate::OverlayAnchor::Node(UiId::from("anchor")),
                        crate::Size::new(120.0, 60.0),
                        crate::OverlayStyle {
                            background: 0x111111,
                            foreground: 0xffffff,
                            border: 0x888888,
                            selected: 0x333333,
                            radius: 8,
                        },
                    ),
                    Button::new((), "Dismiss"),
                )]
            }
        }

        let stages = [
            HostFailureStage::Presenter,
            HostFailureStage::Clipboard,
            HostFailureStage::Ime,
            HostFailureStage::Accessibility,
            HostFailureStage::Controller,
        ];
        for stage in stages {
            let mut host = UiHost::new(FaultApplication, 320, 200);
            assert!(host.open_transient(OverlayId::new("fault-dialog"), UiId::from("root/anchor")));
            let before = host.inspect();
            let failure = HostFailure {
                surface: "fault-test".into(),
                stage,
                optional: stage != HostFailureStage::Presenter,
                detail: format!("injected {stage:?} failure"),
            };
            let outcome = host.step(HostBatch {
                failures: vec![failure.clone()],
                events: vec![HostEvent::Ui(UiEvent::ControllerBack)],
                ..HostBatch::default()
            });

            assert_eq!(outcome.failures, [failure]);
            assert!(before.open_overlay.is_some());
            assert!(host.inspect().open_overlay.is_none());
            assert_eq!(host.inspect().window_focused, before.window_focused);
            assert!(
                host.accessibility_nodes()
                    .iter()
                    .any(|node| node.label.as_deref() == Some("Anchor"))
            );
        }
    }

    #[test]
    fn declarative_frame_layers_are_applied_on_initial_resolve_and_rebuild() {
        #[derive(Clone, PartialEq)]
        enum Message {
            Anchor,
            Context,
            Choose,
            ChooseSecond,
        }

        struct LayerApplication;

        impl Application for LayerApplication {
            type Message = Message;

            fn update(&mut self, _message: Self::Message) {}

            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                crate::Container::new()
                    .id("anchor")
                    .message(Message::Anchor)
                    .context_message(Message::Context)
                    .width(120.0)
                    .height(40.0)
            }

            fn frame_overlays(
                &self,
                _context: ViewContext,
            ) -> Vec<super::FrameOverlay<Self::Message>> {
                vec![super::FrameOverlay::Menu(
                    crate::OverlayMenu::new(
                        "context",
                        crate::OverlayAnchor::Point {
                            invocation_target: crate::UiId::from("anchor"),
                            point: crate::Point { x: 72.0, y: 24.0 },
                        },
                    )
                    .semantic_style(crate::OverlayStyle {
                        background: 0x202630,
                        foreground: 0xe8edf4,
                        border: 0x444444,
                        selected: 0x334455,
                        radius: 0,
                    })
                    .item(crate::OverlayMenuItem::action(
                        "choose",
                        "Choose",
                        Message::Choose,
                    ))
                    .item(crate::OverlayMenuItem::action(
                        "choose-second",
                        "Choose second",
                        Message::ChooseSecond,
                    )),
                )]
            }
        }

        let mut host = UiHost::new(LayerApplication, 320, 200);
        assert!(host.inspect().overlay_failures.is_empty());
        let anchor = host
            .semantic_targets_for_message(&Message::Anchor)
            .into_iter()
            .next()
            .unwrap()
            .id;
        host.handle_event(crate::UiEvent::FocusGained);
        host.handle_event(crate::UiEvent::ControllerDown);
        host.perform_semantic_action(
            anchor,
            crate::SemanticAction::Invoke(crate::ActionKind::ContextMenu),
        );
        assert!(host.inspect().overlay_failures.is_empty());
        let menu = host
            .query(&crate::SemanticSelector::Role(crate::SemanticRole::Menu))
            .pop()
            .expect("menu layer must survive the rebuild caused by opening it");
        assert_eq!(menu.bounds.origin, crate::Point { x: 72.0, y: 24.0 });
        assert_eq!(
            host.query(&crate::SemanticSelector::Role(
                crate::SemanticRole::MenuItem
            ))
            .len(),
            2
        );
        let first = host.inspect().controller_target.unwrap();
        let first_selected_rect = host
            .commands()
            .iter()
            .find_map(|command| match command {
                crate::backend::PaintCommand::RoundedFill { rect, color, .. }
                    if *color == 0x334455 =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .expect("first menu row must own the selected paint");
        host.handle_event(crate::UiEvent::ControllerDown);
        let second = host.inspect().controller_target.unwrap();
        assert_ne!(first, second);
        let items = host.query(&crate::SemanticSelector::Role(
            crate::SemanticRole::MenuItem,
        ));
        assert!(
            items
                .iter()
                .any(|item| item.id == second && item.focused && item.controller_selected)
        );
        assert!(
            items
                .iter()
                .any(|item| item.id == first && !item.focused && !item.controller_selected)
        );
        let second_selected_rect = host
            .commands()
            .iter()
            .find_map(|command| match command {
                crate::backend::PaintCommand::RoundedFill { rect, color, .. }
                    if *color == 0x334455 =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .expect("second menu row must own the selected paint after rebuild");
        assert_ne!(first_selected_rect, second_selected_rect);
    }

    #[test]
    fn invalid_frame_layer_anchor_is_retained_as_typed_host_evidence() {
        struct InvalidLayerApplication;

        impl Application for InvalidLayerApplication {
            type Message = ();

            fn update(&mut self, (): Self::Message) {}

            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                crate::Container::new().id("present")
            }

            fn frame_overlays(
                &self,
                _context: ViewContext,
            ) -> Vec<super::FrameOverlay<Self::Message>> {
                vec![super::FrameOverlay::Menu(crate::OverlayMenu::new(
                    "broken",
                    crate::OverlayAnchor::Node(crate::UiId::from("missing")),
                ))]
            }
        }

        let host = UiHost::new(InvalidLayerApplication, 320, 200);
        assert_eq!(host.inspect().overlay_failures.len(), 1);
        assert_eq!(
            host.inspect().overlay_failures[0].error,
            crate::SemanticActionError::MissingTarget
        );
    }

    #[test]
    fn inspection_and_accessibility_retain_navigation_depth_and_actions_across_rebuild() {
        struct NestedApplication {
            activated: bool,
        }

        impl Application for NestedApplication {
            type Message = ();

            fn update(&mut self, (): Self::Message) {
                self.activated = true;
            }

            fn view(&self, _context: ViewContext) -> impl crate::View<Self::Message> {
                crate::Container::new()
                    .id("outer")
                    .navigation_scope(crate::NavigationScope::group())
                    .child(
                        crate::Container::new()
                            .id("inner")
                            .navigation_scope(crate::NavigationScope::group())
                            .child(
                                crate::Button::new(
                                    (),
                                    if self.activated {
                                        "Activated"
                                    } else {
                                        "Activate"
                                    },
                                )
                                .id("action"),
                            ),
                    )
            }
        }

        let mut host = UiHost::new(NestedApplication { activated: false }, 320, 200);
        host.handle_event(UiEvent::ControllerDown);
        host.handle_event(UiEvent::ControllerActivate);
        assert_eq!(host.inspect().navigation_depth, 1);
        host.handle_event(UiEvent::ControllerActivate);
        assert_eq!(host.inspect().navigation_depth, 2);
        assert_eq!(
            host.inspect().available_semantic_actions,
            [crate::ActionKind::Activate]
        );
        let selected = host
            .inspect()
            .controller_target
            .expect("nested action selected");
        let accessible = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.id == selected)
            .expect("selected action remains accessibility-visible");
        assert_eq!(accessible.navigation_depth, 2);
        assert_eq!(accessible.actions, [crate::ActionKind::Activate]);

        host.handle_event(UiEvent::ControllerActivate);
        assert!(host.application().activated);
        assert_eq!(host.inspect().navigation_depth, 2);
        assert_eq!(host.inspect().controller_target.as_ref(), Some(&selected));
        assert_eq!(
            host.inspect().available_semantic_actions,
            [crate::ActionKind::Activate]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn outbound_file_drag_uses_encoded_crlf_uri_list() {
        use std::os::unix::ffi::OsStringExt;

        let paths = [
            std::path::PathBuf::from("/tmp/a file.txt"),
            std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/nonutf8-\xff".to_vec())),
        ];
        assert_eq!(
            super::file_uri_list(&paths),
            b"file:///tmp/a%20file.txt\r\nfile:///tmp/nonutf8-%FF\r\n"
        );
    }
}
