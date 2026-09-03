use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use nickel_core::theme::ThemePalette;
use nickel_ui::{
    AnyView, Collection, CollectionPresentation, CollectionState, ComponentBuilderExt, Insets,
    LinearGradient, NavigationScope, Point, Rect, SemanticNodeSnapshot, SemanticRole, TextField,
    VerticalScroll, VirtualWindow, ui,
};

use super::{FileMessage, FileViewMode};
use crate::{
    EntrySortKey, SortDirection,
    app::{FileApp, NARROW_WORKSPACE_BREAKPOINT, SIDEBAR_RESIZE_WIDTH, TOOLBAR_HEIGHT},
    components,
    platform::{location_groups, places},
};

pub(crate) fn build_view(
    app: &FileApp,
    _width: f32,
    height: f32,
    palette: ThemePalette,
    light_mode: bool,
) -> AnyView<FileMessage> {
    let narrow = _width < NARROW_WORKSPACE_BREAKPOINT;
    let content_height = (height - TOOLBAR_HEIGHT - 30.0).max(0.0);
    let breadcrumb_width = (_width - if narrow { 390.0 } else { 320.0 }).max(90.0);
    let breadcrumbs =
        collapse_breadcrumbs(breadcrumb_paths(app.browser.current()), breadcrumb_width);
    let tab_strip = components::tab_strip(app, palette, light_mode);
    let breadcrumb_row = ui! {
        <Row gap={7.0}>
            {breadcrumbs.iter().enumerate().map(|(index, (label, path))| ui! {
                <Row gap={7.0}>
                    {if index > 0 {
                        ui! { <Text width={10.0} color={palette.muted}>{"›"}</Text> }
                    } else {
                        ui! { <></> }
                    }}
                    <Container on_press={FileMessage::Breadcrumb(path.clone())}
                        focus_border={palette.accent} controller_focus_border={palette.complement} padding={Insets {
                        top: 4.0, right: 2.0, bottom: 3.0, left: 2.0,
                    }} accessibility_label={format!("Open {label}")}>
                        <Text scale={1.05} color={palette.text}>{label}</Text>
                    </Container>
                </Row>
            })}
        </Row>
    };
    let location_control = if app.address_editing {
        AnyView::new(ui! {
            <Container grow={1.0} background={palette.background} border={(palette.accent, 1.0)} padding={Insets {
                top: 5.0, right: 12.0, bottom: 4.0, left: 10.0,
            }}>
                {TextField::on_change(&app.address_text, address_changed)
                    .id("file-address-field")
                    .accessibility_label("Location")
                    .color(palette.text)}
            </Container>
        })
    } else {
        AnyView::new(ui! {
            <Container grow={1.0} background={palette.background} padding={Insets {
                top: 5.0, right: 12.0, bottom: 4.0, left: 10.0,
            }}>
                {breadcrumb_row}
            </Container>
        })
    };
    let navigation = ui! {
        <Container id={"navigation-toolbar"} height={46.0} background={palette.surface} padding={Insets {
            top: 6.0, right: 12.0, bottom: 6.0, left: 10.0,
        }}>
            <Row gap={4.0}>
                {if narrow {
                    ui! {
                        <Button on_press={FileMessage::TogglePlaces} width={74.0} height={34.0}
                            color={if app.places_open { palette.accent } else { palette.text }}
                            focus_border={palette.accent} controller_focus_border={palette.complement}
                            accessibility_label={if app.places_open { "Close places" } else { "Open places" }}>
                            {if app.places_open { "Files" } else { "Places" }}
                        </Button>
                    }
                } else { ui! { <></> } }}
                <Button on_press={FileMessage::Back} enabled={app.browser.can_go_back() && !app.navigation_pending()} width={34.0} height={34.0}
                    focus_border={palette.accent} controller_focus_border={palette.complement}
                    color={if app.browser.can_go_back() { palette.text } else { palette.muted }} accessibility_label={"Back"}>
                    {"←"}
                </Button>
                <Button on_press={FileMessage::Forward} enabled={app.browser.can_go_forward() && !app.navigation_pending()} width={34.0} height={34.0}
                    focus_border={palette.accent} controller_focus_border={palette.complement}
                    color={if app.browser.can_go_forward() { palette.text } else { palette.muted }} accessibility_label={"Forward"}>
                    {"→"}
                </Button>
                <Button on_press={FileMessage::Up} enabled={app.browser.can_go_up() && !app.navigation_pending()} width={34.0} height={34.0} color={palette.text}
                    focus_border={palette.accent} controller_focus_border={palette.complement} accessibility_label={"Up one folder"}>{"↑"}</Button>
                {location_control}
                <Button on_press={FileMessage::ToggleAddressEditing} width={34.0} height={34.0}
                    color={if app.address_editing { palette.accent } else { palette.text }}
                    focus_border={palette.accent} controller_focus_border={palette.complement}
                    accessibility_label={if app.address_editing { "Cancel location editing" } else { "Edit location" }}>
                    {if app.address_editing { "×" } else { "✎" }}
                </Button>
                <Button on_press={FileMessage::Refresh} enabled={!app.navigation_pending()} width={34.0} height={34.0} color={palette.text}
                    focus_border={palette.accent} controller_focus_border={palette.complement} accessibility_label={"Refresh"}>{"↻"}</Button>
                <Button on_press={FileMessage::SetViewMode(FileViewMode::Grid)} width={34.0} height={34.0}
                    color={if app.view_mode == FileViewMode::Grid { palette.accent } else { palette.text }}
                    focus_border={palette.accent} controller_focus_border={palette.complement} accessibility_label={"Grid view"}>{"▦"}</Button>
                <Button on_press={FileMessage::SetViewMode(FileViewMode::Details)} width={34.0} height={34.0}
                    color={if app.view_mode == FileViewMode::Details { palette.accent } else { palette.text }}
                    focus_border={palette.accent} controller_focus_border={palette.complement} accessibility_label={"Details view"}>{"☷"}</Button>
                <Button on_press={FileMessage::ToggleCommandSurface} width={42.0} height={34.0}
                    color={if app.command_surface_open { palette.accent } else { palette.text }}
                    focus_border={palette.accent} controller_focus_border={palette.complement}
                    accessibility_label={"Open commands"}>{"⌘"}</Button>
            </Row>
        </Container>
    };
    let toolbar = ui! {
        <Container id={"toolbar-pane"} height={TOOLBAR_HEIGHT} shrink={0.0}
            navigation_scope={NavigationScope::pane(false)} navigation_scope_highlight={palette.complement}
            background={LinearGradient::vertical(palette.panel, palette.surface)}>
            <Column>{tab_strip}{navigation}</Column>
        </Container>
    };
    let location_groups = location_groups()
        .into_iter()
        .map(|group| {
            let rows = sidebar_folder_elements(
                &group.entries,
                &app.expanded_folders,
                app.browser.current(),
                None,
                palette,
            );
            components::location_group(
                group.id,
                group.title,
                rows,
                app.collapsed_location_groups.contains(group.id),
                palette,
            )
        })
        .collect();
    let resolved_sidebar_width = if narrow { _width } else { app.sidebar_width };
    let sidebar = components::places_sidebar(resolved_sidebar_width, location_groups, palette);
    let icon_size = (app.tile_width * 0.42).clamp(42.0, 96.0);
    let tile_rows = app
        .browser
        .entries()
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            (
                index,
                entry.clone(),
                app.selected_entries.contains(&index),
                app.icons.get(&entry.path).cloned(),
            )
        })
        .collect::<Vec<_>>();
    let files = if app.browser.entries().is_empty() {
        ui! {
            <Container id={"file-content"} grow={1.0} padding={Insets::all(28.0)}
                on_press={FileMessage::SelectionSurface} context_message={FileMessage::ContextBackground}
                focus_border={palette.accent} controller_focus_border={palette.complement}
                accessibility_label={"Files"}>
                <Text color={palette.muted}>{"This folder is empty."}</Text>
            </Container>
        }
    } else {
        let viewport_width = if narrow {
            (_width - 32.0).max(1.0)
        } else {
            (_width - app.sidebar_width - SIDEBAR_RESIZE_WIDTH - 32.0).max(1.0)
        };
        let viewport_height = (height - TOOLBAR_HEIGHT - 30.0 - 28.0 - 1.0).max(1.0);
        let collection = match app.view_mode {
            FileViewMode::Grid => AnyView::new(
                Collection::try_new(
                    CollectionState::Ready(tile_rows),
                    |(_, entry, _, _)| entry.path.to_string_lossy().into_owned(),
                    move |(index, entry, selected, icon)| {
                        components::grid_item(
                            index, &entry, selected, icon, palette, icon_size, light_mode,
                        )
                    },
                )
                .expect("directory entries have unique paths")
                .id("file-grid")
                .accessibility_label("Files")
                .gap(10.0)
                .navigation_scope(NavigationScope::group())
                .presentation(CollectionPresentation::VirtualGrid {
                    minimum_item_width: app.tile_width,
                    row_height: 54.0 + icon_size,
                    offset: app.file_scroll_offset,
                    viewport_width,
                    viewport_height,
                    overscan: (54.0 + icon_size) * 2.0,
                }),
            ),
            FileViewMode::Details => AnyView::new(
                Collection::try_new(
                    CollectionState::Ready(tile_rows),
                    |(_, entry, _, _)| entry.path.to_string_lossy().into_owned(),
                    move |(index, entry, selected, icon)| {
                        components::details_row(
                            index,
                            &entry,
                            selected,
                            icon,
                            palette,
                            light_mode,
                            app.details_column_widths,
                        )
                    },
                )
                .expect("directory entries have unique paths")
                .id("file-details")
                .accessibility_label("Files")
                .gap(1.0)
                .navigation_scope(NavigationScope::group())
                .presentation(CollectionPresentation::VirtualList {
                    item_height: 58.0,
                    offset: app.file_scroll_offset,
                    viewport_height,
                    overscan: 116.0,
                }),
            ),
        };
        let scroll = VerticalScroll::new(
            FileMessage::FileScroll(app.file_scroll_offset),
            app.file_scroll_offset,
        )
        .on_scroll(FileMessage::FileScroll)
        .controlled(true)
        .height(viewport_height)
        .id("file-list")
        .child({
            let sort_label = |label: &str, key: EntrySortKey| {
                if app.sort_key != key {
                    label.to_owned()
                } else {
                    format!(
                        "{label} {}",
                        match app.sort_direction {
                            SortDirection::Ascending => "↑",
                            SortDirection::Descending => "↓",
                        }
                    )
                }
            };
            let name_sort = sort_label("Name", EntrySortKey::Name);
            let type_sort = sort_label("Type", EntrySortKey::Type);
            let modified_sort = sort_label("Modified", EntrySortKey::Modified);
            let size_sort = sort_label("Size", EntrySortKey::Size);
            ui! {
            <Column>
                {if app.view_mode == FileViewMode::Details {
                    ui! {
                        <Container id={"details-header"} height={32.0} background={palette.surface} padding={Insets {
                            top: 8.0, right: 10.0, bottom: 6.0, left: 10.0,
                        }}>
                            <Row gap={12.0}>
                                <Text width={32.0} color={palette.muted}>{""}</Text>
                                <Container grow={1.0} min_width={120.0} on_press={FileMessage::SortBy(EntrySortKey::Name)}
                                    focus_border={palette.accent} controller_focus_border={palette.complement}
                                    accessibility_label={"Sort by name"}>
                                    <Text color={palette.muted}>{name_sort}</Text>
                                </Container>
                                <Container width={app.details_column_widths.type_width} on_press={FileMessage::SortBy(EntrySortKey::Type)}
                                    focus_border={palette.accent} controller_focus_border={palette.complement}
                                    accessibility_label={"Sort by type"}><Text color={palette.muted}>{type_sort}</Text></Container>
                                <Container id={"resize-details-type"} width={5.0} on_press={FileMessage::ResizeDetailsColumn(crate::app::DetailsColumn::Type)}
                                    background={palette.surface_hover} focus_border={palette.accent} accessibility_label={"Resize type column"} />
                                <Container width={app.details_column_widths.modified_width} on_press={FileMessage::SortBy(EntrySortKey::Modified)}
                                    focus_border={palette.accent} controller_focus_border={palette.complement}
                                    accessibility_label={"Sort by modified time"}><Text color={palette.muted}>{modified_sort}</Text></Container>
                                <Container id={"resize-details-modified"} width={5.0} on_press={FileMessage::ResizeDetailsColumn(crate::app::DetailsColumn::Modified)}
                                    background={palette.surface_hover} focus_border={palette.accent} accessibility_label={"Resize modified column"} />
                                <Container width={app.details_column_widths.size_width} on_press={FileMessage::SortBy(EntrySortKey::Size)}
                                    focus_border={palette.accent} controller_focus_border={palette.complement}
                                    accessibility_label={"Sort by size"}><Text color={palette.muted}>{size_sort}</Text></Container>
                                <Container id={"resize-details-size"} width={5.0} on_press={FileMessage::ResizeDetailsColumn(crate::app::DetailsColumn::Size)}
                                    background={palette.surface_hover} focus_border={palette.accent} accessibility_label={"Resize size column"} />
                            </Row>
                        </Container>
                    }
                } else { ui! { <></> } }}
                {collection}
            </Column>
        }});
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 14.0, right: 16.0, bottom: 14.0, left: 16.0,
            }}>
                {scroll}
                <Container id={"file-content"} height={1.0} on_press={FileMessage::SelectionSurface}
                    context_message={FileMessage::ContextBackground} focus_border={palette.accent}
                    controller_focus_border={palette.complement} accessibility_label={"Files background"} />
            </Column>
        }
    };
    let footer_text = components::status_text(app);
    let footer = components::status_bar(footer_text, palette);
    let resize_handle = ui! {
        <Container id={"sidebar-resize"} width={SIDEBAR_RESIZE_WIDTH} shrink={0.0}
            background={if app.is_resizing_sidebar() { palette.accent } else { palette.surface_hover }}
            on_press={FileMessage::ResizeSidebar} focus_border={palette.accent}
            controller_focus_border={palette.complement} accessibility_label={"Resize sidebar"} />
    };
    let sidebar_pane_width = app.sidebar_width + SIDEBAR_RESIZE_WIDTH;
    let content = if app.command_surface_open {
        ui! {
            <Container id={"file-layout"} height={content_height} shrink={0.0} accessibility_label={"Commands"}>
                {command_surface(app, palette)}
            </Container>
        }
    } else if narrow {
        ui! {
            <Container id={"file-layout"} height={content_height} shrink={0.0} accessibility_label={"Files"}>
                {if app.places_open {
                    ui! {
                        <Container id={"narrow-places-surface"} grow={1.0}
                            navigation_scope={NavigationScope::pane(true)} navigation_scope_highlight={palette.complement}>
                            {sidebar}
                        </Container>
                    }
                } else {
                    ui! {
                        <Container id={"files-pane"} grow={1.0} min_width={0.0}
                            navigation_scope={NavigationScope::pane(true)} navigation_scope_highlight={palette.complement}>
                            {files}
                        </Container>
                    }
                }}
            </Container>
        }
    } else {
        ui! {
            <Container id={"file-layout"} height={content_height} shrink={0.0} accessibility_label={"Files"}>
                <Row grow={1.0}><Container id={"sidebar-pane"} width={sidebar_pane_width} shrink={0.0}
                    navigation_scope={NavigationScope::pane(false)} navigation_scope_highlight={palette.complement}>
                    <Row width={sidebar_pane_width} shrink={0.0}>{sidebar}{resize_handle}</Row>
                </Container>
                    <Container id={"files-pane"} grow={1.0} min_width={0.0}
                    navigation_scope={NavigationScope::pane(true)} navigation_scope_highlight={palette.complement}>{files}</Container></Row>
            </Container>
        }
    };
    let root = ui! {
        <Column height={height} background={palette.background}>{toolbar}{content}{footer}</Column>
    };
    AnyView::new(root)
}

