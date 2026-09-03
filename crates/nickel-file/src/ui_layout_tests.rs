use super::*;
use crate::{
    components::{format_modified, status_text as file_status_text},
    layout::{collapse_breadcrumbs, entries_in_selection},
};
use nickel_ui::{ActionKind, Rect, SemanticRole, UiHost};
use nickel_ui_testkit::{Fixture, FocusDirection, Scenario, ScenarioBudget, Selector};
use sha2::{Digest, Sha256};

const MINIMUM_SELECTION_TEXT_CONTRAST: f32 = 4.5;

fn relative_luminance(color: u32) -> f32 {
    let channel = |shift: u32| {
        let value = ((color >> shift) & 0xff_u32) as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

fn bounds_overlap(left: Rect, right: Rect) -> bool {
    left.origin.x < right.origin.x + right.size.width
        && left.origin.x + left.size.width > right.origin.x
        && left.origin.y < right.origin.y + right.size.height
        && left.origin.y + left.size.height > right.origin.y
}

fn contrast_ratio(first: u32, second: u32) -> f32 {
    let (lighter, darker) = {
        let first = relative_luminance(first);
        let second = relative_luminance(second);
        if first > second {
            (first, second)
        } else {
            (second, first)
        }
    };
    (lighter + 0.05) / (darker + 0.05)
}

fn settle_navigation(app: &mut FileApp) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if app.poll_navigation() {
            return;
        }
        std::thread::yield_now();
    }
    panic!("navigation worker did not settle");
}

#[test]
fn idle_file_host_declares_no_poll_deadline() {
    let mut app = FileApp::new(home_directory());
    app.icon_rx = None;
    assert_eq!(Application::poll_interval(&app), None);
}

#[test]
fn pending_icon_work_uses_bounded_backoff() {
    let mut app = FileApp::new(home_directory());
    let (_sender, receiver) = std::sync::mpsc::channel();
    app.icon_rx = Some(receiver);
    app.icon_poll_delay = std::time::Duration::from_millis(16);
    app.poll_icons();
    assert_eq!(app.icon_poll_delay, std::time::Duration::from_millis(32));
    for _ in 0..8 {
        app.poll_icons();
    }
    assert_eq!(app.icon_poll_delay, std::time::Duration::from_millis(250));
}

#[test]
fn stale_async_artwork_cannot_replace_current_generation() {
    let mut app = FileApp::fixture();
    let path = app.browser.entries()[0].path.clone();
    let request = icons::ArtworkRequest {
        path: &path,
        kind: icons::SemanticIconKind::Folder,
        logical_size: 96,
        scale_milli: 1_000,
        appearance: icons::ArtworkAppearance::Dark,
    };
    let key = icons::cache_key(FileIconPreference::System, &request);
    let artwork = icons::resolve_artwork(FileIconPreference::Nickel, &request);
    let (sender, receiver) = std::sync::mpsc::channel();
    sender
        .send((app.icon_generation, path.clone(), key, artwork))
        .unwrap();
    app.icon_generation = app.icon_generation.wrapping_add(1);
    app.icons.clear();
    app.icon_rx = Some(receiver);

    app.poll_icons();

    assert!(app.icons.get(&path).is_none());
}

#[test]
fn slow_system_provider_keeps_nickel_fallback_visible_before_async_results() {
    let entries = vec![
        FileEntry {
            name: "Documents".into(),
            path: PathBuf::from("/fixture/Documents"),
            is_directory: true,
            size: None,
            modified: None,
        },
        FileEntry {
            name: "notes.txt".into(),
            path: PathBuf::from("/fixture/notes.txt"),
            is_directory: false,
            size: Some(12),
            modified: None,
        },
    ];
    let mut app = FileApp::with_browser(DirectoryBrowser::fixture(entries), String::new());
    app.refresh_icons_for_theme(
        FileIconPreference::System,
        Some("deliberately-unavailable-theme"),
        ThemeMode::Dark,
    );

    assert!(app.icons.len() >= 2);
    assert!(
        app.browser
            .entries()
            .iter()
            .all(|entry| app.icons.get(&entry.path).is_some())
    );
    assert!(
        location_groups()
            .into_iter()
            .flat_map(|group| group.entries)
            .any(|(_, path)| app.icons.get(&path).is_some())
    );
    assert!(app.tab_icon.is_some());
    assert!(app.icon_rx.is_some());
}

#[test]
fn sidebar_locations_render_from_the_shared_resolved_artwork_cache() {
    let app = FileApp::fixture();
    let host = UiHost::new(app, 1_100, 700);
    assert!(
        host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::Image { bounds, .. }
                if bounds.origin.x < DEFAULT_SIDEBAR_WIDTH
                    && bounds.origin.y > TOOLBAR_HEIGHT
        )),
        "the shared image list must include artwork inside the places sidebar"
    );
}

#[test]
fn provider_changes_preserve_independent_tab_view_state() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(directory.path().join("root.txt"), b"root").unwrap();
    std::fs::write(child.join("child.txt"), b"child").unwrap();
    let mut app = FileApp::new(directory.path().to_path_buf());
    app.selected = Some(1);
    app.selected_entries = HashSet::from([1]);
    app.selection_anchor = Some(1);
    app.file_scroll_offset = 31.0;
    app.view_mode = FileViewMode::Details;
    app.sort_key = EntrySortKey::Size;
    app.sort_direction = SortDirection::Descending;
    app.new_tab_at(child.clone());
    settle_navigation(&mut app);
    let inactive_icon_before = app.inactive_tab(0).unwrap().tab_icon.as_ref().unwrap().0;
    app.selected = Some(0);
    app.selected_entries = HashSet::from([0]);
    app.selection_anchor = Some(0);
    app.file_scroll_offset = 17.0;
    app.sort_key = EntrySortKey::Modified;

    app.refresh_icons_for_theme(
        FileIconPreference::System,
        Some("fixture-theme"),
        ThemeMode::Light,
    );
    assert_eq!(app.icon_theme.as_deref(), Some("fixture-theme"));
    app.refresh_icons_for(FileIconPreference::Nickel, ThemeMode::Dark);

    assert_eq!(app.browser.current(), child);
    assert_eq!(app.selected_entries, HashSet::from([0]));
    assert_eq!(app.file_scroll_offset, 17.0);
    assert_eq!(app.view_mode, FileViewMode::Grid);
    assert_eq!(app.sort_key, EntrySortKey::Modified);
    assert!(app.tab_icon.is_some());
    let root = app.inactive_tab(0).unwrap();
    assert!(root.tab_icon.is_some());
    assert_ne!(root.tab_icon.as_ref().unwrap().0, inactive_icon_before);
    assert_eq!(root.browser.current(), directory.path());
    assert_eq!(root.selected_entries, HashSet::from([1]));
    assert_eq!(root.file_scroll_offset, 31.0);
    assert_eq!(root.view_mode, FileViewMode::Details);
    assert_eq!(root.sort_key, EntrySortKey::Size);
    assert_eq!(root.sort_direction, SortDirection::Descending);
}

#[test]
fn provider_revision_changes_refresh_artwork_without_losing_view_state() {
    let mut app = FileApp::fixture();
    app.selected = Some(1);
    app.selected_entries = HashSet::from([1]);
    app.file_scroll_offset = 23.0;
    app.view_mode = FileViewMode::Details;
    let generation = app.icon_generation;
    app.icon_provider_revision = app.icon_provider_revision.wrapping_add(1);

    assert!(app.sync_icon_settings());
    assert_ne!(app.icon_generation, generation);
    assert_eq!(app.selected_entries, HashSet::from([1]));
    assert_eq!(app.file_scroll_offset, 23.0);
    assert_eq!(app.view_mode, FileViewMode::Details);
}

#[test]
fn display_scale_changes_refresh_physical_artwork_without_losing_selection() {
    let mut app = FileApp::fixture();
    let path = app.browser.entries()[0].path.clone();
    app.selected = Some(0);
    app.selected_entries = HashSet::from([0]);
    let generation = app.icon_generation;

    assert!(Application::scale_factor_changed(&mut app, 1.25));
    assert_eq!(app.artwork_scale_milli, 1_250);
    assert_ne!(app.icon_generation, generation);
    assert_eq!(app.selected_entries, HashSet::from([0]));
    let pixels = &app.icons.get(&path).unwrap().1;
    assert_eq!((pixels.width(), pixels.height()), (120, 120));
    assert!(!Application::scale_factor_changed(&mut app, 1.25));
}

