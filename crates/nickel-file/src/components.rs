use std::{path::PathBuf, sync::Arc};

use nickel_core::theme::ThemePalette;
use nickel_ui::{
    AnyView, Collection, CollectionPresentation, CollectionState, Component, ComponentBuilderExt,
    ImageFit, Insets, NavigationScope, SemanticRole, TextField, VerticalScroll, ui,
};

use crate::{
    DirectoryBrowser, EntrySortKey, FileEntry, SortDirection,
    app::{
        DetailsColumn, DetailsColumnWidths, FileApp, FileMessage, FileViewMode, MAX_TILE_WIDTH,
        MIN_TILE_WIDTH,
    },
};

pub(crate) fn properties_dialog(
    app: &FileApp,
    properties: &crate::properties::EntryProperties,
    palette: ThemePalette,
) -> AnyView<FileMessage> {
    let localizer = &app.localizer;
    let size = properties
        .logical_size
        .map(|bytes| localizer.bytes(bytes))
        .unwrap_or_else(|| "Not calculated".into());
    let allocated = properties
        .allocated_size
        .map(|bytes| localizer.bytes(bytes))
        .unwrap_or_else(|| "Unavailable".into());
    let modified = properties
        .modified
        .map(format_modified)
        .unwrap_or_else(|| "Unavailable".into());
    let accessed = properties
        .accessed
        .map(format_modified)
        .unwrap_or_else(|| "Unavailable".into());
    let created = properties
        .created
        .map(format_modified)
        .unwrap_or_else(|| "Unavailable".into());
    let stale = properties.is_stale();
    let association = app.properties_association.as_ref();
    let handlers = association.map(|snapshot| snapshot.handlers.iter().enumerate().map(|(index, handler)| {
        let selected = app.properties_handler == Some(index);
        ui! {
            <Button id={format!("properties-handler-{index}")} on_press={FileMessage::PropertiesSelectHandler(index)}
                width={410.0} height={30.0} color={if selected { palette.accent } else { palette.text }}
                accessibility_label={format!("{} for {}; {}", handler.name, association.unwrap().target.platform_key(), handler.source)}>
                {handler.name.clone()}
            </Button>
        }
    }).collect::<Vec<_>>()).unwrap_or_default();
    let row = |label: &'static str, value: String| {
        ui! {
            <Row height={28.0} gap={12.0}>
                <Text width={110.0} color={palette.muted}>{label}</Text>
                <Text color={palette.text} wrap={true} max_lines={2} grow={1.0}>{value}</Text>
            </Row>
        }
    };
    let content = ui! {
        <Column gap={8.0}>
            <Text height={28.0} scale={1.35} color={palette.text}>{format!("{} Properties", properties.name)}</Text>
            {if app.status.is_empty() { ui! { <></> } } else { ui! { <Text height={24.0} color={palette.accent}>{app.status.clone()}</Text> } }}
            {if stale { ui! { <Text height={22.0} color={palette.accent}>{"This item changed or is no longer available."}</Text> } } else { ui! { <></> } }}
            {row("Kind", properties.kind.clone())}
            {row("Location", properties.path.parent().unwrap_or(&properties.path).display().to_string())}
            {row("Size", size)}
            {row("On disk", allocated)}
            {row("Modified", modified)}
            {row("Accessed", accessed)}
            {row("Created", created)}
            {row("Owner", properties.owner.clone().unwrap_or_else(|| "Unavailable".into()))}
            {row("Permissions", properties.permissions.clone())}
            {if let Some(target) = properties.symlink_target.as_ref() { row("Link target", target.display().to_string()) } else { ui! { <></> } }}
            <Row height={32.0} gap={8.0}>
                <Button on_press={FileMessage::PropertiesToggleReadonly} enabled={!stale} width={145.0} height={30.0} color={palette.text}>{if app.properties_edits.is_some_and(|edits| edits.readonly) { "Read-only: On" } else { "Read-only: Off" }}</Button>
                <Button on_press={FileMessage::PropertiesToggleHidden} enabled={!stale} width={145.0} height={30.0} color={palette.text}>{if app.properties_edits.is_some_and(|edits| edits.hidden) { "Hidden: On" } else { "Hidden: Off" }}</Button>
            </Row>
            {if properties.kind == "Folder" { ui! {
                <Row height={34.0} gap={8.0}>
                    <Button on_press={if app.properties_size_job.is_some() { FileMessage::PropertiesCancelSize } else { FileMessage::PropertiesCalculateSize }} width={150.0} height={32.0} color={palette.text}>
                        {if app.properties_size_job.is_some() { "Cancel calculation" } else { "Calculate contents" }}
                    </Button>
                    <Text color={palette.muted} grow={1.0}>{app.properties_size_progress.clone().unwrap_or_default()}</Text>
                </Row>
            } } else { ui! { <></> } }}
            {if let Some(snapshot) = association { ui! {
                <Column gap={5.0}>
                    <Text height={22.0} color={palette.text}>{format!("Open with — {}", snapshot.target.platform_key())}</Text>
                    <Column gap={4.0} children={handlers} />
                    <Row height={34.0} gap={8.0}>
                        <Button on_press={FileMessage::PropertiesOpenOnce} enabled={app.properties_handler.is_some()} width={120.0} height={32.0} color={palette.text}>{"Open once"}</Button>
                        <Button on_press={FileMessage::PropertiesRequestDefault} enabled={app.properties_handler.is_some() && !stale} width={120.0} height={32.0} color={palette.text}>{"Make default"}</Button>
                    </Row>
                    {if app.properties_confirm_default { ui! {
                        <Row height={36.0} gap={8.0}>
                            <Text color={palette.text} grow={1.0}>{format!("Change the default for {}?", snapshot.target.platform_key())}</Text>
                            <Button on_press={FileMessage::PropertiesConfirmDefault} width={90.0} height={32.0} color={palette.text}>{"Confirm"}</Button>
                        </Row>
                    } } else { ui! { <></> } }}
                </Column>
            } } else { ui! { <Text height={22.0} color={palette.muted}>{if app.properties_association_status.is_empty() { "No supported file association is available.".into() } else { app.properties_association_status.clone() }}</Text> } }}
            {if app.properties_confirm_close { ui! {
                <Row height={36.0} gap={8.0}>
                    <Text color={palette.text} grow={1.0}>{"Discard unsaved property changes?"}</Text>
                    <Button on_press={FileMessage::DiscardProperties} width={90.0} height={32.0} color={palette.text}>{"Discard"}</Button>
                </Row>
            } } else { ui! { <></> } }}
            <Row height={38.0}>
                <Container grow={1.0} />
                <Button id={"file-properties-apply"} on_press={FileMessage::PropertiesApply}
                    enabled={!stale} width={90.0} height={34.0} color={palette.text}>{"Apply"}</Button>
                <Button id={"file-properties-ok"} on_press={FileMessage::PropertiesOk}
                    enabled={!stale} width={90.0} height={34.0} color={palette.text}>{"OK"}</Button>
                <Button id={"file-properties-close"} on_press={FileMessage::CloseProperties}
                    width={90.0} height={34.0} color={palette.text}>{"Cancel"}</Button>
            </Row>
        </Column>
    };
    let scroll = VerticalScroll::new(
        FileMessage::PropertiesScroll(app.properties_scroll),
        app.properties_scroll,
    )
    .on_scroll(FileMessage::PropertiesScroll)
    .controlled(true)
    .height(576.0)
    .child(content);
    AnyView::new(ui! {
        <Container id={"file-properties-content"} semantic_role={SemanticRole::Dialog}
            accessibility_label={format!("Properties for {}", properties.name)}
            background={palette.surface} padding={Insets::all(22.0)}>
            {scroll}
        </Container>
    })
}

