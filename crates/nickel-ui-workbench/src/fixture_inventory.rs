//! Broad, production-component fixture coverage for the native workbench.
//!
//! `DialogInventory` is deliberately a semantic dialog surface built from the
//! public declarative `Surface` and `Container` APIs. `nickel-ui` does not
//! expose a standalone dialog component; native transient placement remains a
//! host responsibility and is not imitated here.

use nickel_ui::backend::PaintCommand;
use nickel_ui::{
    AccountSummaryRow, ActionLegend, ActionLegendEntry, ActionRegion, AnyView, Application, Button,
    ButtonLabel, ChoiceCard, ChoiceCardGroup, ColorSwatch, Column, CompactIconTile,
    ComponentBuilderExt, Container, ContentPane, ControllerFamily, CustomPaint, Dropdown,
    FallbackAvatar, FieldGroup, FileGrid, FileGridItem, FrameOverlay, Grid, Header, HorizontalRule,
    Icon, Image, InlineButtonGroup, Insets, ItemPresentation, LauncherSearchField, Menu, MenuBar,
    MenuItem, NavigationItem, NavigationSectionLabel, OverlayAnchor, OverlayStyle, PageHeader,
    Popover, PreviewTile, ProjectStatusRow, RadioButton, RadioGroup, RadioOption, Rect,
    SectionHeader, SelectField, SelectionIndicator, SelectionRegion, SemanticColors,
    SemanticControllerAction, SemanticRole, SemanticTheme, SessionActionRow, SettingsCard,
    SettingsListCard, SettingsNavigation, SettingsRow, SettingsSearchField, SettingsSection,
    SettingsShell, SettingsStatus, SettingsStatusKind, ShortcutRow, ShoulderHints, Sidebar,
    SidebarFolder, SidebarItem, SidebarSection, Size, Slider, SliderField, Spacer, StartMenuShell,
    StatusRegion, StyledText, StyledTextSpan, Surface, SurfaceRole, SurfaceScaffold, Switch,
    TabList, Text, TextField, ToolRegion, Tooltip, UiId, VerticalScroll, ViewContext,
    VirtualColumn, VirtualWindow,
};
use nickel_ui_testkit::{
    AccessibilityPreset, Fixture, FixtureDirection, FixtureMetadata, FixtureRegistry,
    FixtureSource, FixtureTheme, FixtureVariant, LocalePreset, RegistryError, ScalePreset,
    Selector, SimulatedEffectKind, ViewportPreset,
};
use std::sync::Arc;

const DARK: FixtureVariant = variant("dark", "Dark", 920, 700, FixtureTheme::Dark);
const COMPACT: FixtureVariant = variant("compact", "Compact", 620, 720, FixtureTheme::Dark);

const fn variant(
    id: &'static str,
    title: &'static str,
    width: u32,
    height: u32,
    theme: FixtureTheme,
) -> FixtureVariant {
    FixtureVariant {
        id,
        title,
        viewport: ViewportPreset { id, width, height },
        theme,
        locale: LocalePreset {
            id: "en-US",
            direction: FixtureDirection::LeftToRight,
        },
        scale: ScalePreset {
            id: "1x",
            factor: 1.0,
        },
        controller_family: ControllerFamily::Xbox,
        accessibility: AccessibilityPreset {
            id: "default",
            high_contrast: false,
            reduced_motion: false,
            reduced_transparency: false,
        },
    }
}

fn theme() -> SemanticTheme {
    SemanticTheme::new(SemanticColors {
        window: 0x101722,
        sidebar: 0x151f2d,
        card: 0x1c2838,
        raised: 0x243247,
        hover: 0x2d405a,
        primary_text: 0xf2f6fb,
        secondary_text: 0x9cacc0,
        accent: 0x7b61ff,
        accent_soft: 0x332c61,
        secondary_accent: 0x67b7ff,
        positive: 0x61c993,
    })
}