#[test]
fn details_column_widths_resize_within_bounds_and_belong_to_their_tab() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(directory.path().join("root.txt"), b"root").unwrap();
    std::fs::write(child.join("child.txt"), b"child").unwrap();
    let mut app = FileApp::new(directory.path().to_path_buf());
    app.view_mode = FileViewMode::Details;
    app.cursor.x = 200.0;
    app.update(FileMessage::ResizeDetailsColumn(DetailsColumn::Type));
    app.resize_details_column_to(900.0);
    assert_eq!(
        app.details_column_widths.type_width,
        MAX_DETAILS_COLUMN_WIDTH
    );
    app.resizing_details_column = None;

    app.cursor.x = 200.0;
    app.update(FileMessage::ResizeDetailsColumn(DetailsColumn::Size));
    app.resize_details_column_to(-900.0);
    assert_eq!(
        app.details_column_widths.size_width,
        MIN_DETAILS_COLUMN_WIDTH
    );
    app.resizing_details_column = None;
    let root_widths = app.details_column_widths;

    app.new_tab_at(child);
    assert_eq!(app.details_column_widths, DetailsColumnWidths::default());
    app.switch_tab(0);
    assert_eq!(app.details_column_widths, root_widths);
}

#[test]
fn tile_width_commands_are_bounded_and_owned_by_each_tab() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let mut app = FileApp::new(directory.path().to_path_buf());
    app.update(FileMessage::AdjustTileWidth(1));
    let root_width = app.tile_width;
    assert!(root_width > DEFAULT_TILE_WIDTH);

    app.new_tab_at(child);
    assert_eq!(app.tile_width, DEFAULT_TILE_WIDTH);
    for _ in 0..20 {
        app.update(FileMessage::AdjustTileWidth(-1));
    }
    assert_eq!(app.tile_width, MIN_TILE_WIDTH);
    app.switch_tab(0);
    assert_eq!(app.tile_width, root_width);
}

#[test]
fn status_area_reports_selection_and_total_without_hiding_errors() {
    let mut app = FileApp::fixture();
    assert_eq!(file_status_text(&app), "3 items · fixture");

    app.selected_entries = HashSet::from([0, 2]);
    assert_eq!(file_status_text(&app), "2 selected · 3 items · fixture");

    app.status = "Could not refresh: unavailable".into();
    assert_eq!(file_status_text(&app), app.status);
}

#[test]
fn breadcrumbs_collapse_middle_ancestors_and_keep_actionable_endpoints() {
    let crumbs = ["Home", "Projects", "Nickel", "assets", "concepts"]
        .into_iter()
        .scan(PathBuf::from("/"), |path, label| {
            path.push(label);
            Some((label.to_owned(), path.clone()))
        })
        .collect::<Vec<_>>();

    assert_eq!(collapse_breadcrumbs(crumbs.clone(), 1_000.0), crumbs);
    let collapsed = collapse_breadcrumbs(crumbs, 190.0);
    assert_eq!(collapsed.first().unwrap().0, "Home");
    assert_eq!(collapsed[1].0, "…");
    assert_eq!(
        collapsed[1].1,
        PathBuf::from("/Home/Projects/Nickel/assets")
    );
    assert_eq!(collapsed.last().unwrap().0, "concepts");
}

#[test]
fn grid_and_details_modes_are_owned_independently_by_each_tab() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(directory.path().join("report.txt"), b"report").unwrap();
    std::fs::write(child.join("photo.png"), b"fixture").unwrap();
    let mut app = FileApp::new(directory.path().to_path_buf());

    app.update_message(FileMessage::SetViewMode(FileViewMode::Details));
    app.update_message(FileMessage::SortBy(EntrySortKey::Type));
    app.update_message(FileMessage::Entry(0));
    app.new_tab_at(child);
    assert_eq!(app.view_mode, FileViewMode::Grid);
    assert_eq!(app.sort_key, EntrySortKey::Name);
    app.update_message(FileMessage::Entry(0));

    app.switch_tab(0);
    assert_eq!(app.view_mode, FileViewMode::Details);
    assert_eq!(app.sort_key, EntrySortKey::Type);
    assert_eq!(app.selected_entries, HashSet::from([0]));
    app.switch_tab(1);
    assert_eq!(app.view_mode, FileViewMode::Grid);
    assert_eq!(app.sort_key, EntrySortKey::Name);
    assert_eq!(app.selected_entries, HashSet::from([0]));
}

#[test]
fn sorting_preserves_selection_by_entry_identity() {
    let mut app = FileApp::fixture();
    app.update_message(FileMessage::Entry(2));
    let selected_path = app.browser.entries()[2].path.clone();

    app.update_message(FileMessage::SortBy(EntrySortKey::Type));
    app.update_message(FileMessage::SortBy(EntrySortKey::Type));

    assert_eq!(app.sort_direction, SortDirection::Descending);
    assert_eq!(
        app.selected
            .and_then(|index| app.browser.entries().get(index))
            .map(|entry| &entry.path),
        Some(&selected_path)
    );
}

#[test]
fn editable_location_preserves_history_on_error_and_commits_once_on_success() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let mut app = FileApp::new(directory.path().to_path_buf());

    app.update_message(FileMessage::ToggleAddressEditing);
    assert_eq!(
        Application::take_focus_request(&mut app),
        Some(UiId::from("file-address-field"))
    );
    app.update_message(FileMessage::AddressChanged(
        directory.path().join("missing").display().to_string(),
    ));
    app.update_message(FileMessage::SubmitAddress);
    settle_navigation(&mut app);
    assert_eq!(app.browser.current(), directory.path());
    assert!(!app.browser.can_go_back());
    assert!(app.address_editing);
    assert!(app.status.contains("Could not open"));

    app.update_message(FileMessage::AddressChanged(child.display().to_string()));
    app.update_message(FileMessage::SubmitAddress);
    settle_navigation(&mut app);
    assert_eq!(app.browser.current(), child);
    assert!(app.browser.can_go_back());
    assert!(!app.address_editing);
    assert!(app.address_text.is_empty());
}

#[test]
fn location_editor_draft_and_visibility_belong_to_their_tab() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let mut app = FileApp::new(directory.path().to_path_buf());
    app.update_message(FileMessage::ToggleAddressEditing);
    app.update_message(FileMessage::AddressChanged("unfinished location".into()));

    app.new_tab_at(child);
    assert!(!app.address_editing);
    assert!(app.address_text.is_empty());
    app.switch_tab(0);
    assert!(app.address_editing);
    assert_eq!(app.address_text, "unfinished location");
}

#[test]
fn details_view_exposes_shared_columns_and_entry_targets() {
    let mut app = FileApp::fixture();
    app.view_mode = FileViewMode::Details;
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let frame = nickel_ui::UiFrame::layout(
        app.build_view(960.0, 640.0, palette, false),
        Rect::new(0.0, 0.0, 960.0, 640.0),
    );

    assert!(
        frame
            .resolved_layout()
            .nodes()
            .iter()
            .any(|node| node.id.as_str().ends_with("/details-header"))
    );
    assert_eq!(
        frame
            .semantic_nodes()
            .iter()
            .filter(|node| node.id.as_str().contains("/file-entry-"))
            .count(),
        3
    );
}