fn command_query_message(query: String) -> FileMessage {
    FileMessage::CommandQueryChanged(query)
}

fn address_changed(address: String) -> FileMessage {
    FileMessage::AddressChanged(address)
}

fn command_surface(app: &FileApp, palette: ThemePalette) -> AnyView<FileMessage> {
    let query = app.command_query.trim().to_ascii_lowercase();
    let commands = [
        (
            "Open",
            "activate enter",
            app.selected.is_some() && !app.navigation_pending(),
            FileMessage::ContextOpen,
        ),
        (
            "Open in new tab",
            "activate background tab",
            app.selected.is_some() && !app.navigation_pending(),
            FileMessage::ContextOpenNewTab,
        ),
        (
            "Back",
            "previous navigation",
            app.browser.can_go_back() && !app.navigation_pending(),
            FileMessage::Back,
        ),
        (
            "Forward",
            "next navigation",
            app.browser.can_go_forward() && !app.navigation_pending(),
            FileMessage::Forward,
        ),
        (
            "Up",
            "parent folder",
            app.browser.can_go_up() && !app.navigation_pending(),
            FileMessage::Up,
        ),
        (
            "Refresh",
            "reload f5",
            !app.navigation_pending(),
            FileMessage::Refresh,
        ),
        ("New tab", "create tab ctrl t", true, FileMessage::NewTab),
        (
            "Close tab",
            "dismiss tab ctrl w",
            true,
            FileMessage::CloseTab(app.active_tab),
        ),
        (
            "Grid view",
            "icons thumbnails",
            true,
            FileMessage::SetViewMode(FileViewMode::Grid),
        ),
        (
            "Details view",
            "list columns",
            true,
            FileMessage::SetViewMode(FileViewMode::Details),
        ),
        (
            "Select all",
            "selection ctrl a",
            !app.browser.entries().is_empty(),
            FileMessage::ContextSelectAll,
        ),
        (
            "Sort by name",
            "order filename",
            true,
            FileMessage::SortBy(EntrySortKey::Name),
        ),
        (
            "Sort by type",
            "order extension",
            true,
            FileMessage::SortBy(EntrySortKey::Type),
        ),
        (
            "Sort by modified",
            "order date time",
            true,
            FileMessage::SortBy(EntrySortKey::Modified),
        ),
        (
            "Sort by size",
            "order bytes",
            true,
            FileMessage::SortBy(EntrySortKey::Size),
        ),
        (
            if app.browser.show_hidden() {
                "Hide hidden files"
            } else {
                "Show hidden files"
            },
            "dotfiles visibility",
            !app.navigation_pending(),
            FileMessage::ToggleHiddenFiles,
        ),
    ];
    let rows = commands
        .into_iter()
        .filter(|(label, aliases, _, _)| {
            query.is_empty()
                || label.to_ascii_lowercase().contains(&query)
                || aliases.contains(&query)
        })
        .enumerate()
        .map(|(index, (label, aliases, enabled, message))| {
            AnyView::new(ui! {
                <Container id={format!("file-command-{index}")} height={42.0}
                    on_press={message} enabled={enabled} background={palette.surface}
                    hover_background={palette.surface_hover} pressed_background={palette.accent_soft}
                    focus_border={palette.accent} controller_focus_border={palette.complement}
                    padding={Insets { top: 9.0, right: 12.0, bottom: 7.0, left: 12.0 }}
                    semantic_role={SemanticRole::Button} accessibility_label={label}>
                    <Row gap={12.0}>
                        <Text width={180.0} color={if enabled { palette.text } else { palette.muted }}>{label}</Text>
                        <Text color={palette.muted} scale={0.9}>{aliases}</Text>
                    </Row>
                </Container>
            })
        })
        .collect::<Vec<_>>();
    let results = if rows.is_empty() {
        AnyView::new(ui! {
            <Container padding={Insets::all(16.0)}><Text color={palette.muted}>{"No matching commands."}</Text></Container>
        })
    } else {
        AnyView::new(ui! { <Column gap={2.0} children={rows} /> })
    };
    AnyView::new(ui! {
        <Container id={"file-command-surface"} grow={1.0} background={palette.background}
            padding={Insets { top: 24.0, right: 32.0, bottom: 24.0, left: 32.0 }}>
            <Column gap={10.0}>
                <Text color={palette.text} scale={1.35}>{"Commands"}</Text>
                <Container height={40.0} background={palette.surface} border={(palette.accent, 1.0)}
                    padding={Insets { top: 9.0, right: 12.0, bottom: 7.0, left: 12.0 }}>
                    {TextField::on_change_with_placeholder(
                        &app.command_query,
                        "Type a command…",
                        command_query_message,
                    ).id("file-command-query").color(palette.text)}
                </Container>
                {results}
            </Column>
        </Container>
    })
}