#[derive(Clone, Debug, PartialEq)]
enum Message {
    Activate(&'static str),
    Query(String),
    Adjust(f32),
    Toggle(bool),
}

#[derive(Clone, Copy)]
enum Family {
    Primitives,
    Settings,
    StartMenu,
    Dialog,
    CustomPaint,
}

struct InventoryApp {
    family: Family,
    query: String,
    value: f32,
    enabled: bool,
}

impl InventoryApp {
    fn new(family: Family) -> Self {
        Self {
            family,
            query: String::new(),
            value: 0.55,
            enabled: true,
        }
    }

    fn primitives(&self) -> AnyView<Message> {
        let palette = theme();
        let sample_pixels = Arc::new(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([255, 255, 255, 255]),
        ));
        let navigation = Sidebar::new(210.0)
            .background(palette.surfaces.sidebar)
            .padding(Insets::all(12.0))
            .gap(6.0)
            .child(
                SidebarSection::new("Library", palette.text.secondary)
                    .child(
                        SidebarItem::new(
                            Message::Activate("components"),
                            "Components",
                            palette.text.primary,
                        )
                        .background(palette.surfaces.selected)
                        .accessibility_label("Components"),
                    )
                    .child(
                        SidebarFolder::new(
                            Message::Activate("toggle-folder"),
                            Message::Activate("open-folder"),
                            "Layouts",
                            true,
                            palette.text.primary,
                        )
                        .accessibility_labels(("Toggle Layouts", "Layouts")),
                    ),
            )
            .child(HorizontalRule::new(palette.borders.subtle))
            .child(ShoulderHints::new(
                palette.text.primary,
                palette.text.secondary,
            ));
        let controls = Column::new()
            .gap(12.0)
            .child(Text::new("Declarative primitives").bold(true).scale(1.35))
            .child(
                InlineButtonGroup::new(palette)
                    .action(
                        Button::new(Message::Activate("primary"), "Primary action").id("primary"),
                    )
                    .action(
                        Button::new(Message::Activate("secondary"), "Secondary").id("secondary"),
                    ),
            )
            .child(
                TextField::on_change_with_placeholder(
                    &self.query,
                    "Filter components",
                    Message::Query,
                )
                .id("primitive-filter"),
            )
            .child(
                RadioGroup::new([
                    RadioOption::new(
                        palette,
                        Message::Activate("comfortable"),
                        "Comfortable",
                        true,
                    ),
                    RadioOption::new(palette, Message::Activate("compact"), "Compact", false),
                ])
                .id("density"),
            )
            .child(
                RadioButton::new(Message::Activate("legacy-radio"), "Standalone radio", false)
                    .accessibility_label("Standalone radio"),
            )
            .child(
                Slider::on_change(Message::Adjust, self.value)
                    .id("primitive-slider")
                    .accessibility_label("Primitive value"),
            )
            .child(
                Switch::new(self.enabled, Message::Toggle, palette)
                    .accessibility_label("Shared primitive switch"),
            )
            .child(
                Dropdown::new(
                    Message::Activate("toggle-dropdown"),
                    "Dark",
                    [
                        ("Dark", Message::Activate("dark")),
                        ("Light", Message::Activate("light")),
                    ],
                )
                .accessibility_label("Theme"),
            )
            .child(
                MenuBar::new().child(
                    Menu::new(
                        Message::Activate("toggle-menu"),
                        "Actions",
                        [
                            MenuItem::new("Inspect", Message::Activate("inspect")),
                            MenuItem::disabled("Unavailable"),
                        ],
                    )
                    .accessibility_label("Actions"),
                ),
            )
            .child(
                Grid::fixed(3)
                    .gap(8.0)
                    .children(["Container", "Row / Column", "Grid"].map(|label| {
                        Surface::new(palette, SurfaceRole::Card)
                            .padding(Insets::all(10.0))
                            .child(Text::new(label))
                    })),
            )
            .child(Header::new("Media, selection, and virtualization"))
            .child(
                SelectionRegion::automatic().id("selection-region").child(
                    StyledText::new(
                        "Selectable styled text",
                        vec![StyledTextSpan {
                            range: 0..10,
                            bold: true,
                            italic: false,
                            monospace: false,
                            strikethrough: false,
                            color: Some(palette.text.primary),
                            background: None,
                        }],
                    )
                    .selection_run_id("styled-sample"),
                ),
            )
            .child(
                nickel_ui::Row::new()
                    .gap(10.0)
                    .child(
                        Image::new(41, Arc::clone(&sample_pixels))
                            .width(32.0)
                            .height(32.0),
                    )
                    .child(
                        Icon::new(42, Arc::clone(&sample_pixels), palette.text.primary, 32.0)
                            .label("Sample icon"),
                    ),
            )
            .child(
                VirtualColumn::new()
                    .window(VirtualWindow::from_heights(
                        &[24.0, 24.0, 24.0],
                        4.0,
                        12.0,
                        36.0,
                        8.0,
                    ))
                    .gap(4.0)
                    .children([Text::new("Virtual row 1"), Text::new("Virtual row 2")]),
            )
            .child(
                FileGrid::columns(2).gap(8.0).items([
                    FileGridItem::new(
                        Message::Activate("file-one"),
                        "Fixture.txt",
                        43,
                        Arc::clone(&sample_pixels),
                    )
                    .accessibility_label("Fixture.txt"),
                    FileGridItem::new(Message::Activate("file-two"), "Archive", 44, sample_pixels)
                        .accessibility_label("Archive"),
                ]),
            )
            .child(
                SurfaceScaffold::new(
                    "primitive-scaffold",
                    ItemPresentation::new(
                        "presented-item",
                        ButtonLabel::new("Presented item"),
                        palette,
                    )
                    .on_activate(Message::Activate("presented-item"))
                    .accessibility_label("Presented item"),
                )
                .actions(
                    ActionRegion::new("sample-actions").child(SelectionIndicator::new(
                        true,
                        palette.colors.accent,
                        palette.colors.card,
                    )),
                )
                .footer(StatusRegion::new("sample-status").child(Text::new("Ready"))),
            )
            .child(
                ToolRegion::new("sample-tools")
                    .child(Text::new("Tool one"))
                    .child(Spacer::fixed(8.0))
                    .child(Text::new("Tool two")),
            );
        AnyView::new(
            Surface::new(palette, SurfaceRole::Window).child(
                nickel_ui::Row::new()
                    .fill_width()
                    .fill_height()
                    .child(navigation)
                    .child(ContentPane::new(controls).background(palette.surfaces.window)),
            ),
        )
    }

