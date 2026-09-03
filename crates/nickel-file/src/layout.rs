use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use nickel_core::theme::ThemePalette;
use nickel_ui::{
    AnyView, Collection, CollectionPresentation, CollectionState, Component, ComponentBuilderExt,
    Insets, LinearGradient, NavigationScope, Point, Rect, SemanticNodeSnapshot, SemanticRole,
    VerticalScroll, VirtualWindow, ui,
};

use super::FileMessage;
use crate::{
    DirectoryBrowser, FileEntry,
    app::{FileApp, SIDEBAR_RESIZE_WIDTH, TOOLBAR_HEIGHT},
    platform::places,
};

pub(crate) fn build_view(
    app: &FileApp,
    _width: f32,
    height: f32,
    palette: ThemePalette,
    light_mode: bool,
) -> AnyView<FileMessage> {
    let content_height = (height - TOOLBAR_HEIGHT - 30.0).max(0.0);
    let tabs = (0..app.tabs.len()).map(|index| {
        let active = index == app.active_tab;
        let tab = if active {
            file_tab_element(
                index,
                &app.browser,
                app.tab_icon.as_ref(),
                true,
                palette,
                light_mode,
            )
        } else {
            let tab = app
                .inactive_tab(index)
                .expect("inactive tab slot must contain state");
            file_tab_element(
                index,
                &tab.browser,
                tab.tab_icon.as_ref(),
                false,
                palette,
                light_mode,
            )
        };
        tab.into_element()
    });
    let breadcrumbs = breadcrumb_paths(app.browser.current());
    let tab_strip = ui! {
        <Container id={"tab-strip"} height={32.0} background={palette.panel} padding={Insets {
            top: 5.0, right: 10.0, bottom: 0.0, left: 12.0,
        }}>
            <Row gap={3.0} children={tabs}>
                <Container width={28.0} on_press={FileMessage::NewTab}
                    focus_border={palette.accent} controller_focus_border={palette.complement} padding={Insets {
                    top: 1.0, right: 4.0, bottom: 0.0, left: 4.0,
                }} accessibility_label={"New tab"}>
                    <Text width={20.0} scale={1.25} color={palette.muted}>{"+"}</Text>
                </Container>
            </Row>
        </Container>
    };
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
    let navigation = ui! {
        <Container id={"navigation-toolbar"} height={46.0} background={palette.surface} padding={Insets {
            top: 6.0, right: 12.0, bottom: 6.0, left: 10.0,
        }}>
            <Row gap={4.0}>
                <Button on_press={FileMessage::Back} width={34.0} height={34.0}
                    focus_border={palette.accent} controller_focus_border={palette.complement}
                    color={if app.browser.can_go_back() { palette.text } else { palette.muted }} accessibility_label={"Back"}>
                    {"←"}
                </Button>
                <Button on_press={FileMessage::Forward} width={34.0} height={34.0}
                    focus_border={palette.accent} controller_focus_border={palette.complement}
                    color={if app.browser.can_go_forward() { palette.text } else { palette.muted }} accessibility_label={"Forward"}>
                    {"→"}
                </Button>
                <Button on_press={FileMessage::Up} width={34.0} height={34.0} color={palette.text}
                    focus_border={palette.accent} controller_focus_border={palette.complement} accessibility_label={"Up one folder"}>{"↑"}</Button>
                <Container grow={1.0} background={palette.background} padding={Insets {
                    top: 5.0, right: 12.0, bottom: 4.0, left: 10.0,
                }}>
                    {breadcrumb_row}
                </Container>
                <Button on_press={FileMessage::Refresh} width={34.0} height={34.0} color={palette.text}
                    focus_border={palette.accent} controller_focus_border={palette.complement} accessibility_label={"Refresh"}>{"↻"}</Button>
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
    let folder_rows = sidebar_folder_elements(
        &places(),
        &app.expanded_folders,
        app.browser.current(),
        None,
        palette,
    );
    let sidebar = ui! {
        <Sidebar width={app.sidebar_width} background={palette.panel} padding={Insets {
            top: 14.0, right: 10.0, bottom: 12.0, left: 10.0,
        }} gap={3.0}>
            <Text height={34.0} scale={1.55} color={palette.text}>{"Nickel File"}</Text>
            <HorizontalRule color={palette.muted} spacing_pair={(5.0, 8.0)} />
            <SidebarSection title={"Places"} color={palette.muted}>{folder_rows.into_iter()}</SidebarSection>
        </Sidebar>
    };
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
        let viewport_width = (_width - app.sidebar_width - SIDEBAR_RESIZE_WIDTH - 32.0).max(1.0);
        let viewport_height = (height - TOOLBAR_HEIGHT - 30.0 - 28.0 - 1.0).max(1.0);
        let grid = Collection::try_new(
            CollectionState::Ready(tile_rows),
            |(_, entry, _, _)| entry.path.to_string_lossy().into_owned(),
            move |(index, entry, selected, icon)| {
                file_tile(
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
        });
        let scroll = VerticalScroll::new(
            FileMessage::FileScroll(app.file_scroll_offset),
            app.file_scroll_offset,
        )
        .on_scroll(FileMessage::FileScroll)
        .controlled(true)
        .height(viewport_height)
        .id("file-list")
        .child(grid);
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
    let footer_text = if app.status.is_empty() {
        format!(
            "{} item{}",
            app.browser.entries().len(),
            if app.browser.entries().len() == 1 {
                ""
            } else {
                "s"
            }
        )
    } else {
        app.status.clone()
    };
    let footer = ui! {
        <Container id={"file-footer"} height={30.0} shrink={0.0} background={palette.surface} padding={Insets {
            top: 7.0, right: 14.0, bottom: 5.0, left: 14.0,
        }}>
            <Text scale={1.0} color={palette.muted}>{footer_text}</Text>
        </Container>
    };
    let resize_handle = ui! {
        <Container id={"sidebar-resize"} width={SIDEBAR_RESIZE_WIDTH} shrink={0.0}
            background={if app.is_resizing_sidebar() { palette.accent } else { palette.surface_hover }}
            on_press={FileMessage::ResizeSidebar} focus_border={palette.accent}
            controller_focus_border={palette.complement} accessibility_label={"Resize sidebar"} />
    };
    let sidebar_pane_width = app.sidebar_width + SIDEBAR_RESIZE_WIDTH;
    let content = ui! {
        <Container id={"file-layout"} height={content_height} shrink={0.0} accessibility_label={"Files"}>
            <Row grow={1.0}><Container id={"sidebar-pane"} width={sidebar_pane_width} shrink={0.0}
                navigation_scope={NavigationScope::pane(false)} navigation_scope_highlight={palette.complement}>
                <Row width={sidebar_pane_width} shrink={0.0}>{sidebar}{resize_handle}</Row>
            </Container>
                <Container id={"files-pane"} grow={1.0} min_width={0.0}
                navigation_scope={NavigationScope::pane(true)} navigation_scope_highlight={palette.complement}>{files}</Container></Row>
        </Container>
    };
    let root = ui! {
        <Column height={height} background={palette.background}>{toolbar}{content}{footer}</Column>
    };
    AnyView::new(root)
}

pub(crate) fn visible_file_range(app: &FileApp, width: f32, height: f32) -> std::ops::Range<usize> {
    let count = app.browser.entries().len();
    let viewport_width = (width - app.sidebar_width - SIDEBAR_RESIZE_WIDTH - 32.0).max(1.0);
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

pub(crate) fn file_tab_element(
    index: usize,
    browser: &DirectoryBrowser,
    icon: Option<&(u16, Arc<image::RgbaImage>)>,
    active: bool,
    palette: ThemePalette,
    light_mode: bool,
) -> impl Component<FileMessage> + use<> {
    let label = browser
        .current()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| browser.current().display().to_string());
    ui! {
        <Container width={170.0} background={if active {
            if light_mode {
                0xffffff
            } else {
                palette.background
            }
        } else if light_mode {
            mix_rgb(palette.panel, 0xffffff)
        } else {
            palette.panel
        }} top_corner_radius={5.0} accessibility_label={format!("Tab {label}")}>
            <Column>
                <Row height={25.0} gap={6.0} padding={Insets {
                    top: 4.0, right: 4.0, bottom: 3.0, left: 9.0,
                }}>
                    <Container width={125.0} height={25.0} on_press={FileMessage::SwitchTab(index)}
                        focus_border={palette.accent} controller_focus_border={palette.complement}
                        accessibility_label={format!("Tab {label}")}><Row gap={6.0}>
                        {if let Some((id, image)) = icon {
                            ui! { <Image asset_id={*id} image={image.clone()} generation={u64::from(*id)} width={16.0} height={16.0} /> }
                        } else {
                            ui! { <Container width={16.0} /> }
                        }}
                        <Text width={97.0} height={18.0} scale={1.05}
                            color={if active { palette.text } else { palette.muted }}>{label.clone()}</Text>
                        </Row></Container>
                    <Container width={20.0} on_press={FileMessage::CloseTab(index)}
                        focus_border={palette.accent} controller_focus_border={palette.complement} accessibility_label={format!("Close {label}")}>
                        <Text width={20.0} color={palette.muted}>{"×"}</Text>
                    </Container>
                </Row>
                <Container height={2.0} background={if active { palette.accent } else { palette.panel }} />
            </Column>
        </Container>
    }
}

pub(crate) fn mix_rgb(left: u32, right: u32) -> u32 {
    let channel = |shift: u32| {
        let left = (left >> shift) & 0xff_u32;
        let right = (right >> shift) & 0xff_u32;
        (left + right) / 2
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
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

pub(crate) fn file_tile(
    index: usize,
    entry: &FileEntry,
    selected: bool,
    icon: Option<(u16, Arc<image::RgbaImage>)>,
    palette: ThemePalette,
    icon_size: f32,
    light_mode: bool,
) -> impl Component<FileMessage> + use<> {
    let (icon_id, icon_image) = icon.unwrap_or_else(|| {
        (
            0,
            Arc::new(image::RgbaImage::from_pixel(
                1,
                1,
                image::Rgba([0, 0, 0, 0]),
            )),
        )
    });
    ui! {
        <FileGridItem on_press={FileMessage::Entry(index)} label={entry.display_name()}
            asset_id={icon_id} image={icon_image} generation={u64::from(icon_id)}
            borderless_palette={(if selected {
                palette.accent_soft
            } else if light_mode {
                0xffffff
            } else {
                palette.background
            }, palette.text)} icon_size={icon_size} focus_border={palette.accent}
            controller_focus_border={palette.complement} id={format!("file-entry-{index}")}
            context_message={FileMessage::ContextEntry(index)} semantic_role={nickel_ui::SemanticRole::Button}
            accessibility_label={entry.display_name()} />
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