pub(crate) fn visible_file_range(app: &FileApp, width: f32, height: f32) -> std::ops::Range<usize> {
    let count = app.browser.entries().len();
    let viewport_width = if width < NARROW_WORKSPACE_BREAKPOINT {
        (width - 32.0).max(1.0)
    } else {
        (width - app.sidebar_width - SIDEBAR_RESIZE_WIDTH - 32.0).max(1.0)
    };
    let viewport_height = (height - TOOLBAR_HEIGHT - 30.0 - 28.0 - 1.0).max(1.0);
    let gap = 10.0;
    let columns = (((viewport_width + gap) / (app.tile_width.max(1.0) + gap)).floor() as usize)
        .max(1)
        .min(count.max(1));
    let row_height = 54.0 + (app.tile_width * 0.42).clamp(42.0, 96.0);
    let rows = count.div_ceil(columns);
    let window = VirtualWindow::from_heights(
        &vec![row_height; rows],
        gap,
        app.file_scroll_offset,
        viewport_height,
        row_height * 2.0,
    );
    (window.range.start * columns)..(window.range.end * columns).min(count)
}

pub(crate) fn breadcrumb_paths(current: &Path) -> Vec<(String, PathBuf)> {
    let anchor = places()
        .into_iter()
        .filter(|(_, path)| current.starts_with(path))
        .max_by_key(|(_, path)| path.components().count());
    let Some((anchor_label, anchor_path)) = anchor else {
        return vec![(current.display().to_string(), current.to_path_buf())];
    };

    let mut breadcrumbs = vec![(anchor_label, anchor_path.clone())];
    let mut path = anchor_path;
    if let Ok(relative) = current.strip_prefix(&path) {
        for component in relative.components() {
            path.push(component.as_os_str());
            breadcrumbs.push((
                component.as_os_str().to_string_lossy().into_owned(),
                path.clone(),
            ));
        }
    }
    breadcrumbs
}