    fn settings(&self) -> AnyView<Message> {
        let palette = theme();
        let choices = ChoiceCardGroup::new([
            ChoiceCard::new(
                palette,
                Message::Activate("dark-choice"),
                "Dark",
                true,
                PreviewTile::new(palette, Text::new("Aa")),
            )
            .id("dark-choice"),
            ChoiceCard::new(
                palette,
                Message::Activate("light-choice"),
                "Light",
                false,
                PreviewTile::loading(palette, "Loading preview"),
            )
            .id("light-choice"),
        ]);
        let body = Column::new()
            .gap(16.0)
            .child(PageHeader::new(
                palette,
                "Settings component inventory",
                "Production rows, fields, status, tabs, choices, previews, and swatches",
            ))
            .child(
                SettingsSection::new(palette, "Controls").child(
                    SettingsListCard::new(palette)
                        .row(
                            SettingsRow::new(palette, "Automatic updates", "Install safe updates")
                                .trailing(
                                    Switch::new(self.enabled, Message::Toggle, palette)
                                        .accessibility_label("Automatic updates"),
                                ),
                        )
                        .row(SettingsRow::new(palette, "Open details", "Activatable row"))
                        .id("settings-list"),
                ),
            )
            .child(SliderField::new(
                palette,
                "Text size",
                "Scale interface text",
                "110%",
                self.value,
                Message::Adjust,
            ))
            .child(SelectField::new(
                palette,
                "Theme",
                "Choose an appearance",
                Message::Activate("theme-menu"),
                "Dark",
                [
                    ("Dark", Message::Activate("dark")),
                    ("Light", Message::Activate("light")),
                ],
                false,
            ))
            .child(FieldGroup::new(palette).field(SettingsStatus::new(
                palette,
                SettingsStatusKind::RestartRequired,
                "Restart required to finish applying this change",
            )))
            .child(TabList::new(
                palette,
                [
                    ("Appearance", Message::Activate("appearance"), true),
                    ("Accessibility", Message::Activate("accessibility"), false),
                ],
            ))
            .child(choices)
            .child(
                SettingsCard::titled(palette, "Accent", "Semantic radio swatches").child(
                    nickel_ui::Row::new()
                        .gap(8.0)
                        .child(ColorSwatch::color_labeled(
                            palette,
                            Message::Activate("violet"),
                            0x7b61ff,
                            "Violet",
                            true,
                        ))
                        .child(ColorSwatch::custom(
                            palette,
                            Message::Activate("custom-color"),
                        )),
                ),
            );
        let navigation = SettingsNavigation::new(palette, 220.0)
            .child(SettingsSearchField::new(
                palette,
                "settings-search",
                &self.query,
                "Search settings",
                Message::Query,
            ))
            .child(NavigationSectionLabel::new(palette, "Workbench"))
            .item(NavigationItem::new(
                palette,
                Message::Activate("appearance"),
                "Appearance",
                true,
            ));
        AnyView::new(
            Surface::new(palette, SurfaceRole::Window).child(SettingsShell::new(
                palette,
                920.0,
                navigation,
                Text::new("Settings"),
                VerticalScroll::new(Message::Activate("scroll-settings"), 0.0)
                    .padding(Insets::all(18.0))
                    .child(body),
            )),
        )
    }