#[test]
fn details_view_omits_file_only_metadata_for_directories() {
    let timestamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let entries = vec![
        FileEntry {
            name: "Folder".into(),
            path: PathBuf::from("/fixture/Folder"),
            is_directory: true,
            size: Some(4_096),
            modified: Some(timestamp),
        },
        FileEntry {
            name: "report.bin".into(),
            path: PathBuf::from("/fixture/report.bin"),
            is_directory: false,
            size: Some(4_096),
            modified: Some(timestamp),
        },
    ];
    let mut app = FileApp::with_browser(DirectoryBrowser::fixture(entries), String::new());
    app.view_mode = FileViewMode::Details;
    let palette = ThemePalette::from_appearance(Appearance::default());
    let frame = nickel_ui::UiFrame::layout(
        app.build_view(960.0, 640.0, palette, false),
        Rect::new(0.0, 0.0, 960.0, 640.0),
    );
    let rendered_text = frame
        .commands()
        .iter()
        .filter_map(|command| match command {
            nickel_ui::backend::PaintCommand::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        rendered_text
            .iter()
            .filter(|text| **text == "4.0 KB")
            .count(),
        1,
        "only the regular file may render its size"
    );
    let modified = format_modified(timestamp);
    assert_eq!(
        rendered_text
            .iter()
            .filter(|text| **text == modified)
            .count(),
        1,
        "only the regular file may render its modified time"
    );
}

#[test]
fn file_views_contain_non_square_provider_artwork_without_stretching() {
    let mut app = FileApp::fixture();
    let path = app.browser.entries()[0].path.clone();
    let asset_id = 60_000;
    app.icons.insert(
        path,
        (
            asset_id,
            Arc::new(image::RgbaImage::from_pixel(
                80,
                40,
                image::Rgba([20, 80, 160, 255]),
            )),
        ),
    );
    let palette = ThemePalette::from_appearance(Appearance::default());
    let image_bounds = |app: &FileApp| {
        nickel_ui::UiFrame::layout(
            app.build_view(960.0, 640.0, palette, false),
            Rect::new(0.0, 0.0, 960.0, 640.0),
        )
        .commands()
        .iter()
        .find_map(|command| match command {
            nickel_ui::backend::PaintCommand::Image { bounds, id, .. } if *id == asset_id => {
                Some(*bounds)
            }
            _ => None,
        })
        .expect("injected provider artwork must be painted")
    };

    let grid = image_bounds(&app);
    assert!((grid.size.width / grid.size.height - 2.0).abs() < 0.01);
    app.view_mode = FileViewMode::Details;
    let details = image_bounds(&app);
    assert!((details.size.width / details.size.height - 2.0).abs() < 0.01);
}

#[test]
fn growing_details_name_column_contains_multiline_text_without_row_overlap() {
    let long_name = "A deliberately long file name that wraps across\nmultiple lines.txt";
    let entries = vec![
        FileEntry {
            name: long_name.into(),
            path: PathBuf::from("/fixture").join(long_name),
            is_directory: false,
            size: Some(10),
            modified: None,
        },
        FileEntry {
            name: "following.txt".into(),
            path: PathBuf::from("/fixture/following.txt"),
            is_directory: false,
            size: Some(20),
            modified: None,
        },
    ];
    let mut app = FileApp::with_browser(DirectoryBrowser::fixture(entries), String::new());
    app.view_mode = FileViewMode::Details;
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let frame = nickel_ui::UiFrame::layout(
        app.build_view(620.0, 420.0, palette, false),
        Rect::new(0.0, 0.0, 620.0, 420.0),
    );
    let bounds = |suffix: &str| {
        frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with(suffix))
            .unwrap_or_else(|| panic!("missing {suffix}"))
            .allocated
    };
    let first = bounds("/file-entry-0");
    let name = bounds("/details-name-0");
    let second = bounds("/file-entry-1");

    assert!(name.size.width >= 120.0);
    assert!(name.origin.y >= first.origin.y);
    assert!(name.origin.y + name.size.height <= first.origin.y + first.size.height);
    assert!(first.origin.y + first.size.height <= second.origin.y);
}

#[test]
fn compact_grid_contains_multiline_labels_inside_their_rows() {
    let entries = (0..5)
        .map(|index| {
            let name = if index == 0 {
                "A deliberately long file name that wraps across multiple lines.txt".to_owned()
            } else {
                format!("file-{index}.txt")
            };
            FileEntry {
                path: PathBuf::from("/fixture").join(&name),
                name: name.into(),
                is_directory: false,
                size: Some(10),
                modified: None,
            }
        })
        .collect();
    let app = FileApp::with_browser(DirectoryBrowser::fixture(entries), String::new());
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let frame = nickel_ui::UiFrame::layout(
        app.build_view(660.0, 480.0, palette, false),
        Rect::new(0.0, 0.0, 660.0, 480.0),
    );
    let bounds = |suffix: &str| {
        frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with(suffix))
            .unwrap_or_else(|| panic!("missing {suffix}"))
            .allocated
    };
    let first = bounds("/file-entry-0");
    let next_row = bounds("/file-entry-4");

    assert_eq!(first.size.width, app.tile_width);
    assert!(first.origin.y + first.size.height <= next_row.origin.y);
}

#[test]
fn narrow_places_surface_replaces_squeezed_sidebar_without_losing_view_state() {
    let mut app = FileApp::fixture();
    app.view_mode = FileViewMode::Details;
    app.selected = Some(1);
    app.selected_entries = HashSet::from([1]);
    app.file_scroll_offset = 17.0;
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let render = |app: &FileApp| {
        nickel_ui::UiFrame::layout(
            app.build_view(600.0, 420.0, palette, false),
            Rect::new(0.0, 0.0, 600.0, 420.0),
        )
    };

    let files = render(&app);
    assert!(
        files
            .resolved_layout()
            .nodes()
            .iter()
            .any(|node| node.id.as_str().ends_with("/files-pane"))
    );
    assert!(
        files
            .resolved_layout()
            .nodes()
            .iter()
            .all(|node| !node.id.as_str().ends_with("/sidebar-pane"))
    );

    app.update_message(FileMessage::TogglePlaces);
    let places = render(&app);
    assert!(
        places
            .resolved_layout()
            .nodes()
            .iter()
            .any(|node| node.id.as_str().ends_with("/narrow-places-surface"))
    );
    assert_eq!(app.view_mode, FileViewMode::Details);
    assert_eq!(app.selected_entries, HashSet::from([1]));
    assert_eq!(app.file_scroll_offset, 17.0);

    app.update_message(FileMessage::TogglePlaces);
    assert!(!app.places_open);
    assert_eq!(app.view_mode, FileViewMode::Details);
    assert_eq!(app.selected_entries, HashSet::from([1]));
}

#[test]
fn location_groups_omit_empty_sections_and_collapse_without_reordering() {
    let groups = crate::platform::location_groups();
    assert!(!groups.is_empty());
    assert!(groups.iter().all(|group| !group.entries.is_empty()));
    assert_eq!(groups[0].id, "nickel-home");
    let home = groups[0].entries[0].1.clone();

    let mut app = FileApp::fixture();
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let render = |app: &FileApp| {
        nickel_ui::UiFrame::layout(
            app.build_view(960.0, 640.0, palette, false),
            Rect::new(0.0, 0.0, 960.0, 640.0),
        )
    };
    assert!(
        !render(&app)
            .semantic_targets_for_message(&FileMessage::OpenFolder(home.clone()))
            .is_empty()
    );

    app.update_message(FileMessage::ToggleLocationGroup("nickel-home".into()));
    assert!(
        render(&app)
            .semantic_targets_for_message(&FileMessage::OpenFolder(home.clone()))
            .is_empty()
    );
    assert_eq!(crate::platform::location_groups()[0].entries[0].1, home);
}

#[test]
fn sidebar_expansion_enumerates_children_asynchronously() {
    let root = tempfile::tempdir().unwrap();
    let child = root.path().join("Child");
    std::fs::create_dir(&child).unwrap();
    let mut app = FileApp::new(root.path().to_path_buf());
    let root_path = root.path().to_path_buf();

    app.update_message(FileMessage::ToggleFolder(root_path.clone()));

    assert!(app.expanded_folders.contains(&root_path));
    assert!(app.sidebar_loading.contains(&root_path));
    assert!(!app.sidebar_children.contains_key(&root_path));

    for _ in 0..1_000 {
        if app.poll_sidebar_children() {
            break;
        }
        std::thread::yield_now();
    }
    assert!(!app.sidebar_loading.contains(&root_path));
    assert_eq!(
        app.sidebar_children.get(&root_path),
        Some(&vec![("Child".to_owned(), child.clone())])
    );
    assert!(
        app.icons.get(&child).is_some(),
        "published sidebar children must receive immediate fallback artwork"
    );
}