pub(crate) fn collapse_breadcrumbs(
    breadcrumbs: Vec<(String, PathBuf)>,
    available_width: f32,
) -> Vec<(String, PathBuf)> {
    let item_width = |label: &str| label.chars().count() as f32 * 8.0 + 11.0;
    let full_width = breadcrumbs
        .iter()
        .map(|(label, _)| item_width(label))
        .sum::<f32>()
        + breadcrumbs.len().saturating_sub(1) as f32 * 17.0;
    if breadcrumbs.len() <= 2 || full_width <= available_width {
        return breadcrumbs;
    }

    let root = breadcrumbs[0].clone();
    let mut first_suffix = breadcrumbs.len() - 1;
    let mut used = item_width(&root.0)
        + 17.0
        + item_width("…")
        + 17.0
        + item_width(&breadcrumbs[first_suffix].0);
    while first_suffix > 1 {
        let candidate = item_width(&breadcrumbs[first_suffix - 1].0) + 17.0;
        if used + candidate > available_width {
            break;
        }
        first_suffix -= 1;
        used += candidate;
    }

    let mut collapsed = Vec::with_capacity(breadcrumbs.len() - first_suffix + 2);
    collapsed.push(root);
    collapsed.push(("…".into(), breadcrumbs[first_suffix - 1].1.clone()));
    collapsed.extend(breadcrumbs.into_iter().skip(first_suffix));
    collapsed
}