    fn start_menu(&self, width: f32) -> AnyView<Message> {
        let palette = theme();
        let primary = Column::new()
            .gap(8.0)
            .child(SectionHeader::new(palette, "Projects").action(
                palette,
                "See all",
                Message::Activate("all-projects"),
            ))
            .child(ProjectStatusRow::new(
                palette,
                Text::new("N"),
                "Nickel",
                "Active",
                Some(3),
                Some(Message::Activate("nickel")),
                true,
            ))
            .child(CompactIconTile::new(
                palette,
                Text::new("+"),
                "New project",
                Message::Activate("new-project"),
            ))
            .child(ShortcutRow::new(
                palette,
                Text::new("N"),
                "Nickel shortcut",
                "Pinned",
                Some(Message::Activate("nickel-shortcut")),
                false,
            ));
        let detail = Column::new()
            .gap(8.0)
            .child(SectionHeader::new(palette, "Account"))
            .child(AccountSummaryRow::new(
                palette,
                FallbackAvatar::new(palette, "Steven Nickel"),
                "Steven Nickel",
                "Local account",
                Some(Message::Activate("account")),
                false,
            ))
            .child(SessionActionRow::new(
                palette,
                Text::new("↪"),
                "Log out",
                Message::Activate("logout"),
                true,
                false,
            ));
        let legend = ActionLegend::new(
            palette,
            ControllerFamily::Xbox,
            [
                ActionLegendEntry::available(SemanticControllerAction::Confirm, "Open"),
                ActionLegendEntry::available(SemanticControllerAction::Cancel, "Close"),
            ],
        );
        AnyView::new(
            StartMenuShell::new(palette, width, primary, detail)
                .header(LauncherSearchField::new(
                    palette,
                    Text::new("⌕"),
                    &self.query,
                    "",
                    "Search applications",
                    Message::Query,
                ))
                .primary_footer(Text::new("Pinned projects"))
                .detail_footer(Text::new("Session actions are simulated"))
                .legend(legend),
        )
    }

    fn dialog(&self) -> AnyView<Message> {
        let palette = theme();
        AnyView::new(
            Surface::new(palette, SurfaceRole::Window).child(
                Container::new()
                    .fill_width()
                    .fill_height()
                    .align_items(nickel_ui::Align::Center)
                    .justify_content(nickel_ui::Justify::Center)
                    .child(
                        Surface::new(palette, SurfaceRole::Raised)
                            .id("semantic-dialog")
                            .width(440.0)
                            .padding(Insets::all(20.0))
                            .border((palette.borders.ordinary, 1.0))
                            .radius(palette.radii.card)
                            .child(
                                Column::new()
                                    .gap(14.0)
                                    .semantic_role(SemanticRole::Dialog)
                                    .accessibility_label("Remove pinned project?")
                                    .child(Text::new("Remove pinned project?").bold(true).scale(1.3))
                                    .child(Text::new(
                                        "This semantic dialog surface records fixture actions only.",
                                    ))
                                    .child(
                                        InlineButtonGroup::new(palette)
                                            .action(Button::new(Message::Activate("cancel"), "Cancel"))
                                            .action(
                                                Button::new(Message::Activate("remove"), "Remove")
                                                    .id("remove"),
                                            ),
                                    ),
                            ),
                    ),
            ),
        )
    }

