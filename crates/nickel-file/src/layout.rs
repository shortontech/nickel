use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use nickel_core::theme::ThemePalette;
use nickel_ui::{
    AnyView, Collection, CollectionPresentation, CollectionState, Insets, LinearGradient,
    NavigationScope, Point, Rect, SemanticNodeSnapshot, SemanticRole, SidebarFolder,
    VerticalScroll, VirtualWindow, ui,
};

use super::{FileMessage, FileViewMode};
use crate::{
    app::{FileApp, NARROW_WORKSPACE_BREAKPOINT, SIDEBAR_RESIZE_WIDTH, TOOLBAR_HEIGHT},
    components, icons,
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
            navigation_scope={NavigationScope::pane(false).direction(app.reading_direction)} navigation_scope_highlight={palette.complement}
            background={LinearGradient::vertical(palette.panel, palette.surface)}>
            <Column>{tab_strip}{navigation}</Column>
        </Container>
    };
    let location_groups = app
        .location_groups
        .iter()
        .map(|group| {
            let rows = sidebar_folder_elements(
                &group.entries,
                &app.expanded_folders,
                app.browser.current(),
                None,
                &app.icons,
                &app.sidebar_children,
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
    let empty_message = if app.navigation_pending() {
        if app.status.is_empty() {
            "Loading location…".to_owned()
        } else {
            app.status.clone()
        }
    } else if app.status.is_empty() {
        "This folder is empty.".to_owned()
    } else {
        app.status.clone()
    };
    let files = if app.browser.entries().is_empty() {
        ui! {
            <Container id={"file-content"} grow={1.0} padding={Insets::all(28.0)}
                on_press={FileMessage::SelectionSurface} context_message={FileMessage::ContextBackground}
                focus_border={palette.accent} controller_focus_border={palette.complement}
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
                .navigation_scope(NavigationScope::group().direction(app.reading_direction))
                .direction(app.reading_direction)
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
                {components::command_surface(app, content_height, palette)}
            </Container>
        }
    } else if narrow {
        ui! {
            <Container id={"file-layout"} height={content_height} shrink={0.0} accessibility_label={"Files"}>
                {if app.places_open {
                    ui! {
                        <Container id={"narrow-places-surface"} grow={1.0}
                            navigation_scope={NavigationScope::pane(true).direction(app.reading_direction)} navigation_scope_highlight={palette.complement}>
                            {sidebar}
                        </Container>
                    }
                } else {
                    ui! {
                        <Container id={"files-pane"} grow={1.0} min_width={0.0}
                            navigation_scope={NavigationScope::pane(true).direction(app.reading_direction)} navigation_scope_highlight={palette.surface_hover}>
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
                    navigation_scope={NavigationScope::pane(false).direction(app.reading_direction)} navigation_scope_highlight={palette.complement}>
                    <Row width={sidebar_pane_width} shrink={0.0}>{sidebar}{resize_handle}</Row>
                </Container>
                <Container id={"files-pane"} grow={1.0} min_width={0.0}
                    navigation_scope={NavigationScope::pane(true).direction(app.reading_direction)} navigation_scope_highlight={palette.surface_hover}>{files}</Container></Row>
            </Container>
        }
    };
    let root = ui! {
        <Column height={height} background={palette.background}>{toolbar}{content}{footer}</Column>
    };
    AnyView::new(root)
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

pub(crate) fn sidebar_folder_elements(
    roots: &[(String, PathBuf)],
    expanded: &HashSet<PathBuf>,
    current: &Path,
    hovered_message: Option<&FileMessage>,
    icons: &icons::ArtworkCache,
    children_by_path: &HashMap<PathBuf, Vec<(String, PathBuf)>>,
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
        icons: &icons::ArtworkCache,
        children_by_path: &HashMap<PathBuf, Vec<(String, PathBuf)>>,
        palette: ThemePalette,
    ) {
        let is_expanded = expanded.contains(&path);
        let is_active = current == path;
        let toggle_message = FileMessage::ToggleFolder(path.clone());
        let open_message = FileMessage::OpenFolder(path.clone());
        let is_hovered =
            hovered_message == Some(&toggle_message) || hovered_message == Some(&open_message);
        let mut row = SidebarFolder::new(
            toggle_message,
            open_message,
            label.clone(),
            is_expanded,
            if is_active {
                palette.text
            } else {
                palette.muted
            },
        )
        .accessibility_labels((format!("Toggle {label}"), format!("Open {label}")))
        .focus_borders((palette.accent, palette.complement))
        .indent(depth)
        .background(if is_active {
            palette.accent_soft
        } else if is_hovered {
            palette.surface_hover
        } else {
            palette.panel
        });
        if let Some((id, image)) = icons.get(&path) {
            row = row.artwork(*id, image.clone(), u64::from(*id));
        }
        rows.push(AnyView::new(row));
        if !is_expanded || depth >= 6 {
            return;
        }
        let Some(children) = children_by_path.get(&path) else {
            return;
        };
        for (label, child) in children {
            append_folder(
                rows,
                label.clone(),
                child.clone(),
                depth + 1,
                expanded,
                current,
                hovered_message,
                icons,
                children_by_path,
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
            icons,
            children_by_path,
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