#[test]
fn synthetic_location_sources_use_shared_group_order_and_one_target_identity() {
    let shared = PathBuf::from("/fixture/shared");
    let groups = crate::platform::location_groups_from(crate::platform::LocationSources {
        nickel_home: vec![("Home".into(), PathBuf::from("/fixture/home"))],
        user_pins: vec![("Pinned".into(), shared.clone())],
        user_locations: vec![("Cloud".into(), shared.clone())],
        computer_and_volumes: vec![("Drive".into(), PathBuf::from("/fixture/drive"))],
        network: vec![("Server".into(), PathBuf::from("/fixture/server"))],
    });

    assert_eq!(
        groups.iter().map(|group| group.id).collect::<Vec<_>>(),
        ["nickel-home", "user-pins", "computer-volumes", "network"]
    );
    assert_eq!(
        groups.iter().map(|group| group.title).collect::<Vec<_>>(),
        ["Nickel Home", "Pins", "Computer & volumes", "Network"]
    );
    assert_eq!(
        groups
            .iter()
            .flat_map(|group| &group.entries)
            .filter(|(_, path)| path == &shared)
            .count(),
        1
    );
}

#[test]
fn rtl_file_grid_mirrors_semantic_columns_without_changing_entry_identity() {
    let mut app = FileApp::fixture();
    app.reading_direction = nickel_ui::ReadingDirection::RightToLeft;
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let frame = nickel_ui::UiFrame::layout(
        app.build_view(960.0, 640.0, palette, false),
        Rect::new(0.0, 0.0, 960.0, 640.0),
    );
    let nodes = frame.semantic_nodes();
    let first_cell = nodes
        .iter()
        .find(|node| {
            node.role == Some(nickel_ui::SemanticRole::GridCell)
                && node
                    .description
                    .as_deref()
                    .is_some_and(|description| description.contains("item 1 of 3"))
        })
        .expect("first grid cell retains collection semantics");

    assert_eq!(
        first_cell.description.as_deref(),
        Some("row 1, column 3, item 1 of 3")
    );
    assert!(
        !frame
            .semantic_targets_for_message(&FileMessage::Entry(0))
            .is_empty()
    );
}

#[test]
fn searchable_command_surface_filters_aliases_and_executes_enabled_actions() {
    let mut app = FileApp::fixture();
    app.localizer = nickel_i18n::Localizer::for_locale(Some("es"));
    app.update_message(FileMessage::ToggleCommandSurface);
    assert_eq!(
        Application::take_focus_request(&mut app),
        Some(UiId::from("file-command-query"))
    );
    app.update_message(FileMessage::CommandQueryChanged("detalles".into()));
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let frame = nickel_ui::UiFrame::layout(
        app.build_view(960.0, 640.0, palette, false),
        Rect::new(0.0, 0.0, 960.0, 640.0),
    );

    assert_eq!(
        frame
            .semantic_targets_for_message(&FileMessage::SetViewMode(FileViewMode::Details))
            .iter()
            .filter(|target| target.id.as_str().contains("/file-command-"))
            .count(),
        1
    );
    assert!(
        frame
            .semantic_targets_for_message(&FileMessage::SetViewMode(FileViewMode::Grid))
            .iter()
            .all(|target| !target.id.as_str().contains("/file-command-"))
    );

    app.update_message(FileMessage::CommandQueryChanged("columns".into()));
    let alias_frame = nickel_ui::UiFrame::layout(
        app.build_view(960.0, 640.0, palette, false),
        Rect::new(0.0, 0.0, 960.0, 640.0),
    );
    assert_eq!(
        alias_frame
            .semantic_targets_for_message(&FileMessage::SetViewMode(FileViewMode::Details))
            .iter()
            .filter(|target| target.id.as_str().contains("/file-command-"))
            .count(),
        1
    );

    app.update_message(FileMessage::SetViewMode(FileViewMode::Details));
    assert!(!app.command_surface_open);
    assert!(app.command_query.is_empty());
    assert_eq!(app.view_mode, FileViewMode::Details);
}

#[test]
fn disabled_command_has_no_executable_semantic_target() {
    let mut app = FileApp::fixture();
    app.update_message(FileMessage::ToggleCommandSurface);
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let frame = nickel_ui::UiFrame::layout(
        app.build_view(960.0, 640.0, palette, false),
        Rect::new(0.0, 0.0, 960.0, 640.0),
    );

    let targets = frame.semantic_targets_for_message(&FileMessage::Back);
    assert!(
        targets.is_empty(),
        "Back is visible but must remain non-executable without history: {targets:#?}"
    );
}

#[test]
fn open_in_new_tab_is_enabled_only_for_browsable_containers() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("Documents")).unwrap();
    std::fs::write(root.path().join("report.txt"), b"report").unwrap();
    let mut app = FileApp::new(root.path().to_path_buf());
    app.update_message(FileMessage::ToggleCommandSurface);
    app.selected = Some(1);
    app.selected_entries = HashSet::from([1]);
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let file_frame = nickel_ui::UiFrame::layout(
        app.build_view(960.0, 640.0, palette, false),
        Rect::new(0.0, 0.0, 960.0, 640.0),
    );
    assert!(
        file_frame
            .semantic_targets_for_message(&FileMessage::ContextOpenNewTab)
            .is_empty()
    );
    app.update_message(FileMessage::ContextOpenNewTab);
    assert_eq!(app.tabs.len(), 1);

    app.update_message(FileMessage::ToggleCommandSurface);
    app.selected = Some(0);
    app.selected_entries = HashSet::from([0]);
    let directory_frame = nickel_ui::UiFrame::layout(
        app.build_view(960.0, 640.0, palette, false),
        Rect::new(0.0, 0.0, 960.0, 640.0),
    );
    assert_eq!(
        directory_frame
            .semantic_targets_for_message(&FileMessage::ContextOpenNewTab)
            .len(),
        1
    );
    app.update_message(FileMessage::ContextOpenNewTab);
    assert_eq!(app.tabs.len(), 2);
}

#[test]
fn command_surface_exposes_open_and_complete_tab_management() {
    let mut app = FileApp::fixture();
    app.selected = Some(0);
    app.selected_entries = HashSet::from([0]);
    app.update_message(FileMessage::ToggleCommandSurface);
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let frame = nickel_ui::UiFrame::layout(
        app.build_view(960.0, 640.0, palette, false),
        Rect::new(0.0, 0.0, 960.0, 640.0),
    );

    for message in [
        FileMessage::ContextOpen,
        FileMessage::ContextOpenNewTab,
        FileMessage::NewTab,
        FileMessage::CloseTab(app.active_tab),
    ] {
        assert!(
            frame
                .semantic_targets_for_message(&message)
                .iter()
                .any(|target| target.id.as_str().contains("/file-command-")),
            "missing command target for {message:?}"
        );
    }
}

#[test]
fn navigation_from_idle_publishes_fallback_before_optional_provider_work() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(child.join("new-file.txt"), b"x").unwrap();
    let mut app = FileApp::new(directory.path().to_path_buf());
    app.icon_rx = None;

    assert_eq!(Application::poll_interval(&app), None);
    app.navigate_to(child.clone());

    assert_eq!(app.browser.current(), directory.path());
    assert!(app.navigation_pending());
    assert!(app.status.starts_with("Opening "));
    settle_navigation(&mut app);

    assert_eq!(app.browser.current(), child);
    assert!(app.icons.get(&child.join("new-file.txt")).is_some());
    assert!(app.tab_icon.is_some());
    assert_eq!(
        Application::poll_interval(&app).is_some(),
        app.icon_preference == FileIconPreference::System
    );
}