    fn custom_paint(&self) -> AnyView<Message> {
        fn paint(bounds: Rect) -> Vec<PaintCommand> {
            vec![
                PaintCommand::Fill {
                    rect: bounds,
                    color: 0x17243a,
                },
                PaintCommand::Stroke {
                    rect: Rect::new(
                        bounds.origin.x + 8.0,
                        bounds.origin.y + 8.0,
                        (bounds.size.width - 16.0).max(0.0),
                        (bounds.size.height - 16.0).max(0.0),
                    ),
                    color: 0x7b61ff,
                    width: 3.0,
                },
            ]
        }
        AnyView::new(
            Column::new()
                .gap(12.0)
                .padding(Insets::all(18.0))
                .child(Text::new("Bounded custom-paint exception").bold(true))
                .child(
                    CustomPaint::new(paint)
                        .id("bounded-custom-paint")
                        .width(480.0)
                        .height(220.0)
                        .semantic_role(SemanticRole::Image)
                        .accessibility_label("Decorative bounded custom paint sample"),
                )
                .child(Button::new(
                    Message::Activate("inspect-paint"),
                    "Inspect paint bounds",
                )),
        )
    }
}

impl Application for InventoryApp {
    type Message = Message;

    fn update(&mut self, message: Self::Message) {
        match message {
            Message::Query(query) => self.query = query,
            Message::Adjust(value) => self.value = value,
            Message::Toggle(enabled) => self.enabled = enabled,
            Message::Activate(_) => {}
        }
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        match self.family {
            Family::Primitives => self.primitives(),
            Family::Settings => self.settings(),
            Family::StartMenu => self.start_menu(context.viewport.size.width),
            Family::Dialog => self.dialog(),
            Family::CustomPaint => self.custom_paint(),
        }
    }

    fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        if !matches!(self.family, Family::Primitives) {
            return Vec::new();
        }
        let style = OverlayStyle {
            background: 0x243247,
            foreground: 0xf2f6fb,
            border: 0x5f7390,
            selected: 0x2d405a,
            radius: 8,
        };
        vec![
            Popover::new(
                "primitive-popover",
                OverlayAnchor::InvocationTarget(UiId::from("primary")),
                "Primitive details",
                Size::new(240.0, 96.0),
                style,
                Column::new()
                    .child(Text::new("Production popover"))
                    .child(Button::new(Message::Activate("close-popover"), "Close")),
            )
            .focus_return("primary")
            .into(),
            Tooltip::new(
                "primitive-tooltip",
                OverlayAnchor::InvocationTarget(UiId::from("secondary")),
                "Secondary action help",
                Size::new(180.0, 48.0),
                style,
                Text::new("Runs the secondary action"),
            )
            .into(),
        ]
    }
}

macro_rules! inventory_fixture {
    ($name:ident, $id:literal, $title:literal, $description:literal, $tags:expr, $family:expr, $activation:expr, $effects:expr) => {
        struct $name;
        impl Fixture for $name {
            type App = InventoryApp;

            fn metadata() -> &'static FixtureMetadata {
                static METADATA: FixtureMetadata = FixtureMetadata {
                    id: $id,
                    title: $title,
                    description: $description,
                    tags: $tags,
                    source: FixtureSource {
                        crate_name: "nickel-ui-workbench",
                        file: file!(),
                        line: line!(),
                    },
                    variants: &[DARK, COMPACT],
                    assets: &[],
                    simulated_effects: $effects,
                };
                &METADATA
            }

            fn create() -> Self::App {
                InventoryApp::new($family)
            }

            fn surface_size() -> (u32, u32) {
                (920, 700)
            }

            fn default_activation() -> Option<Selector> {
                Some($activation)
            }
        }
    };
}

