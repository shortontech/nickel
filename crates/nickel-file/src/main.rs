use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

use crate::{
    host::FileHostAdapter,
    icons, layout,
    layout::{rect_between, visible_file_range},
    platform::{home_directory, open_path},
};
use nickel_core::{
    shell_settings::{FileIconPreference, ShellSettings},
    theme::{Appearance, ThemeMode, ThemePalette},
};
use nickel_file::{DirectoryBrowser, EntrySortKey, FileEntry, SortDirection};
use nickel_ui::{
    AnyView, Application, FrameOverlay, Insets, OverlayAnchor, OverlayMenu, OverlayMenuItem, Point,
    ReadingDirection, UiId, ViewContext,
};

const DEFAULT_SIDEBAR_WIDTH: f32 = 190.0;
pub(crate) const MIN_SIDEBAR_WIDTH: f32 = 150.0;
pub(crate) const MAX_SIDEBAR_WIDTH: f32 = 360.0;
pub(crate) const SIDEBAR_RESIZE_WIDTH: f32 = 5.0;
pub(crate) const TOOLBAR_HEIGHT: f32 = 78.0;
const DEFAULT_TILE_WIDTH: f32 = 150.0;
pub(crate) const NARROW_WORKSPACE_BREAKPOINT: f32 = 720.0;
pub(crate) const MIN_TILE_WIDTH: f32 = 110.0;
pub(crate) const MAX_TILE_WIDTH: f32 = 240.0;
const MIN_DETAILS_COLUMN_WIDTH: f32 = 72.0;
const MAX_DETAILS_COLUMN_WIDTH: f32 = 320.0;
type NavigationResult = (u64, Result<Option<DirectoryBrowser>, String>);