pub(crate) fn navigation_toolbar(
    app: &FileApp,
    breadcrumbs: &[(String, PathBuf)],
    narrow: bool,
    palette: ThemePalette,
) -> AnyView<FileMessage> {
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
                        focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement} padding={Insets {
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
    AnyView::new(ui! {
        <Container id={"navigation-toolbar"} height={46.0} background={palette.surface} padding={Insets {
            top: 6.0, right: 12.0, bottom: 6.0, left: 10.0,
        }}>
            <Row gap={4.0}>
                {if narrow {
                    ui! {
                        <Button on_press={FileMessage::TogglePlaces} width={74.0} height={34.0}
                            color={if app.places_open { palette.accent } else { palette.text }}
                            focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                            accessibility_label={if app.places_open { "Close places" } else { "Open places" }}>
                            {if app.places_open { "Files" } else { "Places" }}
                        </Button>
                    }
                } else { ui! { <></> } }}
                <Button on_press={FileMessage::Back} enabled={app.browser.can_go_back() && !app.navigation_pending()} width={34.0} height={34.0}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                    color={if app.browser.can_go_back() { palette.text } else { palette.muted }} accessibility_label={"Back"}>{"←"}</Button>
                <Button on_press={FileMessage::Forward} enabled={app.browser.can_go_forward() && !app.navigation_pending()} width={34.0} height={34.0}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                    color={if app.browser.can_go_forward() { palette.text } else { palette.muted }} accessibility_label={"Forward"}>{"→"}</Button>
                <Button on_press={FileMessage::Up} enabled={app.browser.can_go_up() && !app.navigation_pending()} width={34.0} height={34.0} color={palette.text}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement} accessibility_label={"Up one folder"}>{"↑"}</Button>
                {location_control}
                <Button on_press={FileMessage::ToggleAddressEditing} width={34.0} height={34.0}
                    color={if app.address_editing { palette.accent } else { palette.text }}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                    accessibility_label={if app.address_editing { "Cancel location editing" } else { "Edit location" }}>
                    {if app.address_editing { "×" } else { "✎" }}
                </Button>
                <Button on_press={FileMessage::Refresh} enabled={!app.navigation_pending()} width={34.0} height={34.0} color={palette.text}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement} accessibility_label={"Refresh"}>{"↻"}</Button>
                <Button on_press={FileMessage::SetViewMode(FileViewMode::Grid)} width={34.0} height={34.0}
                    color={if app.view_mode == FileViewMode::Grid { palette.accent } else { palette.text }}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement} accessibility_label={"Grid view"}>{"▦"}</Button>
                <Button on_press={FileMessage::SetViewMode(FileViewMode::Details)} width={34.0} height={34.0}
                    color={if app.view_mode == FileViewMode::Details { palette.accent } else { palette.text }}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement} accessibility_label={"Details view"}>{"☷"}</Button>
                <Button on_press={FileMessage::ToggleCommandSurface} width={42.0} height={34.0}
                    color={if app.command_surface_open { palette.accent } else { palette.text }}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                    accessibility_label={"Open commands"}>{"⌘"}</Button>
            </Row>
        </Container>
    })
}