inventory_fixture!(
    PrimitiveInventory,
    "shared.public-components",
    "Public declarative components",
    "Layout, navigation, controls, fields, menus, and semantic composition using production components",
    &["shared", "public", "primitives", "declarative"],
    Family::Primitives,
    Selector::role_name(SemanticRole::Button, "Primary action"),
    &[]
);
inventory_fixture!(
    SettingsInventory,
    "settings.component-inventory",
    "Settings component inventory",
    "Public Settings cards, rows, fields, states, choices, previews, tabs, and swatches",
    &["settings", "public", "fields", "states"],
    Family::Settings,
    Selector::role_name(SemanticRole::Option, "Dark"),
    &[]
);
inventory_fixture!(
    StartMenuInventory,
    "launcher.component-inventory",
    "Start-menu component inventory",
    "Public launcher shell, search, project, account, session, tile, section, and controller legend components",
    &["launcher", "start-menu", "controller", "public"],
    Family::StartMenu,
    Selector::role_name(SemanticRole::Button, "See all  ›"),
    &[SimulatedEffectKind::Logout]
);
inventory_fixture!(
    DialogInventory,
    "shared.semantic-dialog",
    "Semantic dialog surface",
    "Declarative semantic dialog content; transient placement remains host-owned",
    &["dialog", "semantic", "declarative"],
    Family::Dialog,
    Selector::role_name(SemanticRole::Button, "Remove"),
    &[SimulatedEffectKind::FileMutation]
);
inventory_fixture!(
    CustomPaintInventory,
    "shared.custom-paint",
    "Bounded custom paint",
    "Exceptional bounded painter plus ordinary production semantics",
    &["custom-paint", "bounded", "exception"],
    Family::CustomPaint,
    Selector::role_name(SemanticRole::Button, "Inspect paint bounds"),
    &[]
);

