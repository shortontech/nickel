//! Trusted display recovery chooser, separate from ordinary JSX Quick Settings.
use crate::control_view::ControlAction;
use nickel_core::display_projection::ProjectionMode;
use nickel_core::theme::{Appearance, ThemePalette};
use nickel_i18n::Localizer;
use nickel_ui::{
    Align, AnyView, Application, Button, Column, ComponentBuilderExt, Container, DesktopDensity,
    Insets, LinearGradient, ReadingDirection, Row, Spacer, Text, UiHost, ViewContext,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProjectionRecoveryState {
    pub projection_only: bool,
    pub pending_projection: Option<ProjectionMode>,
}
pub struct ProjectionRecoveryApp {
    palette: ThemePalette,
    supported_projection_modes: Vec<ProjectionMode>,
    state: ProjectionRecoveryState,
    effects: Vec<ControlAction>,
    dirty: bool,
}
impl ProjectionRecoveryApp {
    pub fn new() -> Self {
        Self {
            palette: ThemePalette::from_appearance(Appearance::default()),
            supported_projection_modes: Vec::new(),
            state: Default::default(),
            effects: Vec::new(),
            dirty: false,
        }
    }
    pub fn set_palette(&mut self, palette: ThemePalette) {
        if self.palette != palette {
            self.palette = palette;
            self.dirty = true;
        }
    }
    pub fn sync_projection_modes(&mut self, modes: &[ProjectionMode]) {
        if self.supported_projection_modes != modes {
            self.supported_projection_modes = modes.to_vec();
            self.dirty = true;
        }
    }
    pub fn show_projection_chooser(&mut self) {
        self.state.projection_only = true;
        self.state.pending_projection = None;
        self.dirty = true;
    }
    pub fn dismiss(&mut self) {
        if self.state.projection_only {
            self.state = Default::default();
            self.dirty = true;
        }
    }
    pub fn view_state(&self) -> ProjectionRecoveryState {
        self.state
    }
    pub fn projection_preview_failed(&mut self) {
        if self.state.pending_projection.take().is_some() {
            self.dirty = true;
        }
    }
    pub fn take_effects(&mut self) -> Vec<ControlAction> {
        std::mem::take(&mut self.effects)
    }
}
impl Application for ProjectionRecoveryApp {
    type Message = ControlAction;
    fn update(&mut self, message: ControlAction) {
        match message {
            ControlAction::PreviewProjection(mode) => self.state.pending_projection = Some(mode),
            ControlAction::ConfirmProjection | ControlAction::CancelProjection => {
                self.state.pending_projection = None
            }
            _ => return,
        };
        self.effects.push(message);
        self.dirty = true;
    }
    fn view(&self, context: ViewContext) -> impl nickel_ui::View<ControlAction> {
        let localizer = Localizer::system();
        let direction = if localizer.is_right_to_left() {
            ReadingDirection::RightToLeft
        } else {
            ReadingDirection::LeftToRight
        };
        projection_chooser_view(
            self.palette,
            self.state.pending_projection,
            &self.supported_projection_modes,
            context.viewport.size.width,
            context.viewport.size.height,
            direction,
            &localizer,
        )
    }
    fn poll(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}
pub type ProjectionRecoveryHost = UiHost<ProjectionRecoveryApp>;
struct Card {
    view: AnyView<ControlAction>,
}
fn directional_row(row: Row<ControlAction>, direction: ReadingDirection) -> Row<ControlAction> {
    if direction == ReadingDirection::RightToLeft {
        row.reverse()
    } else {
        row
    }
}
fn projection_view(
    palette: ThemePalette,
    pending: Option<ProjectionMode>,
    supported: &[ProjectionMode],
    direction: ReadingDirection,
    localizer: &Localizer,
) -> Card {
    if pending.is_some() {
        return card(
            palette,
            vec![
                AnyView::new(
                    Text::new(localizer.text("control-center-keep-display-settings"))
                        .color(palette.text),
                ),
                AnyView::new(directional_row(
                    Row::new()
                        .gap(8.0)
                        .child(button(
                            palette,
                            action(ControlAction::CancelProjection),
                            localizer.text("control-center-revert"),
                        ))
                        .child(button(
                            palette,
                            action(ControlAction::ConfirmProjection),
                            localizer.text("control-center-keep"),
                        )),
                    direction,
                )),
            ],
        );
    }
    let modes = [
        (
            localizer.text("control-center-display-internal"),
            ProjectionMode::InternalOnly,
        ),
        (
            localizer.text("control-center-display-duplicate"),
            ProjectionMode::Duplicate,
        ),
        (
            localizer.text("control-center-display-extend"),
            ProjectionMode::Extend,
        ),
        (
            localizer.text("control-center-display-external"),
            ProjectionMode::ExternalOnly,
        ),
    ];
    card(
        palette,
        vec![AnyView::new(directional_row(
            Row::new()
                .gap(DesktopDensity::COMPACT.related_gap)
                .align_items(Align::Center)
                .child(Text::new(localizer.text("control-center-displays")).color(palette.text))
                .child(Spacer::flex())
                .children(
                    modes
                        .into_iter()
                        .filter(|(_, mode)| supported.contains(mode))
                        .map(|(label, mode)| {
                            AnyView::new(button(
                                palette,
                                action(ControlAction::PreviewProjection(mode)),
                                label,
                            ))
                        }),
                ),
            direction,
        ))],
    )
}

fn projection_chooser_view(
    palette: ThemePalette,
    pending: Option<ProjectionMode>,
    supported: &[ProjectionMode],
    width: f32,
    height: f32,
    direction: ReadingDirection,
    localizer: &Localizer,
) -> AnyView<ControlAction> {
    let content = if supported.is_empty() {
        AnyView::new(
            Column::new()
                .gap(8.0)
                .child(
                    Text::new(localizer.text("control-center-displays"))
                        .scale(3.0)
                        .bold(true)
                        .color(palette.text),
                )
                .child(
                    Text::new(localizer.text("control-center-displays-unavailable"))
                        .color(palette.muted),
                ),
        )
    } else {
        projection_view(palette, pending, supported, direction, localizer).view
    };
    AnyView::new(
        Container::new()
            .width(width.max(280.0))
            .height(height.max(240.0))
            .padding(24.0)
            .background(LinearGradient::vertical(palette.panel, palette.background))
            .child(content),
    )
}

fn card(palette: ThemePalette, children: Vec<AnyView<ControlAction>>) -> Card {
    let density = DesktopDensity::COMPACT;
    Card {
        view: AnyView::new(
            Column::new()
                .padding(density.related_gap)
                .gap(density.related_gap)
                .background(palette.surface)
                .border(palette.surface_hover, 1.0)
                .radius(12.0)
                .children(children),
        ),
    }
}

fn action(value: ControlAction) -> ControlAction {
    value
}

const fn translucent(color: u32, alpha: u32) -> u32 {
    (color & 0x00ff_ffff) | (alpha << 24)
}

fn button(
    palette: ThemePalette,
    value: ControlAction,
    label: impl Into<String>,
) -> Button<ControlAction> {
    Button::new(value, label)
        .height(DesktopDensity::COMPACT.touch_target)
        .padding(Insets {
            top: 6.0,
            right: 10.0,
            bottom: 6.0,
            left: 10.0,
        })
        .radius(7.0)
        .background(palette.surface_hover)
        .border(translucent(palette.muted, 0x38), 1.0)
        .color(palette.text)
        .center_label_vertically()
        .focus_background_tint(palette.accent)
        .controller_focus_background_tint(palette.accent)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trusted_projection_recovery_previews_and_confirms_without_ordinary_settings_controls() {
        let mut app = ProjectionRecoveryApp::new();
        app.sync_projection_modes(&[ProjectionMode::Extend]);
        app.show_projection_chooser();
        let mut host = UiHost::new(app, 420, 600);
        let localizer = Localizer::system();
        let extend = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: localizer.text("control-center-display-extend").into(),
            })
            .unwrap();
        host.perform_semantic_action(
            extend.id,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
        );
        assert_eq!(
            host.application_mut().take_effects(),
            vec![ControlAction::PreviewProjection(ProjectionMode::Extend)]
        );
        let keep = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: localizer.text("control-center-keep").into(),
            })
            .unwrap();
        host.perform_semantic_action(
            keep.id,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
        );
        assert_eq!(
            host.application_mut().take_effects(),
            vec![ControlAction::ConfirmProjection]
        );
        assert!(host.application().view_state().pending_projection.is_none());
        assert!(
            host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Lock".into()
            })
            .is_err()
        );
    }
}