#[test]
fn pending_navigation_belongs_to_the_requesting_tab() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("destination");
    let other = directory.path().join("other");
    std::fs::create_dir(&destination).unwrap();
    std::fs::create_dir(&other).unwrap();
    let mut app = FileApp::new(directory.path().to_path_buf());

    app.navigate_to(destination.clone());
    assert!(app.navigation_pending());
    app.new_tab_at(other.clone());
    assert_eq!(app.browser.current(), other);
    assert!(app.navigation_pending());

    app.switch_tab(0);
    assert!(app.navigation_pending());
    settle_navigation(&mut app);
    assert_eq!(app.browser.current(), destination);
    app.switch_tab(1);
    assert!(app.navigation_pending());
    settle_navigation(&mut app);
    assert_eq!(app.browser.current(), other);
    assert!(!app.browser.can_go_back());
}

#[test]
fn new_tab_publishes_immediately_and_reports_open_failure_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing");
    let mut app = FileApp::new(directory.path().to_path_buf());

    app.new_tab_at(missing.clone());

    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.browser.current(), missing);
    assert!(app.browser.entries().is_empty());
    assert!(app.navigation_pending());
    assert_eq!(app.status, "Opening location…");

    settle_navigation(&mut app);
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.browser.current(), missing);
    assert!(app.browser.entries().is_empty());
    assert!(!app.browser.can_go_back());
    assert!(app.status.contains("Could not open new tab"));
}

#[test]
fn failed_location_state_belongs_to_its_tab_and_recovers_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing");
    let valid = directory.path().join("valid");
    std::fs::create_dir(&valid).unwrap();
    std::fs::write(valid.join("recovered.txt"), b"ok").unwrap();
    let mut app = FileApp::new(directory.path().to_path_buf());

    app.new_tab_at(missing.clone());
    settle_navigation(&mut app);
    let failure = app.status.clone();
    assert!(failure.contains("Could not open new tab"));

    app.switch_tab(0);
    assert!(app.status.is_empty());
    assert_eq!(app.browser.current(), directory.path());

    app.switch_tab(1);
    assert_eq!(app.status, failure);
    assert_eq!(app.browser.current(), missing);
    app.navigate_to(valid.clone());
    settle_navigation(&mut app);

    assert!(app.status.is_empty());
    assert_eq!(app.browser.current(), valid);
    assert_eq!(app.browser.entries()[0].display_name(), "recovered.txt");
    app.switch_tab(0);
    assert!(app.status.is_empty());
    assert_eq!(app.browser.current(), directory.path());
}

#[test]
fn closing_active_tab_selects_nearest_survivor_without_mutating_it() {
    let root = tempfile::tempdir().unwrap();
    let middle = root.path().join("middle");
    let right = root.path().join("right");
    std::fs::create_dir(&middle).unwrap();
    std::fs::create_dir(&right).unwrap();
    std::fs::write(middle.join("middle.txt"), b"middle").unwrap();
    std::fs::write(right.join("right.txt"), b"right").unwrap();
    let mut app = FileApp::new(root.path().to_path_buf());

    app.new_tab_at(middle);
    settle_navigation(&mut app);
    app.selected = Some(0);
    app.selected_entries = HashSet::from([0]);
    app.file_scroll_offset = 23.0;
    app.view_mode = FileViewMode::Details;
    app.sort_key = EntrySortKey::Size;

    app.new_tab_at(right.clone());
    settle_navigation(&mut app);
    app.selected = Some(0);
    app.selected_entries = HashSet::from([0]);
    app.file_scroll_offset = 41.0;
    app.view_mode = FileViewMode::Grid;
    app.sort_key = EntrySortKey::Modified;

    app.switch_tab(1);
    app.close_tab(1);

    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.active_tab, 1);
    assert_eq!(app.browser.current(), right);
    assert_eq!(app.selected_entries, HashSet::from([0]));
    assert_eq!(app.file_scroll_offset, 41.0);
    assert_eq!(app.view_mode, FileViewMode::Grid);
    assert_eq!(app.sort_key, EntrySortKey::Modified);

    app.close_tab(1);
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.browser.current(), root.path());
}

#[test]
fn refresh_keeps_usable_contents_until_async_enumeration_publishes() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("existing.txt"), b"existing").unwrap();
    let mut app = FileApp::new(directory.path().to_path_buf());
    let existing_path = directory.path().join("existing.txt");
    let artwork_id_before = app.icons.get(&existing_path).unwrap().0;
    std::fs::write(directory.path().join("new.txt"), b"new").unwrap();

    app.update_message(FileMessage::ContextRefresh);
    assert!(app.navigation_pending());
    assert!(
        app.browser
            .entries()
            .iter()
            .all(|entry| entry.name != std::ffi::OsStr::new("new.txt"))
    );
    settle_navigation(&mut app);
    assert!(
        app.browser
            .entries()
            .iter()
            .any(|entry| entry.name == std::ffi::OsStr::new("new.txt"))
    );
    assert_ne!(app.icons.get(&existing_path).unwrap().0, artwork_id_before);
}

#[test]
fn hidden_file_enumeration_is_published_asynchronously() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("visible.txt"), b"visible").unwrap();
    std::fs::write(directory.path().join(".hidden.txt"), b"hidden").unwrap();
    let browser = DirectoryBrowser::open_with_hidden(directory.path(), false).unwrap();
    let mut app = FileApp::with_browser(browser, String::new());

    app.update_message(FileMessage::ToggleHiddenFiles);
    assert!(app.navigation_pending());
    assert!(!app.browser.show_hidden());
    settle_navigation(&mut app);
    assert!(app.browser.show_hidden());
    assert!(
        app.browser
            .entries()
            .iter()
            .any(|entry| entry.name == std::ffi::OsStr::new(".hidden.txt"))
    );
}

#[test]
fn component_owned_fixture_registers_the_production_file_app() {
    use nickel_ui_testkit::FixtureProvider;
    let mut registry = nickel_ui_testkit::FixtureRegistry::new();
    FileFixtureProvider.register(&mut registry).unwrap();
    let entries = registry.finish();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].metadata.id, "file.browser");
    let session = entries[0].open();
    assert!(
        session
            .semantic_nodes()
            .iter()
            .any(|node| node.id.as_str().ends_with("/file-entry-0"))
    );
}

#[test]
fn every_file_fixture_passes_geometry_accessibility_and_selection_contrast_gates() {
    for variant in FILE_FIXTURE_VARIANTS {
        let app = FileWorkbenchFixture::create_variant(variant);
        let appearance = app.fixture_appearance.expect("fixtures declare appearance");
        let palette = ThemePalette::from_appearance(appearance);
        assert!(
            contrast_ratio(palette.text, palette.accent_soft) >= MINIMUM_SELECTION_TEXT_CONTRAST,
            "{} selection text contrast is {:.2}, below the declared {:.1}:1 threshold",
            variant.id,
            contrast_ratio(palette.text, palette.accent_soft),
            MINIMUM_SELECTION_TEXT_CONTRAST,
        );

        let mut host = UiHost::new(app, variant.viewport.width, variant.viewport.height);
        host.set_scale_factor(variant.scale.factor);
        let inspection = host.inspect();
        assert!(
            inspection.diagnostics.is_empty(),
            "{} has layout diagnostics: {:?}",
            variant.id,
            inspection.diagnostics,
        );
        assert!(
            inspection.overlay_failures.is_empty(),
            "{} has overlay failures: {:?}",
            variant.id,
            inspection.overlay_failures,
        );

        let nodes = host.semantic_nodes();
        for node in &nodes {
            assert!(
                node.bounds.origin.x.is_finite()
                    && node.bounds.origin.y.is_finite()
                    && node.bounds.size.width.is_finite()
                    && node.bounds.size.height.is_finite(),
                "{} contains non-finite geometry for {:?}: {:?}",
                variant.id,
                node.id,
                node.bounds,
            );
            assert!(
                node.actions.is_empty()
                    || node
                        .name
                        .as_deref()
                        .is_some_and(|name| !name.trim().is_empty()),
                "{} has an actionable node without an accessible name: {:?}",
                variant.id,
                node.id,
            );
            if !node.actions.is_empty() {
                assert!(
                    node.bounds.size.width > 0.0
                        && node.bounds.size.height > 0.0
                        && node.bounds.origin.x >= 0.0
                        && node.bounds.origin.y >= 0.0
                        && node.bounds.origin.x + node.bounds.size.width
                            <= variant.viewport.width as f32
                        && node.bounds.origin.y + node.bounds.size.height
                            <= variant.viewport.height as f32,
                    "{} clips actionable control {:?} at {:?}",
                    variant.id,
                    node.id,
                    node.bounds,
                );
            }
        }

        let entries = nodes
            .iter()
            .filter(|node| node.id.as_str().contains("/file-entry-"))
            .collect::<Vec<_>>();
        for (index, left) in entries.iter().enumerate() {
            for right in entries.iter().skip(index + 1) {
                assert!(
                    !bounds_overlap(left.bounds, right.bounds),
                    "{} has overlapping file hit regions: {:?} {:?} and {:?} {:?}",
                    variant.id,
                    left.id,
                    left.bounds,
                    right.id,
                    right.bounds,
                );
            }
        }
    }
}