pub(crate) fn register(registry: &mut FixtureRegistry) -> Result<(), RegistryError> {
    registry.register::<PrimitiveInventory>()?;
    registry.register::<SettingsInventory>()?;
    registry.register::<StartMenuInventory>()?;
    registry.register::<DialogInventory>()?;
    registry.register::<CustomPaintInventory>()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nickel_ui::{OverlayId, SemanticSelector, UiHost};
    use nickel_ui_testkit::{ActivationVia, FixtureRegistry};

    use super::*;

    #[test]
    fn inventory_registration_is_complete_and_unique() {
        let mut registry = FixtureRegistry::new();
        register(&mut registry).expect("inventory metadata is valid");
        let entries = registry.finish();
        assert_eq!(entries.len(), 5);
        assert!(
            entries
                .iter()
                .all(|entry| entry.metadata.variants.len() == 2)
        );
        assert!(
            entries.iter().all(|entry| entry
                .metadata
                .source
                .file
                .ends_with("fixture_inventory.rs"))
        );
    }

    #[test]
    fn every_inventory_default_activation_uses_production_semantics() {
        let mut registry = FixtureRegistry::new();
        register(&mut registry).expect("inventory metadata is valid");
        for entry in registry.finish() {
            let mut session = entry.open();
            session
                .activate(ActivationVia::Accessibility)
                .unwrap_or_else(|error| {
                    panic!("{} default activation: {error}", entry.metadata.id)
                });
        }
    }

    #[test]
    fn dialog_fixture_declares_dialog_semantics() {
        let mut registry = FixtureRegistry::new();
        register(&mut registry).expect("inventory metadata is valid");
        let entry = registry
            .finish()
            .into_iter()
            .find(|entry| entry.metadata.id == "shared.semantic-dialog")
            .expect("dialog fixture");
        let session = entry.open();
        assert!(
            session
                .semantic_nodes()
                .iter()
                .any(|node| node.role == Some(SemanticRole::Dialog))
        );
    }

    #[test]
    fn public_component_fixture_opens_distinct_popover_and_tooltip_surfaces() {
        for (overlay_id, role, name) in [
            (
                "primitive-popover",
                SemanticRole::Popover,
                "Primitive details",
            ),
            (
                "primitive-tooltip",
                SemanticRole::Tooltip,
                "Secondary action help",
            ),
        ] {
            let mut host = UiHost::new(InventoryApp::new(Family::Primitives), 920, 700);
            assert!(host.open_transient(
                OverlayId::new(overlay_id),
                UiId::from(if role == SemanticRole::Popover {
                    "root/primary"
                } else {
                    "root/secondary"
                }),
            ));
            assert!(
                host.query_unique(&SemanticSelector::RoleAndName {
                    role,
                    name: name.into(),
                })
                .is_ok(),
                "{overlay_id} exposes canonical named semantics"
            );
            assert!(host.accessibility_nodes().iter().any(|node| {
                node.label.as_deref() == Some(name)
                    && node.role.as_deref()
                        == Some(if role == SemanticRole::Popover {
                            "popover"
                        } else {
                            "tooltip"
                        })
            }));
        }
    }

    #[test]
    fn public_authoring_primitive_inventory_is_fail_closed_and_references_real_fixtures() {
        let document: toml::Value =
            toml::from_str(include_str!("../public-authoring-primitives.toml"))
                .expect("authoring primitive inventory is valid TOML");
        let coverage: BTreeMap<String, String> = document["coverage"]
            .as_table()
            .expect("coverage table")
            .iter()
            .map(|(primitive, fixture)| {
                (
                    primitive.clone(),
                    fixture.as_str().expect("fixture ID string").to_owned(),
                )
            })
            .collect();

        let mut implementations = BTreeSet::new();
        for source in [
            include_str!("../../nickel-ui/src/ui.rs"),
            include_str!("../../nickel-ui/src/ui/components.rs"),
            include_str!("../../nickel-ui/src/ui/collection.rs"),
            include_str!("../../nickel-ui/src/ui/responsive_navigation.rs"),
            include_str!("../../nickel-ui/src/ui/settings_components.rs"),
            include_str!("../../nickel-ui/src/ui/start_menu_components.rs"),
            include_str!("../../nickel-ui/src/primitives.rs"),
        ] {
            for suffix in source.split("Component<Message> for ").skip(1) {
                let name: String = suffix
                    .chars()
                    .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                    .collect();
                if name != "T" && name != "$name" && !name.is_empty() {
                    implementations.insert(name);
                }
            }
        }
        implementations.extend(
            [
                "Column",
                "Row",
                "StatusRegion",
                "ActionRegion",
                "ToolRegion",
                "Popover",
                "Tooltip",
            ]
            .map(str::to_owned),
        );

        let root = include_str!("../../nickel-ui/src/lib.rs");
        let root_exports = root
            .split("pub use ui::{")
            .nth(1)
            .and_then(|tail| tail.split("};").next())
            .expect("nickel-ui root ui export block");
        let exported_components: BTreeSet<_> = implementations
            .into_iter()
            .filter(|name| {
                root_exports
                    .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                    .any(|token| token == name)
                    || root.contains("pub use primitives::{")
                        && [
                            "ActionRegion",
                            "ArtworkPresentation",
                            "ItemPresentation",
                            "StatusRegion",
                            "SurfaceScaffold",
                            "ToolRegion",
                        ]
                        .contains(&name.as_str())
                    || root.contains("pub use runtime::{")
                        && ["Popover", "Tooltip"].contains(&name.as_str())
            })
            .collect();
        assert_eq!(
            coverage.keys().cloned().collect::<BTreeSet<_>>(),
            exported_components,
            "update the fail-closed inventory whenever a root-exported Component changes"
        );

        let mut registry = FixtureRegistry::new();
        register(&mut registry).expect("inventory metadata is valid");
        let fixture_ids: BTreeSet<_> = registry
            .finish()
            .into_iter()
            .map(|entry| entry.metadata.id)
            .collect();
        for (primitive, fixture_id) in coverage {
            assert!(
                fixture_ids.contains(fixture_id.as_str()),
                "{primitive} references unknown fixture {fixture_id}"
            );
        }
    }
}
