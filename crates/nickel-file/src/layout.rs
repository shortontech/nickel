use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use nickel_core::theme::ThemePalette;
use twinkle::{
    AnyView, CollectionPresentation, Insets, LinearGradient, NavigationScope, Point, Rect,
    SemanticNodeSnapshot, SemanticRole, SidebarFolder, VerticalScroll, VirtualWindow, ui,
};

use super::{FileMessage, FileViewMode};
use crate::{
    app::{FileApp, NARROW_WORKSPACE_BREAKPOINT, SIDEBAR_RESIZE_WIDTH, TOOLBAR_HEIGHT},
    components,
};

fn selection_surface_drag_message(
    _seed: FileMessage,
    gesture: twinkle::DragGesture,
) -> FileMessage {
    FileMessage::SelectionSurfaceDrag(gesture)
}

pub(crate) fn build_view(
    app: &FileApp,
    _width: f32,
    height: f32,
    palette: ThemePalette,
    light_mode: bool,
) -> AnyView<FileMessage> {
    let narrow = _width < NARROW_WORKSPACE_BREAKPOINT;
    let content_height = (height - TOOLBAR_HEIGHT - 36.0).max(0.0);
    let breadcrumb_width = (_width - if narrow { 390.0 } else { 320.0 }).max(90.0);
    let known_places = app
        .location_groups
        .iter()
        .flat_map(|group| group.entries.iter().cloned())
        .collect::<Vec<_>>();
    let breadcrumbs = collapse_breadcrumbs(
        breadcrumb_paths(app.browser.current(), &known_places),
        breadcrumb_width,
    );
    let tab_strip = components::tab_strip(app, palette, light_mode);
    let navigation = components::navigation_toolbar(app, &breadcrumbs, narrow, palette);
    let toolbar = ui! {
        <Container id={"toolbar-pane"} height={TOOLBAR_HEIGHT} shrink={0.0}
            navigation_scope={NavigationScope::pane(false).direction(app.reading_direction)}
            controller_scope_background={twinkle::focused_surface(palette.panel, palette.complement)}
            background={LinearGradient::vertical(palette.panel, palette.surface)}>
            <Column>{tab_strip}{navigation}</Column>
        </Container>
    };
    let resolved_sidebar_width = if narrow { _width } else { app.sidebar_width };
    let sidebar_source = app.sidebar_collection_source();
    let sidebar_collection = sidebar_source
        .collection(|row| sidebar_row_view(row, app, palette))
        .id("file-sidebar-rows")
        .accessibility_label("Places")
        .presentation(CollectionPresentation::VirtualList {
            item_height: 29.0,
            offset: app.sidebar_scroll_offset,
            viewport_height: (content_height - 20.0).max(1.0),
            overscan: 58.0,
        });
    let sidebar = VerticalScroll::new(
        FileMessage::SidebarScroll(app.sidebar_scroll_offset),
        app.sidebar_scroll_offset,
    )
    .theme(nickel_ui_host::semantic_theme(palette))
    .on_scroll(FileMessage::SidebarScroll)
    .controlled(true)
    .height(content_height)
    .id("file-sidebar-scroll")
    .child(components::places_sidebar(
        resolved_sidebar_width,
        vec![AnyView::new(sidebar_collection)],
        palette,
    ));
    let icon_size = (app.tile_width * 0.42).clamp(42.0, 96.0);
    let file_content_revision = app.active_tab_id.rotate_left(32) ^ app.browser.revision();
    let filtered_indices = app.filtered_indices();
    let empty_message = if app.navigation_pending() {
        if app.status.is_empty() {
            "Loading location…".to_owned()
        } else {
            app.status.clone()
        }
    } else if !app.filter_query.is_empty() && filtered_indices.is_empty() {
        "No files match this filter.".to_owned()
    } else if app.status.is_empty() {
        "This folder is empty.".to_owned()
    } else {
        app.status.clone()
    };
    let files = if filtered_indices.is_empty() {
        ui! {
            <Container id={"file-content"} grow={1.0} padding={Insets::all(28.0)}
                on_press={FileMessage::SelectionSurface} context_message={FileMessage::ContextBackground}
                on_drag={(FileMessage::SelectionSurface, selection_surface_drag_message)}
                focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                accessibility_label={"Files"}>
                <Text color={palette.muted} wrap={true} max_lines={3}>{empty_message}</Text>
            </Container>
        }
    } else {
        let viewport_width = if narrow {
            (_width - 32.0).max(1.0)
        } else {
            (_width - app.sidebar_width - SIDEBAR_RESIZE_WIDTH - 32.0).max(1.0)
        };
        let viewport_height = (height - TOOLBAR_HEIGHT - 36.0 - 28.0 - 1.0).max(1.0);
        let source = app.file_collection_source();
        // Entry buttons already use listing indices for their UI ids; selection
        // and actions resolve those indices through the browser's stable identities.
        let collection = match app.view_mode {
            FileViewMode::Grid => AnyView::new(
                source
                    .collection(move |index| {
                        let index = *index;
                        let entry = &app.browser.entries()[index];
                        components::grid_item(
                            index,
                            entry,
                            file_content_revision,
                            app.is_index_selected(index)
                                || app.native_drop_destination.as_ref() == Some(&entry.path),
                            app.icons
                                .get(&entry.path)
                                .or_else(|| app.shared_icon_for(entry))
                                .cloned(),
                            palette,
                            icon_size,
                        )
                    })
                    .id("file-grid")
                    .accessibility_label("Files")
                    .item_revision(move |_| file_content_revision)
                    .gap(10.0)
                    .navigation_scope(NavigationScope::group().direction(app.reading_direction))
                    .direction(app.reading_direction)
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
                source
                    .collection(move |index| {
                        let index = *index;
                        let entry = &app.browser.entries()[index];
                        components::details_row(
                            index,
                            entry,
                            app.is_index_selected(index)
                                || app.native_drop_destination.as_ref() == Some(&entry.path),
                            app.icons
                                .get(&entry.path)
                                .or_else(|| app.shared_icon_for(entry))
                                .cloned(),
                            palette,
                            light_mode,
                            app.details_column_widths,
                        )
                    })
                    .id("file-details")
                    .accessibility_label("Files")
                    .item_revision(move |_| file_content_revision)
                    .gap(1.0)
                    .navigation_scope(NavigationScope::group().direction(app.reading_direction))
                    .direction(app.reading_direction)
                    .presentation(CollectionPresentation::VirtualList {
                        item_height: 40.0,
                        offset: app.file_scroll_offset,
                        viewport_height,
                        overscan: 80.0,
                    }),
            ),
        };
        let scroll = VerticalScroll::new(
            FileMessage::FileScroll(app.file_scroll_offset),
            app.file_scroll_offset,
        )
        .theme(nickel_ui_host::semantic_theme(palette))
        .on_scroll(FileMessage::FileScroll)
        .controlled(true)
        .height(viewport_height)
        .id("file-list")
        .child({
            ui! {
                <Column>
                    {if app.view_mode == FileViewMode::Details {
                        components::details_header(
                            app.sort_key,
                            app.sort_direction,
                            app.details_column_widths,
                            palette,
                        )
                } else { AnyView::new(ui! { <></> }) }}
                    {collection}
                </Column>
            }
        });
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 14.0, right: 16.0, bottom: 14.0, left: 16.0,
            }}>
                <Container id={"file-content"} height={viewport_height}
                    navigation_scope={NavigationScope::group().direction(app.reading_direction)}
                    on_press={FileMessage::SelectionSurface}
                    on_drag={(FileMessage::SelectionSurface, selection_surface_drag_message)}
                    context_message={FileMessage::ContextBackground} focus_background_tint={palette.accent}
                    controller_focus_background_tint={palette.complement} accessibility_label={"Files background"}>
                    {scroll}
                </Container>
            </Column>
        }
    };
    let footer_text = components::status_text(app);
    let footer_accessibility_text = components::status_accessibility_text(app);
    let footer = components::status_bar(app, footer_text, footer_accessibility_text, palette);
    let resize_handle = ui! {
        <Container id={"sidebar-resize"} width={SIDEBAR_RESIZE_WIDTH} shrink={0.0}
            background={if app.is_resizing_sidebar() { palette.accent } else { palette.surface_hover }}
            on_press={FileMessage::ResizeSidebar} focus_background_tint={palette.accent}
            controller_focus_background_tint={palette.complement} accessibility_label={"Resize sidebar"} />
    };
    let sidebar_pane_width = app.sidebar_width + SIDEBAR_RESIZE_WIDTH;
    let content = if app.command_surface_open {
        ui! {
            <Container id={"file-layout"} height={content_height} shrink={0.0} accessibility_label={"Commands"}>
                {components::command_surface(app, content_height, palette)}
            </Container>
        }
    } else if narrow {
        ui! {
            <Container id={"file-layout"} height={content_height} shrink={0.0} accessibility_label={"Files"}>
                {if app.places_open {
                    ui! {
                        <Container id={"narrow-places-surface"} grow={1.0}
                            navigation_scope={NavigationScope::pane(true).direction(app.reading_direction)}
                            controller_scope_background={twinkle::focused_surface(palette.surface, palette.complement)}>
                            {sidebar}
                        </Container>
                    }
                } else {
                    ui! {
                        <Container id={"files-pane"} grow={1.0} min_width={0.0}
                            navigation_scope={NavigationScope::pane(true).direction(app.reading_direction)}
                            controller_scope_background={twinkle::focused_surface(palette.background, palette.surface_hover)}>
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
                    navigation_scope={NavigationScope::pane(false).direction(app.reading_direction)}
                    controller_scope_background={twinkle::focused_surface(palette.surface, palette.complement)}>
                    <Row width={sidebar_pane_width} shrink={0.0}>{sidebar}{resize_handle}</Row>
                </Container>
                <Container id={"files-pane"} grow={1.0} min_width={0.0}
                    navigation_scope={NavigationScope::pane(true).direction(app.reading_direction)}
                    controller_scope_background={twinkle::focused_surface(palette.background, palette.surface_hover)}>{files}</Container></Row>
            </Container>
        }
    };
    let root = ui! {
        <Column height={height} background={palette.background}>{toolbar}{content}{footer}</Column>
    };
    AnyView::new(root)
}

pub(crate) fn visible_file_range(app: &FileApp, width: f32, height: f32) -> std::ops::Range<usize> {
    let count = app.filtered_indices().len();
    if count == 0 {
        return 0..0;
    }
    let viewport_width = if width < NARROW_WORKSPACE_BREAKPOINT {
        (width - 32.0).max(1.0)
    } else {
        (width - app.sidebar_width - SIDEBAR_RESIZE_WIDTH - 32.0).max(1.0)
    };
    let viewport_height = (height - TOOLBAR_HEIGHT - 36.0 - 28.0 - 1.0).max(1.0);
    if app.view_mode == FileViewMode::Details {
        let first = (app.file_scroll_offset / 41.0).floor().max(0.0) as usize;
        let last = ((app.file_scroll_offset + viewport_height) / 41.0).ceil() as usize;
        return first.saturating_sub(2).min(count)..last.saturating_add(2).min(count);
    }
    let gap = 10.0;
    let columns = (((viewport_width + gap) / (app.tile_width.max(1.0) + gap)).floor() as usize)
        .max(1)
        .min(count.max(1));
    let row_height = 54.0 + (app.tile_width * 0.42).clamp(42.0, 96.0);
    let rows = count.div_ceil(columns);
    let window = VirtualWindow::from_uniform(
        rows,
        row_height,
        gap,
        app.file_scroll_offset,
        viewport_height,
        row_height * 2.0,
    );
    (window.range.start * columns).min(count)..(window.range.end * columns).min(count)
}

pub(crate) fn breadcrumb_paths(
    current: &Path,
    places: &[(String, PathBuf)],
) -> Vec<(String, PathBuf)> {
    let anchor = places
        .iter()
        .filter(|(_, path)| current.starts_with(path))
        .max_by_key(|(_, path)| path.components().count())
        .cloned();
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

pub(crate) enum SidebarRow {
    Header {
        key: String,
        id: String,
        title: String,
        collapsed: bool,
    },
    Folder {
        key: String,
        label: String,
        path: PathBuf,
        depth: usize,
    },
}

impl SidebarRow {
    pub(crate) fn key(&self) -> String {
        match self {
            Self::Header { key, .. } | Self::Folder { key, .. } => key.clone(),
        }
    }
}

pub(crate) fn sidebar_rows(app: &FileApp) -> Vec<SidebarRow> {
    fn append_folder(
        rows: &mut Vec<SidebarRow>,
        key_prefix: &str,
        label: &str,
        path: &Path,
        depth: usize,
        expanded: &HashSet<PathBuf>,
        children_by_path: &HashMap<PathBuf, Vec<(String, PathBuf)>>,
    ) {
        rows.push(SidebarRow::Folder {
            key: format!("folder/{key_prefix}/{path:?}"),
            label: label.to_owned(),
            path: path.to_path_buf(),
            depth,
        });
        if !expanded.contains(path) || depth >= 6 {
            return;
        }
        if let Some(children) = children_by_path.get(path) {
            for (label, child) in children {
                append_folder(
                    rows,
                    key_prefix,
                    label,
                    child,
                    depth + 1,
                    expanded,
                    children_by_path,
                );
            }
        }
    }

    let mut rows = Vec::new();
    for group in &app.location_groups {
        let collapsed = app.collapsed_location_groups.contains(group.id);
        rows.push(SidebarRow::Header {
            key: format!("header/{}", group.id),
            id: group.id.to_owned(),
            title: group.title.to_owned(),
            collapsed,
        });
        if !collapsed {
            for (root_index, (label, path)) in group.entries.iter().enumerate() {
                let key_prefix = format!("{}/{root_index}", group.id);
                append_folder(
                    &mut rows,
                    &key_prefix,
                    label,
                    path,
                    0,
                    &app.sidebar.expanded,
                    &app.sidebar.children,
                );
            }
        }
    }
    rows
}

fn sidebar_row_view(
    row: &SidebarRow,
    app: &FileApp,
    palette: ThemePalette,
) -> AnyView<FileMessage> {
    match row {
        SidebarRow::Header {
            id,
            title,
            collapsed,
            ..
        } => AnyView::new(components::location_group_header(
            id, title, *collapsed, palette,
        )),
        SidebarRow::Folder {
            label, path, depth, ..
        } => {
            let is_expanded = app.sidebar.expanded.contains(path);
            let is_active = app.browser.current() == path;
            let mut row = SidebarFolder::new(
                FileMessage::ToggleFolder(path.clone()),
                FileMessage::OpenFolder(path.clone()),
                label.clone(),
                is_expanded,
                if is_active {
                    palette.text
                } else {
                    palette.muted
                },
            )
            .open_id(crate::app::drop_target_id("sidebar", path))
            .row_height(28.0)
            .accessibility_labels((format!("Toggle {label}"), format!("Open {label}")))
            .focus_background_tints((palette.accent, palette.complement))
            .indent(*depth)
            .background(if is_active {
                palette.accent_soft
            } else {
                palette.panel
            });
            if let Some((id, image)) = app.icons.get(path) {
                row = row.artwork(*id, image.clone(), u64::from(*id));
            }
            AnyView::new(row)
        }
    }
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