#[test]
fn every_file_fixture_asset_matches_its_admitted_source_master() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for asset in FILE_FIXTURE_ASSETS {
        let path = repository.join(asset.path);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            asset.sha256,
            "stale fixture digest for {}",
            asset.path,
        );
        let image = image::load_from_memory(&bytes)
            .unwrap_or_else(|error| panic!("could not decode {}: {error}", path.display()));
        assert_eq!(
            (image.width(), image.height()),
            (1_254, 1_254),
            "{} must remain the retained source master",
            asset.path,
        );
    }
}

#[test]
fn synthetic_windows_and_linux_adapters_share_semantics_geometry_and_commands() {
    #[derive(Clone, Copy)]
    enum SyntheticPlatform {
        Windows,
        Linux,
    }

    let scenario = |_platform: SyntheticPlatform| {
        let mut app = FileApp::fixture();
        app.refresh_icons_for(FileIconPreference::Nickel, ThemeMode::Dark);
        app.icon_rx = None;
        Scenario::new(app, 1100, 700)
    };
    let mut windows = scenario(SyntheticPlatform::Windows);
    let mut linux = scenario(SyntheticPlatform::Linux);

    assert_eq!(windows.semantic_nodes(), linux.semantic_nodes());
    assert_eq!(
        windows.host().accessibility_nodes(),
        linux.host().accessibility_nodes()
    );
    assert_eq!(
        nickel_ui_testkit::render_host(windows.host(), 1100, 700, 1.0),
        nickel_ui_testkit::render_host(linux.host(), 1100, 700, 1.0)
    );

    let script = |scenario: &mut Scenario<FileApp>| {
        scenario
            .activate(&Selector::role_name(SemanticRole::Button, "report.txt"))?
            .activate(&Selector::role_name(SemanticRole::Button, "Details view"))?
            .activate(&Selector::role_name(SemanticRole::Button, "Sort by size"))?
            .activate(&Selector::role_name(SemanticRole::Button, "Open commands"))?;
        Ok::<_, nickel_ui_testkit::ScenarioError>(())
    };
    script(&mut windows).unwrap();
    script(&mut linux).unwrap();

    let state = |scenario: &Scenario<FileApp>| {
        let app = scenario.host().application();
        (
            app.browser.current().to_path_buf(),
            app.selected,
            app.selected_entries.clone(),
            app.view_mode,
            app.sort_key,
            app.sort_direction,
            app.command_surface_open,
            app.command_query.clone(),
            app.tabs.len(),
            app.active_tab,
        )
    };
    assert_eq!(state(&windows), state(&linux));
    assert_eq!(windows.semantic_nodes(), linux.semantic_nodes());
    assert_eq!(
        nickel_ui_testkit::render_host(windows.host(), 1100, 700, 1.0),
        nickel_ui_testkit::render_host(linux.host(), 1100, 700, 1.0)
    );
}

#[test]
fn every_advertised_controller_action_has_a_bounded_production_path() {
    use nickel_ui_testkit::{FixtureProvider, ReachabilityModality, ReachabilityPolicy};

    let mut registry = nickel_ui_testkit::FixtureRegistry::new();
    FileFixtureProvider.register(&mut registry).unwrap();
    let session = registry.finish().remove(0).open();
    let report = session.reachability_report(&ReachabilityPolicy {
        modalities: [ReachabilityModality::Controller].into_iter().collect(),
        // This exhaustively rebuilds the production tree for each controller
        // state. Keep a finite ceiling while allowing loaded source artwork on
        // slower debug and cross-platform CI hosts.
        wall_time_ms: 15_000,
        ..ReachabilityPolicy::default()
    });

    assert!(report.issues.is_empty(), "{:#?}", report.issues);
    assert!(
        report.paths.iter().all(|path| path.reached),
        "unreached controller paths: {:#?}",
        report
            .paths
            .iter()
            .filter(|path| !path.reached)
            .collect::<Vec<_>>()
    );
}

fn pixel(raster: &nickel_ui_testkit::HeadlessRaster, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * raster.width + x) * 4) as usize;
    raster.rgba[offset..offset + 4].try_into().unwrap()
}

fn changed_pixels_in(
    before: &nickel_ui_testkit::HeadlessRaster,
    after: &nickel_ui_testkit::HeadlessRaster,
    rect: Rect,
) -> usize {
    let x0 = rect.origin.x.max(0.0) as u32;
    let y0 = rect.origin.y.max(0.0) as u32;
    let x1 = (rect.origin.x + rect.size.width).min(before.width as f32) as u32;
    let y1 = (rect.origin.y + rect.size.height).min(before.height as f32) as u32;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|(x, y)| pixel(before, *x, *y) != pixel(after, *x, *y))
        .count()
}

#[test]
fn file_grid_resolves_responsively_without_application_column_arithmetic() {
    let directory = tempfile::tempdir().unwrap();
    for index in 0..18 {
        std::fs::write(directory.path().join(format!("item-{index}.txt")), b"x").unwrap();
    }
    let app = FileApp::new(directory.path().to_path_buf());
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let narrow = nickel_ui::UiFrame::layout(
        app.build_view(560.0, 420.0, palette, false),
        Rect::new(0.0, 0.0, 560.0, 420.0),
    );
    let wide = nickel_ui::UiFrame::layout(
        app.build_view(1280.0, 720.0, palette, false),
        Rect::new(0.0, 0.0, 1280.0, 720.0),
    );

    assert!(
        narrow.resolved_grid_columns().unwrap() < wide.resolved_grid_columns().unwrap(),
        "auto-fit should add columns as the resolved content pane widens"
    );
    assert!(
        narrow
            .scroll_extent(&FileMessage::FileScroll(0.0))
            .is_some_and(|extent| extent.can_scroll()),
        "all files should be measured and remain reachable through scrolling"
    );
}

#[test]
fn sidebar_divider_and_file_pane_share_one_fixed_boundary() {
    let directory = tempfile::tempdir().unwrap();
    for index in 0..256 {
        std::fs::write(directory.path().join(format!("item-{index}.txt")), b"x").unwrap();
    }
    let app = FileApp::new(directory.path().to_path_buf());
    let expected_boundary = app.sidebar_width + SIDEBAR_RESIZE_WIDTH;
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );

    for (width, height) in [(760, 420), (1280, 720)] {
        let frame = nickel_ui::UiFrame::layout(
            app.build_view(width as f32, height as f32, palette, false),
            Rect::new(0.0, 0.0, width as f32, height as f32),
        );
        let nodes = frame.resolved_layout().nodes();
        let bounds = |suffix: &str| {
            nodes
                .iter()
                .find(|node| node.id.as_str().ends_with(suffix))
                .unwrap_or_else(|| panic!("missing semantic node ending in {suffix}"))
                .allocated
        };
        let sidebar = bounds("/sidebar-pane");
        let divider = bounds("/sidebar-resize");
        let files = bounds("/files-pane");

        assert_eq!(sidebar.size.width, expected_boundary);
        assert_eq!(divider.origin.x, app.sidebar_width);
        assert_eq!(divider.size.width, SIDEBAR_RESIZE_WIDTH);
        assert_eq!(files.origin.x, expected_boundary);
        assert_eq!(files.size.width, width as f32 - expected_boundary);
    }
}