fn address_changed(address: String) -> FileMessage {
    FileMessage::AddressChanged(address)
}

pub(crate) fn details_header(
    sort_key: EntrySortKey,
    sort_direction: SortDirection,
    widths: DetailsColumnWidths,
    palette: ThemePalette,
) -> AnyView<FileMessage> {
    let sort_label = |label: &str, key: EntrySortKey| {
        if sort_key != key {
            label.to_owned()
        } else {
            format!(
                "{label} {}",
                match sort_direction {
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
    AnyView::new(ui! {
        <Container id={"details-header"} height={32.0} background={palette.surface} padding={Insets {
            top: 8.0, right: 10.0, bottom: 6.0, left: 10.0,
        }}>
            <Row gap={12.0}>
                <Text width={32.0} color={palette.muted}>{""}</Text>
                <Container grow={1.0} min_width={120.0} on_press={FileMessage::SortBy(EntrySortKey::Name)}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                    semantic_role={SemanticRole::Button} accessibility_label={"Sort by name"}>
                    <Text color={palette.muted}>{name_sort}</Text>
                </Container>
                <Container width={widths.type_width} on_press={FileMessage::SortBy(EntrySortKey::Type)}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                    semantic_role={SemanticRole::Button} accessibility_label={"Sort by type"}>
                    <Text color={palette.muted}>{type_sort}</Text>
                </Container>
                <Container id={"resize-details-type"} width={5.0} on_press={FileMessage::ResizeDetailsColumn(DetailsColumn::Type)}
                    background={palette.surface_hover} focus_background_tint={palette.accent} accessibility_label={"Resize type column"} />
                <Container width={widths.modified_width} on_press={FileMessage::SortBy(EntrySortKey::Modified)}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                    semantic_role={SemanticRole::Button} accessibility_label={"Sort by modified time"}>
                    <Text color={palette.muted}>{modified_sort}</Text>
                </Container>
                <Container id={"resize-details-modified"} width={5.0} on_press={FileMessage::ResizeDetailsColumn(DetailsColumn::Modified)}
                    background={palette.surface_hover} focus_background_tint={palette.accent} accessibility_label={"Resize modified column"} />
                <Container width={widths.size_width} on_press={FileMessage::SortBy(EntrySortKey::Size)}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                    semantic_role={SemanticRole::Button} accessibility_label={"Sort by size"}>
                    <Text color={palette.muted}>{size_sort}</Text>
                </Container>
                <Container id={"resize-details-size"} width={5.0} on_press={FileMessage::ResizeDetailsColumn(DetailsColumn::Size)}
                    background={palette.surface_hover} focus_background_tint={palette.accent} accessibility_label={"Resize size column"} />
            </Row>
        </Container>
    })
}

pub(crate) fn command_surface(
    app: &FileApp,
    available_height: f32,
    palette: ThemePalette,
) -> AnyView<FileMessage> {
    let query = app.command_query.trim().to_ascii_lowercase();
    let command_label = |id| app.localizer.text(id);
    let commands = [
        (
            command_label("file-command-open"),
            "activate enter",
            app.selected.is_some() && !app.navigation_pending(),
            FileMessage::ContextOpen,
        ),
        (
            command_label("file-command-open-new-tab"),
            "activate background tab",
            app.selected_is_container() && !app.navigation_pending(),
            FileMessage::ContextOpenNewTab,
        ),
        (
            command_label("file-command-back"),
            "previous navigation",
            app.browser.can_go_back() && !app.navigation_pending(),
            FileMessage::Back,
        ),
        (
            command_label("file-command-forward"),
            "next navigation",
            app.browser.can_go_forward() && !app.navigation_pending(),
            FileMessage::Forward,
        ),
        (
            command_label("file-command-up"),
            "parent folder",
            app.browser.can_go_up() && !app.navigation_pending(),
            FileMessage::Up,
        ),
        (
            command_label("file-command-refresh"),
            "reload f5",
            !app.navigation_pending(),
            FileMessage::Refresh,
        ),
        (
            command_label("file-command-new-tab"),
            "create tab ctrl t",
            true,
            FileMessage::NewTab,
        ),
        (
            command_label("file-command-close-tab"),
            "dismiss tab ctrl w",
            true,
            FileMessage::CloseTab(app.active_tab),
        ),
        (
            command_label("file-command-grid-view"),
            "icons thumbnails",
            true,
            FileMessage::SetViewMode(FileViewMode::Grid),
        ),
        (
            command_label("file-command-details-view"),
            "list columns",
            true,
            FileMessage::SetViewMode(FileViewMode::Details),
        ),
        (
            command_label("file-command-increase-tile-size"),
            "grid zoom larger ctrl plus",
            app.view_mode == FileViewMode::Grid && app.tile_width < MAX_TILE_WIDTH,
            FileMessage::AdjustTileWidth(1),
        ),
        (
            command_label("file-command-decrease-tile-size"),
            "grid zoom smaller ctrl minus",
            app.view_mode == FileViewMode::Grid && app.tile_width > MIN_TILE_WIDTH,
            FileMessage::AdjustTileWidth(-1),
        ),
        (
            command_label("file-command-select-all"),
            "selection ctrl a",
            !app.browser.entries().is_empty(),
            FileMessage::ContextSelectAll,
        ),
        (
            command_label("file-command-sort-name"),
            "order filename",
            true,
            FileMessage::SortBy(EntrySortKey::Name),
        ),
        (
            command_label("file-command-sort-type"),
            "order extension",
            true,
            FileMessage::SortBy(EntrySortKey::Type),
        ),
        (
            command_label("file-command-sort-modified"),
            "order date time",
            true,
            FileMessage::SortBy(EntrySortKey::Modified),
        ),
        (
            command_label("file-command-sort-size"),
            "order bytes",
            true,
            FileMessage::SortBy(EntrySortKey::Size),
        ),
        (
            if app.browser.show_hidden() {
                command_label("file-command-hide-hidden")
            } else {
                command_label("file-command-show-hidden")
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
            (index, label, aliases, enabled, message)
        })
        .collect::<Vec<_>>();
    let results_height = (available_height - 132.0).max(1.0);
    let results = if rows.is_empty() {
        AnyView::new(ui! {
            <Container padding={Insets::all(16.0)}><Text color={palette.muted}>{"No matching commands."}</Text></Container>
        })
    } else {
        AnyView::new(
            Collection::try_new(
                CollectionState::Ready(rows),
                |(index, _, _, _, _)| index.to_string(),
                move |(index, label, aliases, enabled, message)| {
                    let accessibility_label = label.clone();
                    AnyView::new(ui! {
                        <Container id={format!("file-command-{index}")} height={42.0}
                            on_press={message} enabled={enabled} background={palette.surface}
                            hover_background={palette.surface_hover} pressed_background={palette.accent_soft}
                            focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                            padding={Insets { top: 9.0, right: 12.0, bottom: 7.0, left: 12.0 }}
                            semantic_role={SemanticRole::Button} accessibility_label={accessibility_label}>
                            <Row gap={12.0}>
                                <Text width={180.0} color={if enabled { palette.text } else { palette.muted }}>{label}</Text>
                                <Text color={palette.muted} scale={0.9}>{aliases}</Text>
                            </Row>
                        </Container>
                    })
                },
            )
            .expect("command identifiers are unique")
            .id("file-command-collection")
            .gap(2.0)
            .navigation_scope(NavigationScope::group())
            .presentation(CollectionPresentation::VirtualList {
                item_height: 42.0,
                offset: app.command_scroll_offset,
                viewport_height: results_height,
                overscan: 0.0,
            }),
        )
    };
    let results = VerticalScroll::new(
        FileMessage::CommandScroll(app.command_scroll_offset),
        app.command_scroll_offset,
    )
    .on_scroll(FileMessage::CommandScroll)
    .controlled(true)
    .height(results_height)
    .id("file-command-results")
    .child(results);
    AnyView::new(ui! {
        <Container id={"file-command-surface"} grow={1.0} background={palette.background}
            padding={Insets { top: 24.0, right: 32.0, bottom: 24.0, left: 32.0 }}>
            <Column gap={10.0}>
                <Container height={22.0} shrink={0.0}>
                    <Text color={palette.text} scale={1.35}>{"Commands"}</Text>
                </Container>
                <Container height={40.0} shrink={0.0} background={palette.surface} border={(palette.accent, 1.0)}
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

fn command_query_message(query: String) -> FileMessage {
    FileMessage::CommandQueryChanged(query)
}

pub(crate) fn status_text(app: &FileApp) -> String {
    if !app.status.is_empty() {
        return app.status.clone();
    }
    let total = app.browser.entries().len();
    let total_label = format!("{total} item{}", if total == 1 { "" } else { "s" });
    let location = app
        .browser
        .current()
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| app.browser.current().display().to_string());
    let selected = app.selected_entries.len();
    if selected == 0 {
        format!("{total_label} · {location}")
    } else {
        let summary = app.selection_summary().visible_label(&app.localizer);
        format!("{summary} · {total_label} · {location}")
    }
}

pub(crate) fn status_accessibility_text(app: &FileApp) -> String {
    if !app.status.is_empty() || app.selected_entries.is_empty() {
        return status_text(app);
    }
    let summary = app.selection_summary().accessible_label(&app.localizer);
    let total = app.browser.entries().len();
    format!("{summary}; {total} total items")
}

pub(crate) fn tab_strip(
    app: &FileApp,
    palette: ThemePalette,
    light_mode: bool,
) -> AnyView<FileMessage> {
    let tabs = (0..app.tabs.len()).map(|index| {
        let active = index == app.active_tab;
        let tab = if active {
            tab(
                index,
                &app.browser,
                app.tab_icon.as_ref(),
                true,
                palette,
                light_mode,
            )
        } else {
            let state = app
                .inactive_tab(index)
                .expect("inactive tab slot must contain state");
            tab(
                index,
                &state.browser,
                state.tab_icon.as_ref(),
                false,
                palette,
                light_mode,
            )
        };
        tab.into_element()
    });
    AnyView::new(ui! {
        <Container id={"tab-strip"} height={32.0} background={palette.panel} padding={Insets {
            top: 5.0, right: 10.0, bottom: 0.0, left: 12.0,
        }}>
            <Row gap={3.0} children={tabs}>
                <Container width={28.0} on_press={FileMessage::NewTab}
                    focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement} padding={Insets {
                    top: 1.0, right: 4.0, bottom: 0.0, left: 4.0,
                }} accessibility_label={"New tab"}>
                    <Text width={20.0} scale={1.25} color={palette.muted}>{"+"}</Text>
                </Container>
            </Row>
        </Container>
    })
}

pub(crate) fn places_sidebar(
    width: f32,
    groups: Vec<AnyView<FileMessage>>,
    palette: ThemePalette,
) -> AnyView<FileMessage> {
    AnyView::new(ui! {
        <Sidebar width={width} background={palette.panel} padding={Insets {
            top: 14.0, right: 10.0, bottom: 12.0, left: 10.0,
        }} gap={3.0}>
            <Text height={34.0} scale={1.55} color={palette.text}>{"Nickel File"}</Text>
            <HorizontalRule color={palette.muted} spacing_pair={(5.0, 8.0)} />
            <Column gap={4.0} children={groups} />
        </Sidebar>
    })
}

pub(crate) fn location_group(
    id: &str,
    title: &str,
    rows: Vec<AnyView<FileMessage>>,
    collapsed: bool,
    palette: ThemePalette,
) -> AnyView<FileMessage> {
    let group_id = id.to_owned();
    AnyView::new(ui! {
        <Column id={format!("location-group-{id}")} gap={2.0}>
            <Container height={28.0} on_press={FileMessage::ToggleLocationGroup(group_id)}
                focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                accessibility_label={format!("{} {title}", if collapsed { "Expand" } else { "Collapse" })}
                padding={Insets { top: 6.0, right: 4.0, bottom: 4.0, left: 4.0 }}>
                <Row gap={7.0}>
                    <Text width={12.0} color={palette.muted}>{if collapsed { "›" } else { "⌄" }}</Text>
                    <Text color={palette.muted}>{title}</Text>
                </Row>
            </Container>
            {if collapsed { ui! { <></> } } else { ui! { <Column gap={2.0} children={rows} /> } }}
        </Column>
    })
}

pub(crate) fn status_bar(
    text: String,
    accessibility_text: String,
    palette: ThemePalette,
) -> AnyView<FileMessage> {
    AnyView::new(ui! {
        <Container id={"file-footer"} accessibility_label={accessibility_text} height={30.0} shrink={0.0} background={palette.surface} padding={Insets {
            top: 7.0, right: 14.0, bottom: 5.0, left: 14.0,
        }}>
            <Text scale={1.0} color={palette.muted}>{text}</Text>
        </Container>
    })
}

pub(crate) fn tab(
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
            palette.background
        } else if light_mode {
            mix_rgb(palette.panel, palette.background)
        } else {
            palette.panel
        }} top_corner_radius={5.0} accessibility_label={format!("Tab {label}")}>
            <Column>
                <Row height={25.0} gap={6.0} padding={Insets {
                    top: 4.0, right: 4.0, bottom: 3.0, left: 9.0,
                }}>
                    <Container width={125.0} height={25.0} on_press={FileMessage::SwitchTab(index)}
                        focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
                        accessibility_label={format!("Tab {label}")}><Row gap={6.0}>
                        {if let Some((id, image)) = icon {
                            ui! { <Image asset_id={*id} image={image.clone()} generation={u64::from(*id)} fit={ImageFit::Contain} width={16.0} height={16.0} /> }
                        } else {
                            ui! { <Container width={16.0} /> }
                        }}
                        <Text width={97.0} height={18.0} scale={1.05}
                            color={if active { palette.text } else { palette.muted }}>{label.clone()}</Text>
                        </Row></Container>
                    <Container width={20.0} on_press={FileMessage::CloseTab(index)}
                        focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement} accessibility_label={format!("Close {label}")}>
                        <Text width={20.0} color={palette.muted}>{"×"}</Text>
                    </Container>
                </Row>
                <Container height={2.0} background={if active { palette.accent } else { palette.panel }} />
            </Column>
        </Container>
    }
}

pub(crate) fn grid_item(
    index: usize,
    entry: &FileEntry,
    selected: bool,
    icon: Option<(u16, Arc<image::RgbaImage>)>,
    palette: ThemePalette,
    icon_size: f32,
    _light_mode: bool,
) -> impl Component<FileMessage> + use<> {
    let (icon_id, icon_image) = icon.unwrap_or_else(empty_artwork);
    ui! {
        <FileGridItem on_press={FileMessage::Entry(index)} label={entry.display_name()}
            asset_id={icon_id} image={icon_image} generation={u64::from(icon_id)}
            borderless_palette={(if selected { palette.accent_soft } else { palette.background }, palette.text)}
            icon_size={icon_size} focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}
            id={format!("file-entry-{index}")} context_message={FileMessage::ContextEntry(index)}
            semantic_role={SemanticRole::Button} accessibility_label={entry.display_name()} />
    }
}

pub(crate) fn details_row(
    index: usize,
    entry: &FileEntry,
    selected: bool,
    icon: Option<(u16, Arc<image::RgbaImage>)>,
    palette: ThemePalette,
    _light_mode: bool,
    widths: DetailsColumnWidths,
) -> impl Component<FileMessage> + use<> {
    let (icon_id, icon_image) = icon.unwrap_or_else(empty_artwork);
    let kind = if entry.is_directory {
        "File folder".to_owned()
    } else {
        entry
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| format!("{} file", extension.to_ascii_uppercase()))
            .unwrap_or_else(|| "File".to_owned())
    };
    let (size, modified) = if entry.is_directory {
        (String::new(), String::new())
    } else {
        (
            entry.size.map(format_file_size).unwrap_or_default(),
            entry.modified.map(format_modified).unwrap_or_default(),
        )
    };
    ui! {
        <Container id={format!("file-entry-{index}")} height={58.0}
            background={if selected { palette.accent_soft } else { palette.background }}
            hover_background={palette.surface_hover} pressed_background={palette.accent_soft}
            padding={Insets { top: 7.0, right: 10.0, bottom: 7.0, left: 10.0 }}
            on_press={FileMessage::Entry(index)} context_message={FileMessage::ContextEntry(index)}
            semantic_role={SemanticRole::Button} accessibility_label={entry.display_name()}
            focus_background_tint={palette.accent} controller_focus_background_tint={palette.complement}>
            <Row gap={12.0}>
                <Image asset_id={icon_id} image={icon_image} generation={u64::from(icon_id)} fit={ImageFit::Contain} width={28.0} height={28.0} />
                <Container id={format!("details-name-{index}")} grow={1.0} min_width={120.0} height={44.0}>
                    <Text color={palette.text} wrap={true} max_lines={2} ellipsis={true} line_height={18.0}>{entry.display_name()}</Text>
                </Container>
                <Text id={format!("details-type-{index}")} width={widths.type_width} color={palette.muted}>{kind}</Text>
                <Container width={5.0} />
                <Text id={format!("details-modified-{index}")} width={widths.modified_width} color={palette.muted}>{modified}</Text>
                <Container width={5.0} />
                <Text id={format!("details-size-{index}")} width={widths.size_width} color={palette.muted}>{size}</Text>
                <Container width={5.0} />
            </Row>
        </Container>
    }
}

fn empty_artwork() -> (u16, Arc<image::RgbaImage>) {
    (
        0,
        Arc::new(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([0, 0, 0, 0]),
        )),
    )
}

fn mix_rgb(left: u32, right: u32) -> u32 {
    let channel = |shift: u32| (((left >> shift) & 0xff) + ((right >> shift) & 0xff)) / 2;
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn format_file_size(bytes: u64) -> String {
    if bytes < 1_024 {
        format!("{bytes} B")
    } else if bytes < 1_048_576 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else if bytes < 1_073_741_824 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    }
}

pub(crate) fn format_modified(time: std::time::SystemTime) -> String {
    let Ok(duration) = time.duration_since(std::time::UNIX_EPOCH) else {
        return String::new();
    };
    let seconds = duration.as_secs();
    let (year, month, day) = civil_date((seconds / 86_400) as i64);
    let seconds_of_day = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds_of_day / 3_600,
        seconds_of_day % 3_600 / 60
    )
}

fn civil_date(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}