pub(crate) fn sidebar_folder_elements(
    roots: &[(String, PathBuf)],
    expanded: &HashSet<PathBuf>,
    current: &Path,
    hovered_message: Option<&FileMessage>,
    palette: ThemePalette,
) -> Vec<AnyView<FileMessage>> {
    #[allow(clippy::too_many_arguments)]
    fn append_folder(
        rows: &mut Vec<AnyView<FileMessage>>,
        label: String,
        path: PathBuf,
        depth: usize,
        expanded: &HashSet<PathBuf>,
        current: &Path,
        hovered_message: Option<&FileMessage>,
        palette: ThemePalette,
    ) {
        let is_expanded = expanded.contains(&path);
        let is_active = current == path;
        let toggle_message = FileMessage::ToggleFolder(path.clone());
        let open_message = FileMessage::OpenFolder(path.clone());
        let is_hovered =
            hovered_message == Some(&toggle_message) || hovered_message == Some(&open_message);
        rows.push(AnyView::new(ui! {
            <SidebarFolder on_toggle={toggle_message} on_open={open_message} label={label.clone()}
                accessibility_labels={(format!("Toggle {label}"), format!("Open {label}"))}
                focus_borders={(palette.accent, palette.complement)}
                expanded={is_expanded} foreground={if is_active { palette.text } else { palette.muted }}
                indent={depth} background={if is_active {
                    palette.accent_soft
                } else if is_hovered {
                    palette.surface_hover
                } else {
                    palette.panel
                }} />
        }));
        if !is_expanded || depth >= 6 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&path) else {
            return;
        };
        let mut children = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry
                    .file_type()
                    .ok()
                    .filter(|kind| kind.is_dir())
                    .map(|_| {
                        (
                            entry.file_name().to_string_lossy().into_owned(),
                            entry.path(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        children.sort_by(|left, right| left.0.to_lowercase().cmp(&right.0.to_lowercase()));
        for (label, child) in children {
            append_folder(
                rows,
                label,
                child,
                depth + 1,
                expanded,
                current,
                hovered_message,
                palette,
            );
        }
    }

    let mut rows = Vec::new();
    for (label, path) in roots {
        append_folder(
            &mut rows,
            label.clone(),
            path.clone(),
            0,
            expanded,
            current,
            hovered_message,
            palette,
        );
    }
    rows
}

pub(crate) fn rect_between(start: Point, end: Point) -> Rect {
    Rect::new(
        start.x.min(end.x),
        start.y.min(end.y),
        (start.x - end.x).abs().max(1.0),
        (start.y - end.y).abs().max(1.0),
    )
}

pub(crate) fn rects_intersect(left: Rect, right: Rect) -> bool {
    left.origin.x < right.origin.x + right.size.width
        && left.origin.x + left.size.width > right.origin.x
        && left.origin.y < right.origin.y + right.size.height
        && left.origin.y + left.size.height > right.origin.y
}

pub(crate) fn entries_in_selection(
    nodes: &[SemanticNodeSnapshot],
    selection: Rect,
    entry_count: usize,
) -> HashSet<usize> {
    let grids = nodes
        .iter()
        .filter(|node| {
            node.role == Some(SemanticRole::Grid) && node.name.as_deref() == Some("Files")
        })
        .collect::<Vec<_>>();
    let [grid] = grids.as_slice() else {
        return HashSet::new();
    };
    let grid_prefix = format!("{}/", grid.id.as_str());
    nodes
        .iter()
        .filter(|node| node.role == Some(SemanticRole::Button))
        .filter(|node| node.id.as_str().starts_with(&grid_prefix))
        .filter_map(|node| {
            let index = node
                .id
                .as_str()
                .rsplit("/file-entry-")
                .next()?
                .parse::<usize>()
                .ok()?;
            (index < entry_count && rects_intersect(selection, node.bounds)).then_some(index)
        })
        .collect()
}