#[test]
fn directory_cardinality_does_not_change_shell_geometry_or_mounted_tile_bound() {
    let browser = |count| {
        DirectoryBrowser::fixture(
            (0..count)
                .map(|index| FileEntry {
                    name: format!("item-{index}.txt").into(),
                    path: PathBuf::from(format!("/fixture/item-{index}.txt")),
                    is_directory: false,
                    size: Some(1),
                    modified: None,
                })
                .collect(),
        )
    };
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let resolve = |count| {
        let app = FileApp::with_browser(browser(count), String::new());
        nickel_ui::UiFrame::layout(
            app.build_view(860.0, 620.0, palette, false),
            Rect::new(0.0, 0.0, 860.0, 620.0),
        )
    };
    let small = resolve(8);
    let large = resolve(4_096);
    let bounds = |frame: &nickel_ui::UiFrame<FileMessage>, suffix: &str| {
        frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with(suffix))
            .unwrap_or_else(|| panic!("missing node ending in {suffix}"))
            .allocated
    };

    for suffix in [
        "/toolbar-pane",
        "/sidebar-pane",
        "/sidebar-resize",
        "/files-pane",
        "/file-list",
    ] {
        assert_eq!(bounds(&small, suffix), bounds(&large, suffix), "{suffix}");
    }
    let mounted = large
        .semantic_nodes()
        .iter()
        .filter(|node| node.id.as_str().contains("/file-entry-"))
        .count();
    assert!(
        mounted <= 40,
        "virtual grid mounted {mounted} of 4096 tiles"
    );
    let viewport = bounds(&large, "/file-list");
    for tile in large
        .semantic_nodes()
        .iter()
        .filter(|node| node.id.as_str().contains("/file-entry-"))
    {
        assert!(
            tile.bounds.origin.x >= viewport.origin.x
                && tile.bounds.origin.x + tile.bounds.size.width
                    <= viewport.origin.x + viewport.size.width,
            "tile {:?} escaped the horizontal scroll clip: {:?} outside {:?}",
            tile.id,
            tile.bounds,
            viewport
        );
    }
}

#[test]
fn far_offscreen_selection_can_be_revealed_before_its_tile_is_mounted() {
    let entries = (0..4_096)
        .map(|index| FileEntry {
            name: format!("item-{index:04}.txt").into(),
            path: PathBuf::from(format!("/fixture/item-{index:04}.txt")),
            is_directory: false,
            size: Some(1),
            modified: None,
        })
        .collect();
    let mut app = FileApp::with_browser(DirectoryBrowser::fixture(entries), String::new());
    let selected = 4_000;
    let columns = 4;
    let row_height = 54.0 + (app.tile_width * 0.42).clamp(42.0, 96.0);
    let viewport_height = 483.0;
    let row_top = (selected / columns) as f32 * (row_height + 10.0);
    app.file_scroll_offset = row_top + row_height - viewport_height;
    let palette = ThemePalette::from_appearance(
        ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
    );
    let frame = nickel_ui::UiFrame::layout(
        app.build_view(860.0, 620.0, palette, false),
        Rect::new(0.0, 0.0, 860.0, 620.0),
    );

    assert!(
        frame
            .semantic_nodes()
            .iter()
            .any(|node| node.id.as_str().ends_with("/file-entry-4000")),
        "the target row must be mounted after its model-owned offset is applied"
    );
}

#[test]
fn drag_selection_uses_semantic_grid_membership_and_fails_closed() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["alpha.txt", "beta.txt"] {
        std::fs::write(directory.path().join(name), b"x").unwrap();
    }
    let mut host = UiHost::new(FileApp::new(directory.path().to_path_buf()), 860, 620);
    host.poll();
    let mut nodes = host.semantic_nodes();
    let all = Rect::new(0.0, 0.0, 860.0, 620.0);

    assert_eq!(entries_in_selection(&nodes, all, 2), HashSet::from([0, 1]));

    for node in nodes
        .iter_mut()
        .filter(|node| node.id.as_str().contains("/file-entry-"))
    {
        node.id = UiId::new("opaque-entry");
    }
    assert!(entries_in_selection(&nodes, all, 2).is_empty());
}

#[test]
fn context_menu_is_one_semantic_controller_and_accessibility_surface() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("report.txt"), b"x").unwrap();
    let mut host = UiHost::new(FileApp::new(directory.path().to_path_buf()), 860, 620);
    let entry = host
        .semantic_nodes()
        .into_iter()
        .find(|node| node.id.as_str().ends_with("/file-entry-0"))
        .expect("entry must retain its explicit local identity")
        .id;

    assert!(
        host.inspect().overlay_failures.is_empty(),
        "{:?}",
        host.inspect().overlay_failures
    );
    assert!(
        host.resolve_effective_target(&entry, ActionKind::ContextMenu)
            .is_ok()
    );
    host.handle_event(nickel_ui::UiEvent::AccessibilityContextMenu(entry.clone()));

    let menu_items = host.query(&nickel_ui::SemanticSelector::Role(
        nickel_ui::SemanticRole::MenuItem,
    ));
    assert_eq!(menu_items.len(), 1);
    assert!(menu_items.iter().all(|item| {
        item.actions.contains(&ActionKind::Activate)
            && host
                .accessibility_nodes()
                .iter()
                .any(|node| node.id == item.id)
    }));

    host.handle_event(nickel_ui::UiEvent::ControllerDown);
    assert!(host.inspect().controller_target.is_some());
    host.handle_event(nickel_ui::UiEvent::ControllerBack);
    assert!(host.inspect().open_overlay.is_none());
}

#[test]
fn controller_context_uses_target_geometry_not_stale_pointer_position() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("report.txt"), b"x").unwrap();
    let mut host = UiHost::new(FileApp::new(directory.path().to_path_buf()), 860, 620);
    host.handle_event(nickel_ui::UiEvent::FocusGained);
    host.handle_event(nickel_ui::UiEvent::ControllerDown);
    assert_eq!(
        host.inspect().modality,
        nickel_ui::InputModality::Controller
    );
    let background = host
        .semantic_nodes()
        .into_iter()
        .find(|node| node.id.as_str().ends_with("/file-content"))
        .unwrap()
        .id;
    host.perform_semantic_action(
        background,
        nickel_ui::SemanticAction::Invoke(ActionKind::ContextMenu),
    );

    let menu = host
        .query(&nickel_ui::SemanticSelector::Role(
            nickel_ui::SemanticRole::Menu,
        ))
        .pop()
        .unwrap();
    assert!(menu.bounds.origin.x > 100.0 && menu.bounds.origin.y > 100.0);
    let items = host.query(&nickel_ui::SemanticSelector::Role(
        nickel_ui::SemanticRole::MenuItem,
    ));
    let selected = items
        .iter()
        .find(|item| item.controller_selected)
        .expect("controller menu must visibly select its first action");
    let unselected = items
        .iter()
        .find(|item| !item.controller_selected)
        .expect("menu fixture has a second unselected action");
    let raster = nickel_ui_testkit::render_host(&host, 860, 620, 1.0);
    let first_id = selected.id.clone();
    let second_id = unselected.id.clone();
    let first_point = (
        selected.bounds.origin.x as u32 + 8,
        selected.bounds.origin.y as u32 + 8,
    );
    let second_point = (
        unselected.bounds.origin.x as u32 + 8,
        unselected.bounds.origin.y as u32 + 8,
    );
    let first_selected_color = pixel(&raster, first_point.0, first_point.1);
    let unselected_color = pixel(&raster, second_point.0, second_point.1);
    assert_ne!(
        first_selected_color, unselected_color,
        "controller-selected menu row must have a distinct raster fill"
    );

    host.handle_event(nickel_ui::UiEvent::ControllerDown);
    assert_eq!(
        host.inspect().controller_target,
        Some(second_id.clone()),
        "items after navigation: {:?}",
        host.query(&nickel_ui::SemanticSelector::Role(
            nickel_ui::SemanticRole::MenuItem
        ))
    );
    assert_ne!(host.inspect().controller_target, Some(first_id.clone()));
    let moved_items = host.query(&nickel_ui::SemanticSelector::Role(
        nickel_ui::SemanticRole::MenuItem,
    ));
    assert!(
        moved_items
            .iter()
            .any(|item| item.id == first_id && !item.focused && !item.controller_selected)
    );
    assert!(
        moved_items
            .iter()
            .any(|item| item.id == second_id && item.focused && item.controller_selected)
    );
    let moved = nickel_ui_testkit::render_host(&host, 860, 620, 1.0);
    assert!(
        changed_pixels_in(&raster, &moved, selected.bounds) > 0,
        "the former menu target must visibly lose its selected treatment"
    );
    assert!(
        changed_pixels_in(&raster, &moved, unselected.bounds) > 0,
        "the new controller target must visibly gain selected treatment"
    );
}