#[derive(Clone, Debug, PartialEq)]
pub enum FileMessage {
    ContextEntry(usize),
    ContextBackground,
    ContextOpen,
    ContextOpenNewTab,
    ContextRefresh,
    ContextSelectAll,
    ToggleCommandSurface,
    CommandQueryChanged(String),
    ToggleAddressEditing,
    AddressChanged(String),
    SubmitAddress,
    ToggleHiddenFiles,
    ResizeSidebar,
    ResizeDetailsColumn(DetailsColumn),
    ToggleLocationGroup(String),
    TogglePlaces,
    NewTab,
    SwitchTab(usize),
    CloseTab(usize),
    Back,
    Forward,
    Up,
    Refresh,
    SetViewMode(FileViewMode),
    SortBy(EntrySortKey),
    Breadcrumb(PathBuf),
    ToggleFolder(PathBuf),
    OpenFolder(PathBuf),
    Entry(usize),
    SelectionSurface,
    FileScroll(f32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileViewMode {
    Grid,
    Details,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetailsColumn {
    Type,
    Modified,
    Size,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DetailsColumnWidths {
    pub(crate) type_width: f32,
    pub(crate) modified_width: f32,
    pub(crate) size_width: f32,
}

impl Default for DetailsColumnWidths {
    fn default() -> Self {
        Self {
            type_width: 100.0,
            modified_width: 140.0,
            size_width: 80.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct DetailsColumnResize {
    column: DetailsColumn,
    pointer_start: f32,
    width_start: f32,
}

pub struct FileApp {
    pub(crate) browser: DirectoryBrowser,
    pub(crate) cursor: Point,
    pub(crate) selected: Option<usize>,
    pub(crate) selected_entries: HashSet<usize>,
    pub(crate) selection_anchor: Option<usize>,
    pub(crate) active_tab_id: u64,
    pub(crate) next_tab_id: u64,
    pub(crate) status: String,
    pub(crate) last_click: Option<(usize, Instant)>,
    pub(crate) icons: icons::ArtworkCache,
    pub(crate) icon_rx:
        Option<Receiver<(u64, PathBuf, icons::ArtworkCacheKey, icons::ResolvedArtwork)>>,
    pub(crate) icon_poll_delay: std::time::Duration,
    pub(crate) icon_generation: u64,
    navigation_rx: Option<Receiver<NavigationResult>>,
    navigation_generation: u64,
    navigation_poll_delay: Duration,
    navigation_closes_address: bool,
    navigation_invalidates_icons: bool,
    pub(crate) icon_preference: FileIconPreference,
    pub(crate) icon_theme: Option<String>,
    pub(crate) next_icon_id: u16,
    pub(crate) sidebar_width: f32,
    pub(crate) expanded_folders: HashSet<PathBuf>,
    pub(crate) collapsed_location_groups: HashSet<String>,
    pub(crate) control_down: bool,
    pub(crate) shift_down: bool,
    pub(crate) selection_drag: Option<Point>,
    pub(crate) resizing_sidebar: bool,
    pub(crate) resizing_details_column: Option<DetailsColumnResize>,
    pub(crate) places_open: bool,
    pub(crate) command_surface_open: bool,
    pub(crate) command_query: String,
    pub(crate) pending_focus: Option<UiId>,
    pub(crate) address_editing: bool,
    pub(crate) address_text: String,
    pub(crate) tile_width: f32,
    pub(crate) file_scroll_offset: f32,
    pub(crate) view_mode: FileViewMode,
    pub(crate) sort_key: EntrySortKey,
    pub(crate) sort_direction: SortDirection,
    pub(crate) details_column_widths: DetailsColumnWidths,
    pub(crate) tab_icon: Option<(u16, Arc<image::RgbaImage>)>,
    pub(crate) tabs: Vec<Option<FileTab>>,
    pub(crate) active_tab: usize,
    pub(crate) pending_ensure_visible: bool,
    pub(crate) resolved_grid_columns: usize,
    pub(crate) exit_requested: bool,
    pub(crate) fixture_appearance: Option<Appearance>,
    pub(crate) fixture_navigation_busy: bool,
    pub(crate) reading_direction: ReadingDirection,
}

pub struct FileFixtureProvider;

pub struct FileWorkbenchFixture;

macro_rules! file_fixture_variant {
    ($id:literal, $title:literal, $viewport:literal, $width:literal, $height:literal, $theme:ident) => {
        nickel_ui_testkit::FixtureVariant {
            id: $id,
            title: $title,
            viewport: nickel_ui_testkit::ViewportPreset {
                id: $viewport,
                width: $width,
                height: $height,
            },
            theme: nickel_ui_testkit::FixtureTheme::$theme,
            locale: nickel_ui_testkit::DEFAULT_LOCALE,
            scale: nickel_ui_testkit::DEFAULT_SCALE,
            controller_family: nickel_ui::ControllerFamily::Generic,
            accessibility: nickel_ui_testkit::DEFAULT_ACCESSIBILITY,
        }
    };
}

const FILE_FIXTURE_VARIANTS: &[nickel_ui_testkit::FixtureVariant] = &[
    file_fixture_variant!("wide-grid-dark", "Wide Grid Dark", "wide", 1100, 700, Dark),
    file_fixture_variant!(
        "wide-grid-light",
        "Wide Grid Light",
        "wide",
        1100,
        700,
        Light
    ),
    file_fixture_variant!(
        "wide-details-dark",
        "Wide Details Dark",
        "wide",
        1100,
        700,
        Dark
    ),
    file_fixture_variant!(
        "wide-details-light",
        "Wide Details Light",
        "wide",
        1100,
        700,
        Light
    ),
    file_fixture_variant!(
        "medium-grid-dark",
        "Medium Grid Dark",
        "medium",
        820,
        620,
        Dark
    ),
    file_fixture_variant!(
        "medium-grid-light",
        "Medium Grid Light",
        "medium",
        820,
        620,
        Light
    ),
    file_fixture_variant!(
        "medium-details-dark",
        "Medium Details Dark",
        "medium",
        820,
        620,
        Dark
    ),
    file_fixture_variant!(
        "medium-details-light",
        "Medium Details Light",
        "medium",
        820,
        620,
        Light
    ),
    file_fixture_variant!(
        "narrow-grid-dark",
        "Narrow Grid Dark",
        "narrow",
        660,
        640,
        Dark
    ),
    file_fixture_variant!(
        "narrow-grid-light",
        "Narrow Grid Light",
        "narrow",
        660,
        640,
        Light
    ),
    file_fixture_variant!(
        "narrow-details-dark",
        "Narrow Details Dark",
        "narrow",
        660,
        640,
        Dark
    ),
    file_fixture_variant!(
        "narrow-details-light",
        "Narrow Details Light",
        "narrow",
        660,
        640,
        Light
    ),
    file_fixture_variant!(
        "long-unicode",
        "Long and Unicode Names",
        "wide",
        1100,
        700,
        Dark
    ),
    file_fixture_variant!(
        "hidden-selection",
        "Hidden Multi-selection",
        "wide",
        1100,
        700,
        Light
    ),
    file_fixture_variant!("empty", "Empty Folder", "medium", 820, 620, Dark),
    file_fixture_variant!(
        "unreadable",
        "Unreadable Location",
        "medium",
        820,
        620,
        Dark
    ),
    file_fixture_variant!(
        "unavailable",
        "Unavailable Location",
        "medium",
        820,
        620,
        Light
    ),
    file_fixture_variant!("loading", "Loading Location", "medium", 820, 620, Dark),
    file_fixture_variant!(
        "disconnected",
        "Disconnected Location",
        "medium",
        820,
        620,
        Light
    ),
    nickel_ui_testkit::FixtureVariant {
        id: "rtl-grid",
        title: "RTL Grid",
        viewport: nickel_ui_testkit::ViewportPreset {
            id: "wide",
            width: 1100,
            height: 700,
        },
        theme: nickel_ui_testkit::FixtureTheme::Dark,
        locale: nickel_ui_testkit::LocalePreset {
            id: "ar",
            direction: nickel_ui_testkit::FixtureDirection::RightToLeft,
        },
        scale: nickel_ui_testkit::DEFAULT_SCALE,
        controller_family: nickel_ui::ControllerFamily::Generic,
        accessibility: nickel_ui_testkit::DEFAULT_ACCESSIBILITY,
    },
    nickel_ui_testkit::FixtureVariant {
        id: "medium-details-125",
        title: "Medium Details 125%",
        viewport: nickel_ui_testkit::ViewportPreset {
            id: "medium",
            width: 1025,
            height: 775,
        },
        theme: nickel_ui_testkit::FixtureTheme::Light,
        locale: nickel_ui_testkit::DEFAULT_LOCALE,
        scale: nickel_ui_testkit::ScalePreset {
            id: "1.25x",
            factor: 1.25,
        },
        controller_family: nickel_ui::ControllerFamily::Generic,
        accessibility: nickel_ui_testkit::DEFAULT_ACCESSIBILITY,
    },
    nickel_ui_testkit::FixtureVariant {
        id: "narrow-200",
        title: "Narrow 200%",
        viewport: nickel_ui_testkit::ViewportPreset {
            id: "narrow",
            width: 960,
            height: 720,
        },
        theme: nickel_ui_testkit::FixtureTheme::Dark,
        locale: nickel_ui_testkit::DEFAULT_LOCALE,
        scale: nickel_ui_testkit::ScalePreset {
            id: "2x",
            factor: 2.0,
        },
        controller_family: nickel_ui::ControllerFamily::Generic,
        accessibility: nickel_ui_testkit::DEFAULT_ACCESSIBILITY,
    },
];

static FILE_FIXTURE_METADATA: nickel_ui_testkit::FixtureMetadata =
    nickel_ui_testkit::FixtureMetadata {
        id: "file.browser",
        title: "Nickel File",
        description: "Production Nickel File browser surface",
        tags: &["file", "browser", "collection", "context-menu"],
        source: nickel_ui_testkit::FixtureSource {
            crate_name: "nickel-file",
            file: file!(),
            line: line!(),
        },
        variants: FILE_FIXTURE_VARIANTS,
        assets: FILE_FIXTURE_ASSETS,
        simulated_effects: &[],
    };

impl nickel_ui_testkit::Fixture for FileWorkbenchFixture {
    type App = FileApp;
    fn metadata() -> &'static nickel_ui_testkit::FixtureMetadata {
        &FILE_FIXTURE_METADATA
    }
    fn create() -> Self::App {
        FileApp::fixture()
    }
    fn create_variant(variant: &nickel_ui_testkit::FixtureVariant) -> Self::App {
        let mut app = match variant.id {
            "long-unicode" => FileApp::with_browser(
                DirectoryBrowser::fixture(vec![
                    FileEntry {
                        name: "Quarterly planning notes with a deliberately long wrapped name.md"
                            .into(),
                        path: PathBuf::from(
                            "/fixture/Quarterly planning notes with a deliberately long wrapped name.md",
                        ),
                        is_directory: false,
                        size: Some(8_192),
                        modified: None,
                    },
                    FileEntry {
                        name: "写真と音楽 🎵".into(),
                        path: PathBuf::from("/fixture/写真と音楽 🎵"),
                        is_directory: true,
                        size: None,
                        modified: None,
                    },
                    FileEntry {
                        name: "مرحبا.txt".into(),
                        path: PathBuf::from("/fixture/مرحبا.txt"),
                        is_directory: false,
                        size: Some(512),
                        modified: None,
                    },
                ]),
                String::new(),
            ),
            "hidden-selection" => {
                let mut app = FileApp::with_browser(
                    DirectoryBrowser::fixture(vec![
                        FileEntry {
                            name: ".nickel-cache".into(),
                            path: PathBuf::from("/fixture/.nickel-cache"),
                            is_directory: true,
                            size: None,
                            modified: None,
                        },
                        FileEntry {
                            name: "report.txt".into(),
                            path: PathBuf::from("/fixture/report.txt"),
                            is_directory: false,
                            size: Some(128),
                            modified: None,
                        },
                        FileEntry {
                            name: "notes.md".into(),
                            path: PathBuf::from("/fixture/notes.md"),
                            is_directory: false,
                            size: Some(512),
                            modified: None,
                        },
                    ]),
                    String::new(),
                );
                app.selected = Some(1);
                app.selection_anchor = Some(0);
                app.selected_entries = HashSet::from([0, 1]);
                app
            }
            "empty" => FileApp::with_browser(DirectoryBrowser::fixture(Vec::new()), String::new()),
            _ => FileApp::fixture(),
        };
        if variant.id.contains("details") {
            app.view_mode = FileViewMode::Details;
        }
        if variant.id == "unavailable" {
            app.status = "Location unavailable — reconnect the volume and refresh.".into();
        }
        if variant.id == "unreadable" {
            app.status = "Could not read location — check its permissions and refresh.".into();
        }
        if variant.id == "loading" {
            app.status = "Loading network location…".into();
            app.fixture_navigation_busy = true;
        }
        if variant.id == "disconnected" {
            app.status = "Network location disconnected — reconnect and refresh.".into();
        }
        app.fixture_appearance = Some(Appearance {
            mode: match variant.theme {
                nickel_ui_testkit::FixtureTheme::Light => ThemeMode::Light,
                nickel_ui_testkit::FixtureTheme::Dark
                | nickel_ui_testkit::FixtureTheme::HighContrast => ThemeMode::Dark,
            },
            accent: [0, 164, 96],
            intensity: 100,
        });
        app.reading_direction = match variant.locale.direction {
            nickel_ui_testkit::FixtureDirection::LeftToRight => ReadingDirection::LeftToRight,
            nickel_ui_testkit::FixtureDirection::RightToLeft => ReadingDirection::RightToLeft,
        };
        app
    }
    fn surface_size() -> (u32, u32) {
        (960, 640)
    }
    fn default_activation() -> Option<nickel_ui_testkit::Selector> {
        Some(nickel_ui_testkit::Selector::role_name(
            nickel_ui::SemanticRole::Button,
            "report.txt",
        ))
    }
}

const FILE_FIXTURE_ASSETS: &[nickel_ui_testkit::FixtureAsset] = &[
    nickel_ui_testkit::FixtureAsset {
        id: "nickel-file-folder",
        path: "assets/concepts/nickel-file-icon-family/folder.png",
        license: "Same license as Nickel",
        sha256: "befa4351e2f22c200f07103d4b1c2f51de4303e0da9c3a7352fdbeec05066ec2",
    },
    nickel_ui_testkit::FixtureAsset {
        id: "nickel-file-home-folder",
        path: "assets/concepts/nickel-file-icon-family/home-folder.png",
        license: "Same license as Nickel",
        sha256: "2c492d54307371cc58255d3f2cc58e465e14a7959a11b0ca13f5cb1e5181e21c",
    },
    nickel_ui_testkit::FixtureAsset {
        id: "nickel-file-pictures-folder",
        path: "assets/concepts/nickel-file-icon-family/pictures-folder.png",
        license: "Same license as Nickel",
        sha256: "b4477db1170bae282f22c38099e3327fbdc0e680c8c652b62177e1b57684cc77",
    },
    nickel_ui_testkit::FixtureAsset {
        id: "nickel-file-music-folder",
        path: "assets/concepts/nickel-file-icon-family/music-folder.png",
        license: "Same license as Nickel",
        sha256: "667c2b11bcb2351e6e6476c1b761d4d04b268a02f1af604aff7c2229c10c24ee",
    },
    nickel_ui_testkit::FixtureAsset {
        id: "nickel-file-image-file",
        path: "assets/concepts/nickel-file-icon-family/image-file.png",
        license: "Same license as Nickel",
        sha256: "7ad8b1935c1bb774f41a9e47e0e75e29639310d613fef08f651185ec1c478056",
    },
    nickel_ui_testkit::FixtureAsset {
        id: "nickel-file-text-file",
        path: "assets/concepts/nickel-file-icon-family/text-file.png",
        license: "Same license as Nickel",
        sha256: "e6bd3410eb2b294b2f3fc600271f35d8ef6cbd9c2182add560bdc584ac539e92",
    },
];

impl nickel_ui_testkit::FixtureProvider for FileFixtureProvider {
    fn register(
        &self,
        registry: &mut nickel_ui_testkit::FixtureRegistry,
    ) -> Result<(), nickel_ui_testkit::RegistryError> {
        registry.register::<FileWorkbenchFixture>()
    }
}

pub(crate) struct FileTab {
    pub(crate) browser: DirectoryBrowser,
    pub(crate) selected: Option<usize>,
    pub(crate) selected_entries: HashSet<usize>,
    pub(crate) selection_anchor: Option<usize>,
    pub(crate) tab_id: u64,
    pub(crate) status: String,
    pub(crate) last_click: Option<(usize, Instant)>,
    pub(crate) file_scroll_offset: f32,
    pub(crate) view_mode: FileViewMode,
    pub(crate) sort_key: EntrySortKey,
    pub(crate) sort_direction: SortDirection,
    pub(crate) details_column_widths: DetailsColumnWidths,
    pub(crate) address_editing: bool,
    pub(crate) address_text: String,
    pub(crate) tab_icon: Option<(u16, Arc<image::RgbaImage>)>,
    navigation_rx: Option<Receiver<NavigationResult>>,
    navigation_generation: u64,
    navigation_poll_delay: Duration,
    navigation_closes_address: bool,
    navigation_invalidates_icons: bool,
}

impl FileApp {
    pub(crate) fn is_resizing_sidebar(&self) -> bool {
        self.resizing_sidebar
    }

    pub(crate) fn is_resizing_details_column(&self) -> bool {
        self.resizing_details_column.is_some()
    }

    pub(crate) fn navigation_pending(&self) -> bool {
        self.navigation_rx.is_some() || self.fixture_navigation_busy
    }

    pub(crate) fn resize_details_column_to(&mut self, pointer_x: f32) {
        let Some(resize) = self.resizing_details_column else {
            return;
        };
        let width = (resize.width_start + pointer_x - resize.pointer_start)
            .clamp(MIN_DETAILS_COLUMN_WIDTH, MAX_DETAILS_COLUMN_WIDTH);
        match resize.column {
            DetailsColumn::Type => self.details_column_widths.type_width = width,
            DetailsColumn::Modified => self.details_column_widths.modified_width = width,
            DetailsColumn::Size => self.details_column_widths.size_width = width,
        }
    }

    fn begin_details_column_resize(&mut self, column: DetailsColumn) {
        let width_start = match column {
            DetailsColumn::Type => self.details_column_widths.type_width,
            DetailsColumn::Modified => self.details_column_widths.modified_width,
            DetailsColumn::Size => self.details_column_widths.size_width,
        };
        self.resizing_details_column = Some(DetailsColumnResize {
            column,
            pointer_start: self.cursor.x,
            width_start,
        });
    }

    pub fn new(path: PathBuf) -> Self {
        let show_hidden = nickel_platform::show_hidden_files();
        let (browser, status) = match DirectoryBrowser::open_with_hidden(&path, show_hidden) {
            Ok(browser) => (browser, String::new()),
            Err(error) => {
                let home = home_directory();
                let browser = DirectoryBrowser::open_with_hidden(&home, show_hidden)
                    .unwrap_or_else(|_| {
                        DirectoryBrowser::open_with_hidden(".", show_hidden)
                            .expect("open a directory")
                    });
                (
                    browser,
                    format!("Could not open {}: {error}", path.display()),
                )
            }
        };
        Self::with_browser(browser, status)
    }

    fn with_browser(browser: DirectoryBrowser, status: String) -> Self {
        let mut app = Self {
            browser,
            cursor: Point { x: 0.0, y: 0.0 },
            selected: None,
            selected_entries: HashSet::new(),
            selection_anchor: None,
            active_tab_id: 0,
            next_tab_id: 1,
            status,
            last_click: None,
            icons: icons::ArtworkCache::default(),
            icon_rx: None,
            icon_poll_delay: std::time::Duration::from_millis(16),
            icon_generation: 0,
            navigation_rx: None,
            navigation_generation: 0,
            navigation_poll_delay: Duration::from_millis(16),
            navigation_closes_address: false,
            navigation_invalidates_icons: false,
            icon_preference: ShellSettings::load_default().file_icon_provider,
            icon_theme: ShellSettings::load_default().file_icon_theme,
            next_icon_id: 1,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            expanded_folders: HashSet::new(),
            collapsed_location_groups: HashSet::new(),
            control_down: false,
            shift_down: false,
            selection_drag: None,
            resizing_sidebar: false,
            resizing_details_column: None,
            places_open: false,
            command_surface_open: false,
            command_query: String::new(),
            pending_focus: None,
            address_editing: false,
            address_text: String::new(),
            tile_width: DEFAULT_TILE_WIDTH,
            file_scroll_offset: 0.0,
            view_mode: FileViewMode::Grid,
            sort_key: EntrySortKey::Name,
            sort_direction: SortDirection::Ascending,
            details_column_widths: DetailsColumnWidths::default(),
            tab_icon: None,
            tabs: vec![None],
            active_tab: 0,
            pending_ensure_visible: false,
            resolved_grid_columns: 1,
            exit_requested: false,
            fixture_appearance: None,
            fixture_navigation_busy: false,
            reading_direction: if nickel_i18n::Localizer::system().is_right_to_left() {
                ReadingDirection::RightToLeft
            } else {
                ReadingDirection::LeftToRight
            },
        };
        app.refresh_icons();
        app
    }

    fn fixture() -> Self {
        let entries = [
            ("Documents", true, None),
            ("report.txt", false, Some(128)),
            ("notes.md", false, Some(512)),
        ]
        .into_iter()
        .map(|(name, is_directory, size)| FileEntry {
            name: name.into(),
            path: PathBuf::from("/fixture").join(name),
            is_directory,
            size,
            modified: None,
        })
        .collect();
        let mut app = Self::with_browser(DirectoryBrowser::fixture(entries), String::new());
        app.icon_rx = None;
        app.icons.clear();
        for entry in app.browser.entries() {
            let artwork = icons::resolve_artwork(
                FileIconPreference::Nickel,
                &icons::ArtworkRequest {
                    path: &entry.path,
                    kind: icons::semantic_kind(&entry.path, entry.is_directory),
                    logical_size: 96,
                    scale_milli: 1_000,
                    appearance: icons::ArtworkAppearance::Dark,
                },
            );
            let id = app.next_icon_id;
            app.next_icon_id += 1;
            app.icons.insert(entry.path.clone(), (id, artwork.pixels));
        }
        app
    }

    fn set_scroll_offset(&mut self, offset: f32) {
        self.file_scroll_offset = offset.max(0.0);
    }

    pub(crate) fn inactive_tab(&self, index: usize) -> Option<&FileTab> {
        self.tabs.get(index).and_then(Option::as_ref)
    }

    fn switch_tab(&mut self, index: usize) {
        if index == self.active_tab || index >= self.tabs.len() {
            return;
        }
        let Some(mut target) = self.tabs[index].take() else {
            return;
        };
        std::mem::swap(&mut self.browser, &mut target.browser);
        std::mem::swap(&mut self.selected, &mut target.selected);
        std::mem::swap(&mut self.selected_entries, &mut target.selected_entries);
        std::mem::swap(&mut self.selection_anchor, &mut target.selection_anchor);
        std::mem::swap(&mut self.active_tab_id, &mut target.tab_id);
        std::mem::swap(&mut self.status, &mut target.status);
        std::mem::swap(&mut self.last_click, &mut target.last_click);
        std::mem::swap(&mut self.file_scroll_offset, &mut target.file_scroll_offset);
        std::mem::swap(&mut self.view_mode, &mut target.view_mode);
        std::mem::swap(&mut self.sort_key, &mut target.sort_key);
        std::mem::swap(&mut self.sort_direction, &mut target.sort_direction);
        std::mem::swap(
            &mut self.details_column_widths,
            &mut target.details_column_widths,
        );
        std::mem::swap(&mut self.address_editing, &mut target.address_editing);
        std::mem::swap(&mut self.address_text, &mut target.address_text);
        std::mem::swap(&mut self.tab_icon, &mut target.tab_icon);
        std::mem::swap(&mut self.navigation_rx, &mut target.navigation_rx);
        std::mem::swap(
            &mut self.navigation_generation,
            &mut target.navigation_generation,
        );
        std::mem::swap(
            &mut self.navigation_poll_delay,
            &mut target.navigation_poll_delay,
        );
        std::mem::swap(
            &mut self.navigation_closes_address,
            &mut target.navigation_closes_address,
        );
        std::mem::swap(
            &mut self.navigation_invalidates_icons,
            &mut target.navigation_invalidates_icons,
        );
        self.tabs[self.active_tab] = Some(target);
        self.active_tab = index;
        self.refresh_icons();
    }

    fn new_tab(&mut self) {
        self.new_tab_at(home_directory());
    }

    fn new_tab_at(&mut self, path: PathBuf) {
        let show_hidden = nickel_platform::show_hidden_files();
        let Ok(browser) = DirectoryBrowser::open_with_hidden(path, show_hidden) else {
            return;
        };
        let tab_id = self.next_tab_id;
        self.next_tab_id = self.next_tab_id.saturating_add(1);
        self.tabs.push(Some(FileTab {
            browser,
            selected: None,
            selected_entries: HashSet::new(),
            selection_anchor: None,
            tab_id,
            status: String::new(),
            last_click: None,
            file_scroll_offset: 0.0,
            view_mode: FileViewMode::Grid,
            sort_key: EntrySortKey::Name,
            sort_direction: SortDirection::Ascending,
            details_column_widths: DetailsColumnWidths::default(),
            address_editing: false,
            address_text: String::new(),
            tab_icon: None,
            navigation_rx: None,
            navigation_generation: 0,
            navigation_poll_delay: Duration::from_millis(16),
            navigation_closes_address: false,
            navigation_invalidates_icons: false,
        }));
        self.switch_tab(self.tabs.len() - 1);
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            self.exit_requested = true;
            return;
        }
        if index != self.active_tab {
            self.tabs.remove(index);
            if index < self.active_tab {
                self.active_tab -= 1;
            }
            return;
        }
        let target = if index + 1 < self.tabs.len() {
            index + 1
        } else {
            index - 1
        };
        self.switch_tab(target);
        self.tabs.remove(index);
        if index < self.active_tab {
            self.active_tab -= 1;
        }
    }

    fn build_view(
        &self,
        _width: f32,
        height: f32,
        palette: ThemePalette,
        light_mode: bool,
    ) -> AnyView<FileMessage> {
        layout::build_view(self, _width, height, palette, light_mode)
    }
    fn activate_selected(&mut self) {
        let Some(index) = self.selected else {
            return;
        };
        let Some(entry) = self.browser.entries().get(index).cloned() else {
            return;
        };
        let is_directory = entry.is_directory || entry.path.is_dir();
        if is_directory && !entry.is_directory {
            tracing::warn!(
                path = %entry.path.display(),
                "entry model missed a directory; using activation-time check"
            );
        }
        if is_directory {
            self.navigate_to(entry.path);
        } else if let Err(error) = open_path(&entry.path) {
            self.status = format!("Could not open {}: {error}", entry.display_name());
        }
    }

    fn navigate_to(&mut self, path: PathBuf) {
        let label = path.display().to_string();
        self.start_navigation(
            format!("Opening {label}"),
            "Could not open location",
            false,
            move |browser| {
                browser
                    .enter(path)
                    .map(|()| true)
                    .map_err(|error| error.to_string())
            },
        );
    }

    pub(crate) fn go_back(&mut self) {
        if self.browser.can_go_back() {
            self.start_navigation("Going back".into(), "Could not go back", false, |browser| {
                browser.back().map_err(|error| error.to_string())
            });
        }
    }

    fn go_forward(&mut self) {
        if self.browser.can_go_forward() {
            self.start_navigation(
                "Going forward".into(),
                "Could not go forward",
                false,
                |browser| browser.forward().map_err(|error| error.to_string()),
            );
        }
    }

    fn go_up(&mut self) {
        if self.browser.can_go_up() {
            self.start_navigation(
                "Opening parent".into(),
                "Could not open parent",
                false,
                |browser| browser.up().map_err(|error| error.to_string()),
            );
        }
    }

    fn start_navigation(
        &mut self,
        progress: String,
        error_context: &'static str,
        closes_address: bool,
        operation: impl FnOnce(&mut DirectoryBrowser) -> Result<bool, String> + Send + 'static,
    ) {
        self.navigation_generation = self.navigation_generation.wrapping_add(1);
        let generation = self.navigation_generation;
        let mut browser = self.browser.clone();
        let (sender, receiver) = mpsc::channel();
        self.navigation_rx = Some(receiver);
        self.navigation_poll_delay = Duration::from_millis(16);
        self.navigation_closes_address = closes_address;
        self.navigation_invalidates_icons = false;
        self.status = format!("{progress}…");
        let _ = std::thread::Builder::new()
            .name("nickel-file-navigation".into())
            .spawn(move || {
                let result = operation(&mut browser)
                    .map(|changed| changed.then_some(browser))
                    .map_err(|error| format!("{error_context}: {error}"));
                let _ = sender.send((generation, result));
            });
    }

    fn refresh_directory(&mut self, show_hidden: bool) {
        self.start_navigation(
            "Refreshing".into(),
            "Could not refresh",
            false,
            move |browser| {
                browser
                    .set_show_hidden(show_hidden)
                    .map(|()| true)
                    .map_err(|error| error.to_string())
            },
        );
        self.navigation_invalidates_icons = true;
    }

    fn poll_navigation(&mut self) -> bool {
        let result = match self.navigation_rx.as_ref() {
            Some(receiver) => receiver.try_recv(),
            None => return false,
        };
        match result {
            Ok((generation, result)) if generation == self.navigation_generation => {
                self.navigation_rx = None;
                match result {
                    Ok(Some(browser)) => {
                        self.browser = browser;
                        if self.navigation_invalidates_icons {
                            self.icons.clear();
                            self.tab_icon = None;
                        }
                        self.navigation_invalidates_icons = false;
                        if self.navigation_closes_address {
                            self.address_editing = false;
                            self.address_text.clear();
                        }
                        self.navigation_changed();
                    }
                    Ok(None) => {
                        self.navigation_invalidates_icons = false;
                        self.status.clear();
                    }
                    Err(error) => {
                        self.navigation_invalidates_icons = false;
                        self.status = error;
                    }
                }
                true
            }
            Ok(_) => true,
            Err(TryRecvError::Empty) => {
                self.navigation_poll_delay = self
                    .navigation_poll_delay
                    .saturating_mul(2)
                    .min(Duration::from_millis(250));
                false
            }
            Err(TryRecvError::Disconnected) => {
                self.navigation_rx = None;
                self.navigation_invalidates_icons = false;
                self.status = "Could not open location: navigation worker stopped".into();
                true
            }
        }
    }

    fn navigation_changed(&mut self) {
        self.browser.sort(self.sort_key, self.sort_direction);
        self.selected = None;
        self.selected_entries.clear();
        self.selection_anchor = None;
        self.set_scroll_offset(0.0);
        self.status.clear();
        self.refresh_icons();
    }

    fn sort_by(&mut self, key: EntrySortKey) {
        let focused = self
            .selected
            .and_then(|index| self.browser.entries().get(index))
            .map(|entry| entry.path.clone());
        let selected = self
            .selected_entries
            .iter()
            .filter_map(|index| self.browser.entries().get(*index))
            .map(|entry| entry.path.clone())
            .collect::<HashSet<_>>();
        if self.sort_key == key {
            self.sort_direction = match self.sort_direction {
                SortDirection::Ascending => SortDirection::Descending,
                SortDirection::Descending => SortDirection::Ascending,
            };
        } else {
            self.sort_key = key;
            self.sort_direction = SortDirection::Ascending;
        }
        self.browser.sort(self.sort_key, self.sort_direction);
        self.selected_entries = self
            .browser
            .entries()
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| selected.contains(&entry.path).then_some(index))
            .collect();
        self.selected = focused.and_then(|path| {
            self.browser
                .entries()
                .iter()
                .position(|entry| entry.path == path)
        });
        self.selection_anchor = self.selected;
        self.set_scroll_offset(0.0);
        self.ensure_selection_visible();
    }

    pub(crate) fn submit_address(&mut self) {
        let target = PathBuf::from(self.address_text.trim());
        if target.as_os_str().is_empty() {
            self.status = "Enter a location.".into();
            return;
        }
        let label = target.display().to_string();
        self.start_navigation(
            format!("Opening {label}"),
            "Could not open location",
            true,
            move |browser| {
                browser
                    .enter(target)
                    .map(|()| true)
                    .map_err(|error| error.to_string())
            },
        );
    }

    pub(crate) fn refresh_icons(&mut self) {
        let settings = ShellSettings::load_default();
        let preference = settings.file_icon_provider;
        let appearance = settings
            .resolve_appearance(nickel_platform::appearance())
            .mode;
        self.refresh_icons_for_theme(preference, settings.file_icon_theme.as_deref(), appearance);
    }

    pub(crate) fn refresh_icons_for(
        &mut self,
        preference: FileIconPreference,
        appearance: ThemeMode,
    ) {
        self.refresh_icons_for_theme(preference, None, appearance);
    }

    fn refresh_icons_for_theme(
        &mut self,
        preference: FileIconPreference,
        theme: Option<&str>,
        appearance: ThemeMode,
    ) {
        if preference != self.icon_preference || self.icon_theme.as_deref() != theme {
            self.icon_preference = preference;
            self.icon_theme = theme.map(str::to_owned);
            self.icons.clear();
            self.tab_icon = None;
        }
        self.icon_generation = self.icon_generation.wrapping_add(1);
        let generation = self.icon_generation;
        let artwork_appearance = if appearance == ThemeMode::Light {
            icons::ArtworkAppearance::Light
        } else {
            icons::ArtworkAppearance::Dark
        };
        let entries = self
            .browser
            .entries()
            .iter()
            .map(|entry| {
                let request = icons::ArtworkRequest {
                    path: &entry.path,
                    kind: icons::semantic_kind(&entry.path, entry.is_directory),
                    logical_size: 96,
                    scale_milli: 1_000,
                    appearance: artwork_appearance,
                };
                (
                    entry.path.clone(),
                    entry.is_directory,
                    icons::cache_key_with_theme(preference, self.icon_theme.as_deref(), &request),
                )
            })
            .collect::<Vec<_>>();
        self.icons
            .retain(|path| entries.iter().any(|(entry, _, _)| entry == path));
        let mut paths = entries
            .into_iter()
            .filter(|(_, _, key)| !self.icons.matches(key))
            .collect::<Vec<_>>();
        let current = self.browser.current().to_path_buf();
        let current_request = icons::ArtworkRequest {
            path: &current,
            kind: icons::semantic_kind(&current, true),
            logical_size: 96,
            scale_milli: 1_000,
            appearance: artwork_appearance,
        };
        let current_key =
            icons::cache_key_with_theme(preference, self.icon_theme.as_deref(), &current_request);
        paths.push((current, true, current_key));

        // Nickel artwork is the guaranteed first frame. A selected system
        // provider may replace it asynchronously, but native lookup must never
        // leave an invisible entry or tab target while it is pending.
        for (path, is_directory, key) in &paths {
            let artwork = icons::resolve_artwork(
                FileIconPreference::Nickel,
                &icons::ArtworkRequest {
                    path,
                    kind: icons::semantic_kind(path, *is_directory),
                    logical_size: 96,
                    scale_milli: 1_000,
                    appearance: artwork_appearance,
                },
            );
            let id = self.next_icon_id;
            self.next_icon_id = self.next_icon_id.checked_add(1).unwrap_or(1);
            if path == self.browser.current() {
                self.tab_icon = Some((id, artwork.pixels));
            } else {
                self.icons
                    .insert_resolved(key.clone(), (id, artwork.pixels));
            }
        }
        if preference == FileIconPreference::Nickel {
            self.icon_rx = None;
            return;
        }
        #[cfg(debug_assertions)]
        if std::env::var_os("NICKEL_FILE_PROFILE_ICONS").is_some() {
            eprintln!(
                "nickel-file icon-profile: fetch batch requested={} retained_cache={}",
                paths.len(),
                self.icons.len()
            );
        }
        let (tx, rx) = mpsc::channel();
        let icon_theme = self.icon_theme.clone();
        self.icon_rx = Some(rx);
        self.icon_poll_delay = std::time::Duration::from_millis(16);
        let _ = std::thread::Builder::new()
            .name("nickel-file-icons".into())
            .spawn(move || {
                for (path, is_directory, key) in paths {
                    #[cfg(debug_assertions)]
                    let profile_started = Instant::now();
                    let artwork = icons::resolve_artwork_with_theme(
                        preference,
                        icon_theme.as_deref(),
                        &icons::ArtworkRequest {
                            path: &path,
                            kind: icons::semantic_kind(&path, is_directory),
                            logical_size: 96,
                            scale_milli: 1_000,
                            appearance: artwork_appearance,
                        },
                    );
                    #[cfg(debug_assertions)]
                    if std::env::var_os("NICKEL_FILE_PROFILE_ICONS").is_some() {
                        let dimensions =
                            format!("{}x{}", artwork.pixels.width(), artwork.pixels.height());
                        eprintln!(
                            "nickel-file icon-profile: fetched path={} result={} time={:.2?}",
                            path.display(),
                            dimensions,
                            profile_started.elapsed()
                        );
                    }
                    if tx.send((generation, path, key, artwork)).is_err() {
                        break;
                    }
                }
            });
    }

    fn poll_icons(&mut self) {
        loop {
            let result = match self.icon_rx.as_ref() {
                Some(rx) => rx.try_recv(),
                None => return,
            };
            match result {
                Ok((generation, path, key, artwork)) if generation == self.icon_generation => {
                    self.icon_poll_delay = std::time::Duration::from_millis(16);
                    let id = self.next_icon_id;
                    self.next_icon_id = self.next_icon_id.checked_add(1).unwrap_or(1);
                    if path == self.browser.current() {
                        self.tab_icon = Some((id, artwork.pixels));
                    } else if self
                        .browser
                        .entries()
                        .iter()
                        .any(|entry| entry.path == path)
                    {
                        self.icons.insert_resolved(key, (id, artwork.pixels));
                    }
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => {
                    self.icon_poll_delay = self
                        .icon_poll_delay
                        .saturating_mul(2)
                        .min(std::time::Duration::from_millis(250));
                    return;
                }
                Err(TryRecvError::Disconnected) => {
                    self.icon_rx = None;
                    return;
                }
            }
        }
    }

    pub(crate) fn select_relative(&mut self, delta: isize) {
        let len = self.browser.entries().len();
        if len == 0 {
            self.selected = None;
            self.selected_entries.clear();
            self.selection_anchor = None;
            return;
        }
        let current = self.selected.unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, len as isize - 1) as usize;
        self.selected = Some(next);
        if self.shift_down {
            let anchor = self.selection_anchor.unwrap_or(current as usize);
            self.selected_entries.clear();
            self.selected_entries
                .extend(anchor.min(next)..=anchor.max(next));
        } else {
            self.selected_entries.clear();
            self.selected_entries.insert(next);
            self.selection_anchor = Some(next);
        }
        self.ensure_selection_visible();
    }

    pub(crate) fn ensure_selection_visible(&mut self) {
        self.pending_ensure_visible = self.selected.is_some();
    }

    pub(crate) fn resolved_grid_columns(&self) -> usize {
        self.resolved_grid_columns.max(1)
    }

    fn update_message(&mut self, message: FileMessage) {
        if self.command_surface_open
            && !matches!(
                message,
                FileMessage::ToggleCommandSurface | FileMessage::CommandQueryChanged(_)
            )
        {
            self.command_surface_open = false;
            self.command_query.clear();
        }
        match message {
            FileMessage::ContextEntry(index) => {
                if !self.selected_entries.contains(&index) {
                    self.selected_entries.clear();
                    self.selected_entries.insert(index);
                    self.selected = Some(index);
                    self.selection_anchor = Some(index);
                }
                self.selection_drag = None;
            }
            FileMessage::ContextBackground => {
                self.selection_drag = None;
            }
            FileMessage::ContextOpen => {
                self.activate_selected();
            }
            FileMessage::ContextOpenNewTab => {
                let entry = self
                    .selected
                    .and_then(|index| self.browser.entries().get(index))
                    .cloned();
                if let Some(entry) = entry {
                    if entry.is_directory || entry.path.is_dir() {
                        self.new_tab_at(entry.path);
                    } else if let Err(error) = open_path(&entry.path) {
                        self.status = format!("Could not open {}: {error}", entry.display_name());
                    }
                }
            }
            FileMessage::ContextRefresh => {
                self.refresh_directory(self.browser.show_hidden());
            }
            FileMessage::ContextSelectAll => {
                self.selected_entries = (0..self.browser.entries().len()).collect();
                self.selected = (!self.browser.entries().is_empty()).then_some(0);
                self.selection_anchor = self.selected;
            }
            FileMessage::ToggleCommandSurface => {
                self.command_surface_open = !self.command_surface_open;
                self.command_query.clear();
                if self.command_surface_open {
                    self.places_open = false;
                    self.pending_focus = Some(UiId::from("file-command-query"));
                }
            }
            FileMessage::CommandQueryChanged(query) => self.command_query = query,
            FileMessage::ToggleAddressEditing => {
                self.address_editing = !self.address_editing;
                if self.address_editing {
                    self.address_text = self.browser.current().display().to_string();
                    self.pending_focus = Some(UiId::from("file-address-field"));
                } else {
                    self.address_text.clear();
                }
            }
            FileMessage::AddressChanged(address) => self.address_text = address,
            FileMessage::SubmitAddress => self.submit_address(),
            FileMessage::ToggleHiddenFiles => {
                let show = !self.browser.show_hidden();
                self.start_navigation(
                    if show {
                        "Showing hidden files".into()
                    } else {
                        "Hiding hidden files".into()
                    },
                    "Could not change hidden files",
                    false,
                    move |browser| {
                        browser
                            .set_show_hidden(show)
                            .map(|()| true)
                            .map_err(|error| error.to_string())
                    },
                );
            }
            FileMessage::ResizeSidebar => {
                self.resizing_sidebar = true;
            }
            FileMessage::ResizeDetailsColumn(column) => {
                self.begin_details_column_resize(column);
            }
            FileMessage::ToggleLocationGroup(group) => {
                if !self.collapsed_location_groups.remove(&group) {
                    self.collapsed_location_groups.insert(group);
                }
            }
            FileMessage::TogglePlaces => self.places_open = !self.places_open,
            FileMessage::NewTab => self.new_tab(),
            FileMessage::Back => self.go_back(),
            FileMessage::Forward => self.go_forward(),
            FileMessage::Up => self.go_up(),
            FileMessage::Refresh => {
                self.refresh_directory(nickel_platform::show_hidden_files());
            }
            FileMessage::SetViewMode(mode) => {
                self.view_mode = mode;
                self.set_scroll_offset(0.0);
                self.ensure_selection_visible();
            }
            FileMessage::SortBy(key) => self.sort_by(key),
            FileMessage::CloseTab(index) => self.close_tab(index),
            FileMessage::SwitchTab(index) => self.switch_tab(index),
            FileMessage::ToggleFolder(path) => {
                if !self.expanded_folders.remove(&path) {
                    self.expanded_folders.insert(path);
                }
            }
            FileMessage::OpenFolder(path) | FileMessage::Breadcrumb(path) => {
                self.places_open = false;
                self.navigate_to(path);
            }
            FileMessage::Entry(index) => {
                let now = Instant::now();
                let activate = self.last_click.is_some_and(|(previous, when)| {
                    previous == index && now.duration_since(when) <= Duration::from_millis(450)
                });
                if self.shift_down {
                    let anchor = self.selection_anchor.unwrap_or(index);
                    self.selected_entries.clear();
                    self.selected_entries
                        .extend(anchor.min(index)..=anchor.max(index));
                } else if self.control_down {
                    if !self.selected_entries.remove(&index) {
                        self.selected_entries.insert(index);
                    }
                    self.selection_anchor = Some(index);
                } else {
                    self.selected_entries.clear();
                    self.selected_entries.insert(index);
                    self.selection_anchor = Some(index);
                }
                self.selected = Some(index);
                self.last_click = Some((index, now));
                if activate && !self.control_down && !self.shift_down {
                    self.activate_selected();
                    self.last_click = None;
                }
            }
            FileMessage::SelectionSurface => {
                self.selection_drag = Some(self.cursor);
                if !self.control_down && !self.shift_down {
                    self.selected = None;
                    self.selected_entries.clear();
                    self.selection_anchor = None;
                }
            }
            FileMessage::FileScroll(offset) => self.file_scroll_offset = offset.max(0.0),
        }
    }
}

impl Application for FileApp {
    type Message = FileMessage;

    fn update(&mut self, message: Self::Message) {
        self.update_message(message);
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let appearance = self.fixture_appearance.unwrap_or_else(|| {
            ShellSettings::load_default().resolve_appearance(nickel_platform::appearance())
        });
        self.build_view(
            context.viewport.size.width,
            context.viewport.size.height,
            ThemePalette::from_appearance(appearance),
            appearance.mode == ThemeMode::Light,
        )
    }

    fn take_focus_request(&mut self) -> Option<UiId> {
        self.pending_focus.take()
    }

    fn frame_overlays(&self, context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        let appearance =
            ShellSettings::load_default().resolve_appearance(nickel_platform::appearance());
        let palette = ThemePalette::from_appearance(appearance);
        let invocation_anchor = |target: UiId| match context.modality {
            nickel_ui::InputModality::Pointer => OverlayAnchor::Point {
                invocation_target: target,
                point: self.cursor,
            },
            nickel_ui::InputModality::Keyboard
            | nickel_ui::InputModality::Controller
            | nickel_ui::InputModality::Accessibility => {
                OverlayAnchor::InvocationTargetCenter(target)
            }
        };
        let configure = |mut menu: OverlayMenu<FileMessage>| {
            menu.width = 170.0;
            menu.row_height = 34.0;
            menu.padding = Insets::all(4.0);
            menu.radius = 7.0;
            menu.background = palette.surface;
            menu.foreground = palette.text;
            menu.text_scale = 1.2;
            menu.item_hover = Some(palette.surface_hover);
            menu.item_pressed = Some(palette.accent_soft);
            menu.item_selected = Some(palette.accent_soft);
            menu.item_radius = 5.0;
            menu
        };
        let visible_entries = visible_file_range(
            self,
            context.viewport.size.width,
            context.viewport.size.height,
        );
        let mut overlays = self
            .browser
            .entries()
            .iter()
            .enumerate()
            .filter(|(index, _)| visible_entries.contains(index))
            .map(|(index, _)| {
                FrameOverlay::Menu(configure(
                    OverlayMenu::new(
                        format!("file-entry-{index}-context"),
                        invocation_anchor(UiId::new(format!("file-entry-{index}"))),
                    )
                    .item(OverlayMenuItem::action(
                        "open",
                        "Open",
                        FileMessage::ContextOpen,
                    ))
                    .item(OverlayMenuItem::action(
                        "open-new-tab",
                        "Open in New Tab",
                        FileMessage::ContextOpenNewTab,
                    )),
                ))
            })
            .collect::<Vec<_>>();
        overlays.push(FrameOverlay::Menu(configure(
            OverlayMenu::new(
                "file-background-context",
                invocation_anchor(UiId::from("file-content")),
            )
            .item(OverlayMenuItem::action(
                "refresh",
                "Refresh",
                FileMessage::ContextRefresh,
            ))
            .item(OverlayMenuItem::action(
                "select-all",
                "Select All",
                FileMessage::ContextSelectAll,
            )),
        )));
        if let Some(start) = self.selection_drag {
            overlays.push(FrameOverlay::SelectionMarquee {
                rect: rect_between(start, self.cursor),
                fill: Some(0x4068_b8ff),
                stroke: palette.accent,
                width: 1.0,
            });
        }
        overlays
    }

    fn poll(&mut self) -> bool {
        let before = self.next_icon_id;
        self.poll_icons();
        self.poll_navigation() || before != self.next_icon_id
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        [
            self.icon_rx.as_ref().map(|_| self.icon_poll_delay),
            self.navigation_rx
                .as_ref()
                .map(|_| self.navigation_poll_delay),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn title(&self) -> &str {
        "Nickel File"
    }

    fn initial_size(&self) -> (u32, u32) {
        (860, 620)
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let _log_path = nickel_logging::init("nickel-file").ok();
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(home_directory);
    nickel_ui::run_with_adapter(FileApp::new(path), FileHostAdapter::default())
}

#[cfg(test)]
#[path = "ui_layout_tests.rs"]
mod ui_layout_tests;