#[test]
fn ordinary_controller_target_has_a_distinct_visible_focus_ring() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("report.txt"), b"x").unwrap();
    let mut host = UiHost::new(FileApp::new(directory.path().to_path_buf()), 860, 620);
    host.handle_event(nickel_ui::UiEvent::FocusGained);
    let before = nickel_ui_testkit::render_host(&host, 860, 620, 1.0);
    let selected = (0..8)
        .find_map(|_| {
            host.handle_event(nickel_ui::UiEvent::ControllerDown);
            let target = host.inspect().controller_target?;
            if let Some(node) = host
                .semantic_nodes()
                .into_iter()
                .find(|node| node.id == target)
                .filter(|node| {
                    node.enabled
                        && node.actions.iter().any(|action| {
                            matches!(
                                action,
                                nickel_ui::ActionKind::Activate
                                    | nickel_ui::ActionKind::ContextMenu
                            )
                        })
                })
            {
                return Some(node);
            }
            host.handle_event(nickel_ui::UiEvent::ControllerActivate);
            None
        })
        .expect("controller must enter a pane and select an ordinary semantic target");
    let after = nickel_ui_testkit::render_host(&host, 860, 620, 1.0);
    assert!(
        changed_pixels_in(&before, &after, selected.bounds) > 0,
        "controller focus must visibly change pixels inside {selected:?}"
    );
}

fn entry_selector(scenario: &Scenario<FileApp>, suffix: &str) -> Selector {
    Selector::id(
        scenario
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str().ends_with(suffix))
            .unwrap_or_else(|| panic!("missing semantic File target {suffix}"))
            .id,
    )
}

#[test]
fn semantic_scenario_covers_context_routes_dismiss_scroll_and_resize() {
    let directory = tempfile::tempdir().unwrap();
    for index in 0..30 {
        std::fs::write(directory.path().join(format!("item-{index:02}.txt")), b"x").unwrap();
    }
    let mut scenario = Scenario::with_budget(
        FileApp::new(directory.path().to_path_buf()),
        960,
        640,
        ScenarioBudget {
            operations: 160,
            frames: 160,
            trace_steps: 160,
        },
    );
    let entry = entry_selector(&scenario, "/file-entry-0");

    scenario
        .accessibility_action(&entry, ActionKind::ContextMenu)
        .unwrap();
    assert!(scenario.host().inspect().open_overlay.is_some());
    scenario
        .controller(nickel_ui::ControllerAction::Cancel)
        .unwrap();
    assert!(scenario.host().inspect().open_overlay.is_none());

    for _ in 0..64 {
        if scenario.host().inspect().keyboard_focus.as_ref()
            == match &entry {
                Selector::Id(id) => Some(id),
                _ => None,
            }
        {
            break;
        }
        scenario.keyboard_focus(FocusDirection::Next).unwrap();
    }
    scenario.keyboard_context_focused().unwrap();
    assert!(scenario.host().inspect().open_overlay.is_some());
    scenario
        .controller(nickel_ui::ControllerAction::Cancel)
        .unwrap();

    scenario
        .controller(nickel_ui::ControllerAction::ContextMenu)
        .unwrap();
    assert!(scenario.host().inspect().open_overlay.is_some());
    scenario
        .controller(nickel_ui::ControllerAction::Cancel)
        .unwrap();

    let content = entry_selector(&scenario, "/file-content");
    scenario.pointer_scroll(&content, 0.0, 240.0).unwrap();
    scenario.resize(540, 420, 1.0).unwrap();
    scenario
        .assert_accessibility()
        .unwrap()
        .assert_no_diagnostics()
        .unwrap();
    let narrow_width = scenario
        .semantic_nodes()
        .into_iter()
        .find(|node| node.id.as_str().ends_with("/file-content"))
        .unwrap()
        .bounds
        .size
        .width;
    scenario.resize(1280, 720, 2.0).unwrap();
    let wide_width = scenario
        .semantic_nodes()
        .into_iter()
        .find(|node| node.id.as_str().ends_with("/file-content"))
        .unwrap()
        .bounds
        .size
        .width;
    assert!(wide_width > narrow_width);
    let operations = scenario.operation_trace();
    assert!(operations.iter().any(|step| matches!(
        step.operation,
        nickel_ui_testkit::ScenarioOperation::KeyboardContext
    )));
    assert!(operations.iter().any(|step| matches!(
        step.operation,
        nickel_ui_testkit::ScenarioOperation::Controller { .. }
    )));
    assert!(operations.iter().any(|step| matches!(
        step.operation,
        nickel_ui_testkit::ScenarioOperation::Accessibility { .. }
    )));
}

#[test]
fn semantic_scenarios_cover_empty_and_failure_states() {
    let empty = tempfile::tempdir().unwrap();
    let empty_scenario = Scenario::new(FileApp::new(empty.path().to_path_buf()), 720, 480);
    assert!(
        empty_scenario
            .host()
            .accessibility_nodes()
            .iter()
            .any(|node| { node.label.as_deref() == Some("This folder is empty.") })
    );
    empty_scenario.assert_accessibility().unwrap();

    let missing = empty.path().join("does-not-exist");
    let failed = Scenario::new(FileApp::new(missing.clone()), 720, 480);
    let status = format!("Could not open {}:", missing.display());
    assert!(failed.host().accessibility_nodes().iter().any(|node| {
        node.label
            .as_deref()
            .is_some_and(|name| name.starts_with(&status))
    }));
    failed.assert_accessibility().unwrap();
}

#[test]
fn scenario_resource_lifecycle_releases_build_scratch_and_suspend_state() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("report.txt"), b"x").unwrap();
    let mut scenario = Scenario::new(FileApp::new(directory.path().to_path_buf()), 860, 620);
    let before = scenario.host().inspect().resources;
    assert_eq!(before.retained_build_scratch_bytes, 0);
    assert!(before.node_count > 0 && before.accessibility_node_count > 0);
    scenario.window_focus(false).unwrap();
    scenario.platform_capability("controller", false).unwrap();
    scenario.suspend().unwrap();
    let after = scenario.host().inspect();
    assert!(after.keyboard_focus.is_none());
    assert!(after.controller_target.is_none());
    assert!(after.pointer_capture.is_none());
    assert_eq!(after.resources.retained_build_scratch_bytes, 0);
}

#[test]
fn focused_selection_is_visually_distinct_and_survives_window_focus_loss() {
    let mut scenario = Scenario::new(FileApp::fixture(), 820, 620);
    let entry = entry_selector(&scenario, "/file-entry-0");
    scenario.pointer_activate(&entry).unwrap();
    for _ in 0..64 {
        if scenario.host().inspect().keyboard_focus.as_ref()
            == match &entry {
                Selector::Id(id) => Some(id),
                _ => None,
            }
        {
            break;
        }
        scenario.keyboard_focus(FocusDirection::Next).unwrap();
    }
    assert_eq!(
        scenario.host().application().selected_entries,
        HashSet::from([0])
    );
    let focused = nickel_ui_testkit::render_host(scenario.host(), 820, 620, 1.0);

    scenario.window_focus(false).unwrap();
    assert_eq!(
        scenario.host().application().selected_entries,
        HashSet::from([0])
    );
    let unfocused = nickel_ui_testkit::render_host(scenario.host(), 820, 620, 1.0);
    assert_ne!(focused, unfocused);
}
