use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
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
    operations::{
        ClipboardOffer, ConflictPolicy, DragOffer, ItemCapabilities, OperationEffect, RenameEditor,
        TransferIntent, TransferReport, TransferSource,
    },
    platform::{LocationGroup, home_directory, location_groups},
    watch::DirectoryWatch,
};
use nickel_core::{
    shell_settings::{FileIconPreference, ShellSettings},
    theme::{Appearance, ThemeMode, ThemePalette},
};
use nickel_file::{DirectoryBrowser, EntrySortKey, FileEntry, SortDirection};
use nickel_i18n::Localizer;
use nickel_platform::{DefaultLaunchError as OpenPathError, open_with_default};
use nickel_ui::{
    AnyView, Application, FrameOverlay, Insets, OverlayAnchor, OverlayMenu, OverlayMenuItem,
    OverlayStyle, Point, ReadingDirection, Size, TextField, TransientSurface, UiId, ViewContext,
    ui,
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
const SETTINGS_SYNC_INTERVAL: Duration = Duration::from_secs(1);
const DIRECTORY_WATCH_POLL_INTERVAL: Duration = Duration::from_millis(100);
const DIRECTORY_WATCH_RETRY_INTERVAL: Duration = Duration::from_secs(2);
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_DISTANCE: f32 = 6.0;

fn entry_context_menu_id(path: &std::path::Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("file-entry-context-{:016x}", hasher.finish())
}

pub(crate) fn drop_target_id(prefix: &str, path: &std::path::Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("file-drop-{prefix}-{:016x}", hasher.finish())
}
type NavigationResult = (u64, Result<Option<DirectoryBrowser>, String>);
type SidebarResult = (PathBuf, Result<Vec<(String, PathBuf)>, String>);
type ActivationResult = (u64, String, Result<(), OpenPathError>);
type TransferUpdate = (TransferIntent, usize, usize, Option<TransferReport>);
type RenameResult = (crate::FileIdentity, PathBuf, Result<(), String>);

#[derive(Clone, Debug)]
pub(crate) struct PendingTransferConflict {
    offer: ClipboardOffer,
    destination: PathBuf,
    conflicts: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct FileClick {
    path: PathBuf,
    identity: Option<crate::FileIdentity>,
    position: Point,
    when: Instant,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FileMessage {
    ContextEntry(usize),
    ContextBackground,
    ContextOpen,
    ContextOpenNewTab,
    ContextCut,
    ContextCopy,
    ContextPasteInto,
    ContextNewFolder,
    ContextCopyPath,
    ContextRename,
    ContextProperties,
    ContextCurrentFolderProperties,
    CloseProperties,
    DiscardProperties,
    PropertiesSelectHandler(usize),
    PropertiesOpenOnce,
    PropertiesRequestDefault,
    PropertiesConfirmDefault,
    PropertiesDefaultSettings,
    PropertiesCalculateSize,
    PropertiesCancelSize,
    PropertiesToggleReadonly,
    PropertiesToggleHidden,
    PropertiesApply,
    PropertiesOk,
    PropertiesScroll(f32),
    ContextRefresh,
    ContextSelectAll,
    BeginRename,
    RenameChanged(String),
    CommitRename,
    CancelRename,
    CancelTransfer,
    TransferKeepBoth,
    TransferSkipConflicts,
    TransferCancelConflicts,
    CopySelection,
    CutSelection,
    Paste,
    ToggleCommandSurface,
    CommandQueryChanged(String),
    CommandScroll(f32),
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
    AdjustTileWidth(i8),
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
    pub(crate) localizer: Localizer,
    pub(crate) browser: DirectoryBrowser,
    pub(crate) cursor: Point,
    /// Stable selection authority; indices are derived only at view and input boundaries.
    pub(crate) selected: Option<crate::FileIdentity>,
    pub(crate) selected_entries: HashSet<crate::FileIdentity>,
    pub(crate) rename_editor: Option<RenameEditor>,
    rename_rx: Option<Receiver<RenameResult>>,
    pub(crate) file_clipboard: Option<ClipboardOffer>,
    pub(crate) drag_hover: Option<PathBuf>,
    native_drop_batch: Vec<PathBuf>,
    native_drop_deadline: Option<Instant>,
    pub(crate) native_drop_destination: Option<PathBuf>,
    native_drop_batch_destination: Option<PathBuf>,
    pub(crate) native_drop_hover_started: Option<(PathBuf, Instant)>,
    native_drop_intent: TransferIntent,
    pub(crate) outbound_drag: Option<DragOffer>,
    pub(crate) primary_down: bool,
    pub(crate) transfer_rx: Option<Receiver<TransferUpdate>>,
    transfer_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) pending_transfer_conflict: Option<PendingTransferConflict>,
    pub(crate) selection_anchor: Option<crate::FileIdentity>,
    pub(crate) active_tab_id: u64,
    pub(crate) next_tab_id: u64,
    pub(crate) status: String,
    pub(crate) last_click: Option<FileClick>,
    /// The pointer anchor and filesystem identity are captured by the context
    /// invocation. They deliberately do not follow later pointer or model motion.
    pub(crate) context_anchor: Option<Point>,
    pub(crate) context_target: Option<PathBuf>,
    pub(crate) context_selection: Vec<PathBuf>,
    pub(crate) properties: Option<crate::properties::EntryProperties>,
    pub(crate) properties_association: Option<nickel_platform::AssociationSnapshot>,
    properties_association_rx:
        Option<Receiver<Result<nickel_platform::AssociationSnapshot, String>>>,
    pub(crate) properties_association_status: String,
    pub(crate) properties_handler: Option<usize>,
    pub(crate) properties_confirm_default: bool,
    pub(crate) properties_size_job: Option<crate::properties::RecursiveSizeJob>,
    pub(crate) properties_size_progress: Option<String>,
    pub(crate) properties_edits: Option<crate::properties::PropertyEdits>,
    pub(crate) properties_scroll: f32,
    pub(crate) properties_confirm_close: bool,
    activation_rx: Option<Receiver<ActivationResult>>,
    activation_op: fn(&std::path::Path) -> Result<(), OpenPathError>,
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
    navigation_preserves_selection: bool,
    directory_watch: Option<DirectoryWatch>,
    directory_watch_retry_at: Option<Instant>,
    pub(crate) icon_preference: FileIconPreference,
    pub(crate) icon_theme: Option<String>,
    pub(crate) icon_provider_revision: u64,
    pub(crate) icon_appearance: ThemeMode,
    pub(crate) artwork_scale_milli: u16,
    pub(crate) next_icon_id: u16,
    pub(crate) sidebar_width: f32,
    pub(crate) expanded_folders: HashSet<PathBuf>,
    pub(crate) location_groups: Vec<LocationGroup>,
    location_groups_rx: Option<Receiver<Vec<LocationGroup>>>,
    pub(crate) sidebar_children: HashMap<PathBuf, Vec<(String, PathBuf)>>,
    sidebar_loading: HashSet<PathBuf>,
    sidebar_sender: mpsc::Sender<SidebarResult>,
    sidebar_receiver: Receiver<SidebarResult>,
    pub(crate) collapsed_location_groups: HashSet<String>,
    pub(crate) control_down: bool,
    pub(crate) shift_down: bool,
    pub(crate) selection_drag: Option<Point>,
    pub(crate) resizing_sidebar: bool,
    pub(crate) resizing_details_column: Option<DetailsColumnResize>,
    pub(crate) places_open: bool,
    pub(crate) command_surface_open: bool,
    pub(crate) command_query: String,
    pub(crate) command_scroll_offset: f32,
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
        "minimum-grid-dark",
        "Minimum Grid Dark",
        "minimum",
        560,
        360,
        Dark
    ),
    file_fixture_variant!(
        "minimum-details-light",
        "Minimum Details Light",
        "minimum",
        560,
        360,
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
    file_fixture_variant!(
        "command-surface",
        "Searchable Command Surface",
        "wide",
        1100,
        700,
        Dark
    ),
    file_fixture_variant!(
        "minimum-command-surface",
        "Minimum Searchable Command Surface",
        "minimum",
        560,
        360,
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
                        display_name_override: None,
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
                        display_name_override: None,
                        name: "写真と音楽 🎵".into(),
                        path: PathBuf::from("/fixture/写真と音楽 🎵"),
                        is_directory: true,
                        size: None,
                        modified: None,
                    },
                    FileEntry {
                        display_name_override: None,
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
                            display_name_override: None,
                            name: ".nickel-cache".into(),
                            path: PathBuf::from("/fixture/.nickel-cache"),
                            is_directory: true,
                            size: None,
                            modified: None,
                        },
                        FileEntry {
                            display_name_override: None,
                            name: "report.txt".into(),
                            path: PathBuf::from("/fixture/report.txt"),
                            is_directory: false,
                            size: Some(128),
                            modified: None,
                        },
                        FileEntry {
                            display_name_override: None,
                            name: "notes.md".into(),
                            path: PathBuf::from("/fixture/notes.md"),
                            is_directory: false,
                            size: Some(512),
                            modified: None,
                        },
                    ]),
                    String::new(),
                );
                app.selected = app.identity_at(1);
                app.selection_anchor = app.identity_at(0);
                app.selected_entries = [0, 1]
                    .into_iter()
                    .filter_map(|index| app.identity_at(index))
                    .collect();
                app
            }
            "empty" | "unavailable" | "unreadable" | "loading" | "disconnected" => {
                FileApp::with_browser(DirectoryBrowser::fixture(Vec::new()), String::new())
            }
            _ => FileApp::fixture(),
        };
        if variant.id.contains("details") {
            app.view_mode = FileViewMode::Details;
        }
        if variant.id.ends_with("command-surface") {
            app.command_surface_open = true;
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
        app.localizer = Localizer::for_locale(Some(variant.locale.id));
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
        sha256: "2c492d5438c43d6c0b7e376ce07399560d64fa66b2ebf0ae4054195bcf9cb2e4",
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
    nickel_ui_testkit::FixtureAsset {
        id: "nickel-file-unknown-file",
        path: "assets/concepts/nickel-file-icon-family/unknown-file.png",
        license: "Same license as Nickel",
        sha256: "e2132ad8b5ca505ea2f453f533835f4d81f8a6f33853e50c73081e05d4e141de",
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
    pub(crate) selected: Option<crate::FileIdentity>,
    pub(crate) selected_entries: HashSet<crate::FileIdentity>,
    pub(crate) selection_anchor: Option<crate::FileIdentity>,
    pub(crate) tab_id: u64,
    pub(crate) status: String,
    pub(crate) last_click: Option<FileClick>,
    pub(crate) file_scroll_offset: f32,
    pub(crate) tile_width: f32,
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
    navigation_preserves_selection: bool,
    directory_watch: Option<DirectoryWatch>,
    directory_watch_retry_at: Option<Instant>,
}

impl FileApp {
    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected
            .and_then(|identity| self.browser.index_of_identity(identity))
    }

    pub(crate) fn is_index_selected(&self, index: usize) -> bool {
        self.browser
            .identity_at(index)
            .is_some_and(|identity| self.selected_entries.contains(&identity))
    }

    fn identity_at(&self, index: usize) -> Option<crate::FileIdentity> {
        self.browser.identity_at(index)
    }

    pub(crate) fn selected_is_container(&self) -> bool {
        self.selected_index()
            .and_then(|index| self.browser.entries().get(index))
            .is_some_and(|entry| entry.is_directory)
    }
    pub(crate) fn is_resizing_sidebar(&self) -> bool {
        self.resizing_sidebar
    }

    pub(crate) fn is_resizing_details_column(&self) -> bool {
        self.resizing_details_column.is_some()
    }

    pub(crate) fn navigation_pending(&self) -> bool {
        self.navigation_rx.is_some() || self.fixture_navigation_busy
    }

    fn assign_tab_icon(&mut self, path: &std::path::Path, icon: (u16, Arc<image::RgbaImage>)) {
        if self.browser.current() == path {
            self.tab_icon = Some(icon.clone());
        }
        for tab in self.tabs.iter_mut().flatten() {
            if tab.browser.current() == path {
                tab.tab_icon = Some(icon.clone());
            }
        }
    }

    fn toggle_sidebar_folder(&mut self, path: PathBuf) {
        if self.expanded_folders.remove(&path) {
            return;
        }
        self.expanded_folders.insert(path.clone());
        if self.sidebar_children.contains_key(&path) || !self.sidebar_loading.insert(path.clone()) {
            return;
        }
        let sender = self.sidebar_sender.clone();
        let _ = std::thread::Builder::new()
            .name("nickel-file-sidebar".into())
            .spawn(move || {
                let result = std::fs::read_dir(&path)
                    .map_err(|error| error.to_string())
                    .map(|entries| {
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
                        children.sort_by(|left, right| {
                            left.0.to_lowercase().cmp(&right.0.to_lowercase())
                        });
                        children
                    });
                let _ = sender.send((path, result));
            });
    }

    fn poll_sidebar_children(&mut self) -> bool {
        let mut changed = false;
        let mut artwork_changed = false;
        loop {
            match self.sidebar_receiver.try_recv() {
                Ok((path, Ok(children))) => {
                    self.sidebar_loading.remove(&path);
                    self.sidebar_children.insert(path, children);
                    changed = true;
                    artwork_changed = true;
                }
                Ok((path, Err(error))) => {
                    self.sidebar_loading.remove(&path);
                    self.status = format!("Could not expand {}: {error}", path.display());
                    changed = true;
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                    if artwork_changed {
                        self.refresh_icons();
                    }
                    return changed;
                }
            }
        }
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

    /// Creates the production application without enumerating the initial location on the UI
    /// thread. Deterministic callers that need an already-populated model can continue to use
    /// [`Self::new`].
    pub fn launch(path: PathBuf) -> Self {
        let show_hidden = nickel_platform::show_hidden_files();
        let browser = DirectoryBrowser::loading(path.clone(), show_hidden);
        let mut app = Self::with_browser(browser, "Opening location…".into());
        let (sender, receiver) = mpsc::channel();
        app.navigation_generation = 1;
        app.navigation_rx = Some(receiver);
        app.navigation_invalidates_icons = true;
        let _ = std::thread::Builder::new()
            .name("nickel-file-initial-location".into())
            .spawn(move || {
                let result = DirectoryBrowser::open_with_hidden(&path, show_hidden)
                    .map(Some)
                    .map_err(|error| {
                        format!(
                            "Could not open initial location {}: {error}",
                            path.display()
                        )
                    });
                let _ = sender.send((1, result));
            });
        app
    }

    pub fn launch_properties(path: PathBuf) -> Self {
        let location = path
            .parent()
            .map(|parent| parent.to_path_buf())
            .unwrap_or_else(|| path.clone());
        let mut app = Self::new(location);
        app.context_target = Some(path);
        <Self as nickel_ui::Application>::update(&mut app, FileMessage::ContextProperties);
        app
    }

    pub fn launch_rename(path: PathBuf) -> Self {
        let location = path
            .parent()
            .map(|parent| parent.to_path_buf())
            .unwrap_or_else(|| path.clone());
        let mut app = Self::new(location);
        if let Some(index) = app
            .browser
            .entries()
            .iter()
            .position(|entry| entry.path == path)
        {
            app.selected = app.identity_at(index);
            app.selected_entries.extend(app.identity_at(index));
            <Self as nickel_ui::Application>::update(&mut app, FileMessage::BeginRename);
        }
        app
    }

    fn with_browser(browser: DirectoryBrowser, status: String) -> Self {
        let settings = ShellSettings::load_default();
        let (sidebar_sender, sidebar_receiver) = mpsc::channel();
        let resolved_location_groups = location_groups();
        let icon_appearance = settings
            .resolve_appearance(nickel_platform::appearance())
            .mode;
        let mut app = Self {
            localizer: Localizer::system(),
            browser,
            cursor: Point { x: 0.0, y: 0.0 },
            selected: None,
            selected_entries: HashSet::new(),
            rename_editor: None,
            rename_rx: None,
            file_clipboard: None,
            drag_hover: None,
            native_drop_batch: Vec::new(),
            native_drop_deadline: None,
            native_drop_destination: None,
            native_drop_batch_destination: None,
            native_drop_hover_started: None,
            native_drop_intent: TransferIntent::Copy,
            outbound_drag: None,
            primary_down: false,
            transfer_rx: None,
            transfer_cancel: None,
            pending_transfer_conflict: None,
            selection_anchor: None,
            active_tab_id: 0,
            next_tab_id: 1,
            status,
            last_click: None,
            context_anchor: None,
            context_target: None,
            context_selection: Vec::new(),
            properties: None,
            properties_association: None,
            properties_association_rx: None,
            properties_association_status: String::new(),
            properties_handler: None,
            properties_confirm_default: false,
            properties_size_job: None,
            properties_size_progress: None,
            properties_edits: None,
            properties_scroll: 0.0,
            properties_confirm_close: false,
            activation_rx: None,
            activation_op: open_with_default,
            icons: icons::ArtworkCache::default(),
            icon_rx: None,
            icon_poll_delay: std::time::Duration::from_millis(16),
            icon_generation: 0,
            navigation_rx: None,
            navigation_generation: 0,
            navigation_poll_delay: Duration::from_millis(16),
            navigation_closes_address: false,
            navigation_invalidates_icons: false,
            navigation_preserves_selection: false,
            directory_watch: None,
            directory_watch_retry_at: None,
            icon_preference: settings.file_icon_provider,
            icon_provider_revision: icons::provider_revision(
                settings.file_icon_provider,
                settings.file_icon_theme.as_deref(),
            ),
            icon_theme: settings.file_icon_theme,
            icon_appearance,
            artwork_scale_milli: 1_000,
            next_icon_id: 1,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            expanded_folders: HashSet::new(),
            location_groups: resolved_location_groups,
            location_groups_rx: None,
            sidebar_children: HashMap::new(),
            sidebar_loading: HashSet::new(),
            sidebar_sender,
            sidebar_receiver,
            collapsed_location_groups: HashSet::new(),
            control_down: false,
            shift_down: false,
            selection_drag: None,
            resizing_sidebar: false,
            resizing_details_column: None,
            places_open: false,
            command_surface_open: false,
            command_query: String::new(),
            command_scroll_offset: 0.0,
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
        app.ensure_directory_watch();
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
            display_name_override: None,
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
        app.refresh_icons_for(FileIconPreference::Nickel, ThemeMode::Dark);
        app
    }

    fn set_scroll_offset(&mut self, offset: f32) {
        self.file_scroll_offset = offset.max(0.0);
    }

    pub(crate) fn inactive_tab(&self, index: usize) -> Option<&FileTab> {
        self.tabs.get(index).and_then(Option::as_ref)
    }

    pub(crate) fn switch_tab(&mut self, index: usize) {
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
        std::mem::swap(&mut self.tile_width, &mut target.tile_width);
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
        std::mem::swap(
            &mut self.navigation_preserves_selection,
            &mut target.navigation_preserves_selection,
        );
        std::mem::swap(&mut self.directory_watch, &mut target.directory_watch);
        std::mem::swap(
            &mut self.directory_watch_retry_at,
            &mut target.directory_watch_retry_at,
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
        let browser = DirectoryBrowser::loading(path.clone(), show_hidden);
        let directory_watch = DirectoryWatch::start(&path).ok();
        let (sender, receiver) = mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("nickel-file-new-tab".into())
            .spawn(move || {
                let result = DirectoryBrowser::open_with_hidden(&path, show_hidden)
                    .map(Some)
                    .map_err(|error| format!("Could not open new tab: {error}"));
                let _ = sender.send((1, result));
            });
        let tab_id = self.next_tab_id;
        self.next_tab_id = self.next_tab_id.saturating_add(1);
        self.tabs.push(Some(FileTab {
            browser,
            selected: None,
            selected_entries: HashSet::new(),
            selection_anchor: None,
            tab_id,
            status: "Opening location…".into(),
            last_click: None,
            file_scroll_offset: 0.0,
            tile_width: DEFAULT_TILE_WIDTH,
            view_mode: FileViewMode::Grid,
            sort_key: EntrySortKey::Name,
            sort_direction: SortDirection::Ascending,
            details_column_widths: DetailsColumnWidths::default(),
            address_editing: false,
            address_text: String::new(),
            tab_icon: None,
            navigation_rx: Some(receiver),
            navigation_generation: 1,
            navigation_poll_delay: Duration::from_millis(16),
            navigation_closes_address: false,
            navigation_invalidates_icons: true,
            navigation_preserves_selection: false,
            directory_watch,
            directory_watch_retry_at: None,
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
    pub(crate) fn activate_selected(&mut self) {
        let entry = self
            .selected_index()
            .and_then(|index| self.browser.entries().get(index))
            .cloned();
        self.activate_entry(entry);
    }

    fn activate_context_target(&mut self) {
        let entry = self.context_target.as_ref().and_then(|path| {
            self.browser
                .entries()
                .iter()
                .find(|entry| &entry.path == path)
                .cloned()
        });
        self.activate_entry(entry);
    }

    fn activate_entry(&mut self, entry: Option<FileEntry>) {
        let Some(entry) = entry else {
            self.status = "The selected item is no longer available".into();
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
        } else {
            if self.activation_rx.is_some() {
                self.status = "Another file is still opening…".into();
                return;
            }
            let label = entry.display_name().to_owned();
            self.status = format!("Opening {label}…");
            let path = entry.path;
            let tab_id = self.active_tab_id;
            let activation_op = self.activation_op;
            let (sender, receiver) = mpsc::channel();
            self.activation_rx = Some(receiver);
            let _ = std::thread::Builder::new()
                .name("nickel-file-activation".into())
                .spawn(move || {
                    let result = activation_op(&path);
                    let _ = sender.send((tab_id, label, result));
                });
        }
    }

    fn poll_activation(&mut self) -> bool {
        let Some(receiver) = self.activation_rx.as_ref() else {
            return false;
        };
        match receiver.try_recv() {
            Ok((tab_id, label, result)) => {
                let status = match result {
                    Ok(()) => format!("Opened {label}"),
                    Err(error) => format!("Could not open {label}: {error}"),
                };
                if tab_id == self.active_tab_id {
                    self.status = status;
                } else if let Some(tab) = self
                    .tabs
                    .iter_mut()
                    .flatten()
                    .find(|tab| tab.tab_id == tab_id)
                {
                    tab.status = status;
                }
                self.activation_rx = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.status = "Could not open item: activation worker stopped".into();
                self.activation_rx = None;
                true
            }
        }
    }

    pub(crate) fn navigate_to(&mut self, path: PathBuf) {
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

    pub(crate) fn go_forward(&mut self) {
        if self.browser.can_go_forward() {
            self.start_navigation(
                "Going forward".into(),
                "Could not go forward",
                false,
                |browser| browser.forward().map_err(|error| error.to_string()),
            );
        }
    }

    pub(crate) fn go_up(&mut self) {
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
        // A refreshed/replaced model cannot complete a click transaction that
        // began against the previous generation.
        self.last_click = None;
        self.navigation_generation = self.navigation_generation.wrapping_add(1);
        let generation = self.navigation_generation;
        let mut browser = self.browser.clone();
        let (sender, receiver) = mpsc::channel();
        self.navigation_rx = Some(receiver);
        self.navigation_poll_delay = Duration::from_millis(16);
        self.navigation_closes_address = closes_address;
        self.navigation_invalidates_icons = false;
        self.navigation_preserves_selection = false;
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
        self.reconcile_directory(show_hidden);
        self.refresh_location_groups();
    }

    fn reconcile_directory(&mut self, show_hidden: bool) {
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
        self.navigation_preserves_selection = true;
    }

    fn refresh_location_groups(&mut self) {
        let (sender, receiver) = mpsc::channel();
        self.location_groups_rx = Some(receiver);
        let _ = std::thread::Builder::new()
            .name("nickel-file-locations".into())
            .spawn(move || {
                let _ = sender.send(location_groups());
            });
    }

    fn poll_location_groups(&mut self) -> bool {
        let result = match self.location_groups_rx.as_ref() {
            Some(receiver) => receiver.try_recv(),
            None => return false,
        };
        match result {
            Ok(groups) => {
                self.location_groups_rx = None;
                if self.location_groups == groups {
                    return false;
                }
                self.location_groups = groups;
                self.refresh_icons();
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.location_groups_rx = None;
                false
            }
        }
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
                        let changed_icon_paths = if self.navigation_invalidates_icons {
                            self.browser
                                .entries()
                                .iter()
                                .filter(|old| {
                                    browser
                                        .entries()
                                        .iter()
                                        .find(|new| new.path == old.path)
                                        .is_none_or(|new| new != *old)
                                })
                                .map(|entry| entry.path.clone())
                                .chain(
                                    browser
                                        .entries()
                                        .iter()
                                        .filter(|new| {
                                            !self
                                                .browser
                                                .entries()
                                                .iter()
                                                .any(|old| old.path == new.path)
                                        })
                                        .map(|new| new.path.clone()),
                                )
                                .collect::<HashSet<_>>()
                        } else {
                            HashSet::new()
                        };
                        let focused = self.selected.and_then(|identity| {
                            self.browser.index_of_identity(identity).and_then(|index| {
                                self.browser
                                    .entries()
                                    .get(index)
                                    .map(|entry| (Some(identity), entry.path.clone()))
                            })
                        });
                        let selected = self
                            .selected_entries
                            .iter()
                            .filter_map(|identity| {
                                self.browser.index_of_identity(*identity).and_then(|index| {
                                    self.browser
                                        .entries()
                                        .get(index)
                                        .map(|entry| (Some(*identity), entry.path.clone()))
                                })
                            })
                            .collect::<Vec<_>>();
                        let anchor = self.selection_anchor.and_then(|identity| {
                            self.browser.index_of_identity(identity).and_then(|index| {
                                self.browser
                                    .entries()
                                    .get(index)
                                    .map(|entry| (Some(identity), entry.path.clone()))
                            })
                        });
                        self.browser = browser;
                        if self.navigation_invalidates_icons {
                            for path in changed_icon_paths {
                                self.icons.remove(&path);
                            }
                        }
                        self.navigation_invalidates_icons = false;
                        if self.navigation_closes_address {
                            self.address_editing = false;
                            self.address_text.clear();
                        }
                        if self.navigation_preserves_selection {
                            self.reconciliation_changed(focused, selected, anchor);
                        } else {
                            self.navigation_changed();
                        }
                        self.navigation_preserves_selection = false;
                    }
                    Ok(None) => {
                        self.navigation_invalidates_icons = false;
                        self.navigation_preserves_selection = false;
                        self.status.clear();
                    }
                    Err(error) => {
                        self.navigation_invalidates_icons = false;
                        if self.navigation_preserves_selection {
                            self.directory_watch = None;
                            self.directory_watch_retry_at =
                                Some(Instant::now() + DIRECTORY_WATCH_RETRY_INTERVAL);
                            self.status = format!(
                                "Live updates are unavailable: {error}. Retrying automatically; Refresh remains available."
                            );
                        } else {
                            self.status = error;
                        }
                        self.navigation_preserves_selection = false;
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
                self.navigation_preserves_selection = false;
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
        self.context_target = None;
        self.context_anchor = None;
        self.context_selection.clear();
        self.set_scroll_offset(0.0);
        self.status.clear();
        self.ensure_directory_watch();
        self.refresh_icons();
    }

    fn reconciliation_changed(
        &mut self,
        focused: Option<(Option<nickel_file::FileIdentity>, PathBuf)>,
        selected: Vec<(Option<nickel_file::FileIdentity>, PathBuf)>,
        anchor: Option<(Option<nickel_file::FileIdentity>, PathBuf)>,
    ) {
        self.browser.sort(self.sort_key, self.sort_direction);
        let resolve = |candidate: &(Option<nickel_file::FileIdentity>, PathBuf)| {
            candidate
                .0
                .filter(|identity| self.browser.index_of_identity(*identity).is_some())
                .or_else(|| {
                    if candidate.0.is_none() {
                        self.browser
                            .entries()
                            .iter()
                            .position(|entry| entry.path == candidate.1)
                            .and_then(|index| self.browser.identity_at(index))
                    } else {
                        None
                    }
                })
        };
        self.selected_entries = selected.iter().filter_map(resolve).collect();
        self.selected = focused
            .as_ref()
            .and_then(resolve)
            .or_else(|| self.selected_entries.iter().copied().min());
        self.selection_anchor = anchor.as_ref().and_then(resolve).or(self.selected);
        if self.context_target.as_ref().is_some_and(|target| {
            !self
                .browser
                .entries()
                .iter()
                .any(|entry| &entry.path == target)
        }) {
            self.context_target = None;
            self.context_anchor = None;
            self.context_selection.clear();
        }
        self.status.clear();
        self.ensure_directory_watch();
        self.refresh_icons();
    }

    fn ensure_directory_watch(&mut self) {
        if self
            .directory_watch
            .as_ref()
            .is_some_and(|watch| watch.watches(self.browser.current()))
        {
            return;
        }
        match DirectoryWatch::start(self.browser.current()) {
            Ok(watch) => {
                self.directory_watch = Some(watch);
                self.directory_watch_retry_at = None;
                if self.status.starts_with("Live updates are unavailable:") {
                    self.status.clear();
                }
            }
            Err(error) => {
                self.directory_watch = None;
                self.directory_watch_retry_at =
                    Some(Instant::now() + DIRECTORY_WATCH_RETRY_INTERVAL);
                if self.browser.current().exists() {
                    self.status = format!(
                        "Live updates are unavailable: {error}. Retrying automatically; Refresh remains available."
                    );
                }
                tracing::warn!(
                    path = %self.browser.current().display(),
                    %error,
                    "live directory updates are unavailable"
                );
            }
        }
    }

    fn poll_directory_watch(&mut self) -> bool {
        if let Some(retry_at) = self.directory_watch_retry_at
            && Instant::now() >= retry_at
        {
            self.ensure_directory_watch();
        }
        let Some(watch) = self.directory_watch.as_ref() else {
            return false;
        };
        if let Some(error) = watch.take_failure() {
            self.status = format!(
                "Live updates are unavailable: {error}. Retrying automatically; Refresh remains available."
            );
            self.directory_watch = None;
            self.directory_watch_retry_at = Some(Instant::now() + DIRECTORY_WATCH_RETRY_INTERVAL);
            return true;
        }
        if self.navigation_rx.is_some() || !watch.take_invalidation() {
            return false;
        }
        self.reconcile_directory(self.browser.show_hidden());
        true
    }

    fn sort_by(&mut self, key: EntrySortKey) {
        let focused = self
            .selected_index()
            .and_then(|index| self.browser.entries().get(index))
            .map(|entry| entry.path.clone());
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
        self.selected_entries
            .retain(|identity| self.browser.index_of_identity(*identity).is_some());
        self.selected = focused.and_then(|path| {
            self.browser
                .entries()
                .iter()
                .position(|entry| entry.path == path)
                .and_then(|index| self.browser.identity_at(index))
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
        let provider_revision = icons::provider_revision(preference, theme);
        if preference != self.icon_preference
            || self.icon_theme.as_deref() != theme
            || self.icon_provider_revision != provider_revision
            || self.icon_appearance != appearance
        {
            self.icon_preference = preference;
            self.icon_theme = theme.map(str::to_owned);
            self.icon_provider_revision = provider_revision;
            self.icon_appearance = appearance;
            self.icons.clear();
            self.tab_icon = None;
            for tab in self.tabs.iter_mut().flatten() {
                tab.tab_icon = None;
            }
        }
        self.icon_generation = self.icon_generation.wrapping_add(1);
        let generation = self.icon_generation;
        let artwork_appearance = if appearance == ThemeMode::Light {
            icons::ArtworkAppearance::Light
        } else {
            icons::ArtworkAppearance::Dark
        };
        let mut entries = self
            .browser
            .entries()
            .iter()
            .map(|entry| {
                let request = icons::ArtworkRequest {
                    path: &entry.path,
                    kind: icons::semantic_kind(&entry.path, entry.is_directory),
                    logical_size: 96,
                    scale_milli: self.artwork_scale_milli,
                    appearance: artwork_appearance,
                };
                (
                    entry.path.clone(),
                    entry.is_directory,
                    icons::cache_key_with_theme(preference, self.icon_theme.as_deref(), &request),
                )
            })
            .collect::<Vec<_>>();
        let mut known_paths = entries
            .iter()
            .map(|(path, _, _)| path.clone())
            .collect::<HashSet<_>>();
        for (path, is_directory) in self
            .location_groups
            .iter()
            .flat_map(|group| group.entries.iter())
            .map(|(_, path)| (path.clone(), true))
        {
            if known_paths.insert(path.clone()) {
                let request = icons::ArtworkRequest {
                    path: &path,
                    kind: icons::semantic_kind(&path, is_directory),
                    logical_size: 96,
                    scale_milli: self.artwork_scale_milli,
                    appearance: artwork_appearance,
                };
                let key =
                    icons::cache_key_with_theme(preference, self.icon_theme.as_deref(), &request);
                entries.push((path, is_directory, key));
            }
        }
        for path in self
            .sidebar_children
            .values()
            .flatten()
            .map(|(_, path)| path.clone())
        {
            if known_paths.insert(path.clone()) {
                let request = icons::ArtworkRequest {
                    path: &path,
                    kind: icons::semantic_kind(&path, true),
                    logical_size: 96,
                    scale_milli: self.artwork_scale_milli,
                    appearance: artwork_appearance,
                };
                let key =
                    icons::cache_key_with_theme(preference, self.icon_theme.as_deref(), &request);
                entries.push((path, true, key));
            }
        }
        for path in self
            .tabs
            .iter()
            .flatten()
            .map(|tab| tab.browser.current().to_path_buf())
        {
            if known_paths.insert(path.clone()) {
                let request = icons::ArtworkRequest {
                    path: &path,
                    kind: icons::semantic_kind(&path, true),
                    logical_size: 96,
                    scale_milli: self.artwork_scale_milli,
                    appearance: artwork_appearance,
                };
                let key =
                    icons::cache_key_with_theme(preference, self.icon_theme.as_deref(), &request);
                entries.push((path, true, key));
            }
        }
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
            scale_milli: self.artwork_scale_milli,
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
                    scale_milli: self.artwork_scale_milli,
                    appearance: artwork_appearance,
                },
            );
            let id = self.next_icon_id;
            self.next_icon_id = self.next_icon_id.checked_add(1).unwrap_or(1);
            self.assign_tab_icon(path, (id, artwork.pixels.clone()));
            self.icons
                .insert_resolved(key.clone(), (id, artwork.pixels));
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
        let artwork_scale_milli = self.artwork_scale_milli;
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
                            scale_milli: artwork_scale_milli,
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

    fn sync_icon_settings(&mut self) -> bool {
        let settings = ShellSettings::load_default();
        let appearance = settings
            .resolve_appearance(nickel_platform::appearance())
            .mode;
        let revision = icons::provider_revision(
            settings.file_icon_provider,
            settings.file_icon_theme.as_deref(),
        );
        let changed = settings.file_icon_provider != self.icon_preference
            || settings.file_icon_theme != self.icon_theme
            || revision != self.icon_provider_revision
            || appearance != self.icon_appearance;
        if changed {
            self.refresh_icons_for_theme(
                settings.file_icon_provider,
                settings.file_icon_theme.as_deref(),
                appearance,
            );
        }
        changed
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
                    self.assign_tab_icon(&path, (id, artwork.pixels.clone()));
                    self.icons.insert_resolved(key, (id, artwork.pixels));
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
        let Some(current) = self.selected_index() else {
            self.select_only(0);
            self.ensure_selection_visible();
            return;
        };
        let next = (current as isize + delta).clamp(0, len as isize - 1) as usize;
        self.selected = self.identity_at(next);
        if self.shift_down {
            self.select_range(next, self.control_down);
        } else if !self.control_down {
            self.select_only(next);
        }
        self.ensure_selection_visible();
    }

    fn select_only(&mut self, index: usize) {
        let Some(identity) = self.identity_at(index) else {
            return;
        };
        self.selected = Some(identity);
        self.selected_entries.clear();
        self.selected_entries.insert(identity);
        self.selection_anchor = Some(identity);
    }

    fn select_range(&mut self, target: usize, additive: bool) {
        let anchor = self
            .selection_anchor
            .and_then(|identity| self.browser.index_of_identity(identity))
            .or_else(|| self.selected_index())
            .unwrap_or(target);
        if !additive {
            self.selected_entries.clear();
        }
        let identities = (anchor.min(target)..=anchor.max(target))
            .filter_map(|index| self.identity_at(index))
            .collect::<Vec<_>>();
        self.selected_entries.extend(identities);
        self.selected = self.identity_at(target);
        self.selection_anchor = self.identity_at(anchor);
    }

    pub(crate) fn toggle_active_selection(&mut self) {
        let Some(identity) = self.selected else {
            return;
        };
        if !self.selected_entries.remove(&identity) {
            self.selected_entries.insert(identity);
            self.selection_anchor = Some(identity);
        } else if self.selection_anchor == Some(identity) {
            self.selection_anchor = self
                .selected_entries
                .iter()
                .copied()
                .min_by_key(|identity| {
                    self.browser
                        .index_of_identity(*identity)
                        .unwrap_or(usize::MAX)
                });
        }
    }

    pub(crate) fn clear_selection(&mut self) {
        self.selected = None;
        self.selected_entries.clear();
        self.selection_anchor = None;
    }

    pub(crate) fn select_all(&mut self) {
        self.selected_entries = (0..self.browser.entries().len())
            .filter_map(|index| self.identity_at(index))
            .collect();
        self.selected = self
            .selected
            .filter(|identity| self.browser.index_of_identity(*identity).is_some())
            .or_else(|| self.identity_at(0));
        self.selection_anchor = self.selected;
    }

    /// The single, visual-order selection authority used by file commands.
    pub(crate) fn ordered_selection_snapshot(&self) -> Vec<PathBuf> {
        self.browser
            .entries()
            .iter()
            .enumerate()
            .filter(|(index, _)| self.is_index_selected(*index))
            .map(|(_, entry)| entry.path.clone())
            .collect()
    }

    pub(crate) fn selection_summary(&self) -> crate::selection_summary::SelectionSummary {
        crate::selection_summary::SelectionSummary::from_entries(
            self.browser
                .entries()
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| self.is_index_selected(index).then_some(entry)),
        )
    }

    fn close_properties(&mut self) {
        self.properties = None;
        self.properties_association = None;
        self.properties_association_rx = None;
        self.properties_association_status.clear();
        self.properties_handler = None;
        self.properties_confirm_default = false;
        self.properties_confirm_close = false;
        if let Some(job) = self.properties_size_job.take() {
            job.cancel();
        }
        self.properties_size_progress = None;
        self.properties_edits = None;
        self.properties_scroll = 0.0;
    }

    fn open_properties(&mut self, entry: &FileEntry, identity: Option<crate::FileIdentity>) {
        if self.properties.is_some() {
            return;
        }
        self.status.clear();
        match crate::properties::EntryProperties::load(entry, identity) {
            Ok(properties) => {
                self.properties_edits = Some(crate::properties::PropertyEdits {
                    readonly: properties.readonly,
                    hidden: properties.hidden,
                });
                self.properties = Some(properties);
            }
            Err(error) => {
                self.status = format!("Could not read properties: {error}");
                return;
            }
        }
        if entry.is_directory {
            self.properties_association_status.clear();
            return;
        }
        let path = entry.path.clone();
        let (sender, receiver) = mpsc::channel();
        self.properties_association_rx = Some(receiver);
        self.properties_association_status = "Loading applications…".into();
        std::thread::spawn(move || {
            let result = nickel_platform::association_target_for_file(&path)
                .and_then(|target| nickel_platform::association_service().inspect(&target))
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
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
                FileMessage::ToggleCommandSurface
                    | FileMessage::CommandQueryChanged(_)
                    | FileMessage::CommandScroll(_)
            )
        {
            self.command_surface_open = false;
            self.command_query.clear();
        }
        match message {
            FileMessage::BeginRename => {
                if self.selected_entries.len() == 1 {
                    let identity = *self.selected_entries.iter().next().unwrap();
                    if let Some(entry) = self
                        .browser
                        .index_of_identity(identity)
                        .and_then(|index| self.browser.entries().get(index))
                    {
                        self.rename_editor =
                            Some(RenameEditor::begin(identity, entry.path.clone()));
                        self.pending_focus = Some(UiId::from("file-rename-field"));
                    }
                }
            }
            FileMessage::ContextRename => {
                if let Some(path) = self.context_target.clone()
                    && let Some(index) = self
                        .browser
                        .entries()
                        .iter()
                        .position(|entry| entry.path == path)
                    && let Some(identity) = self.browser.identity_at(index)
                {
                    self.rename_editor = Some(RenameEditor::begin(identity, path));
                    self.pending_focus = Some(UiId::from("file-rename-field"));
                } else {
                    self.status = "The selected item is no longer available".into();
                }
            }
            FileMessage::RenameChanged(text) => {
                if let Some(editor) = &mut self.rename_editor {
                    editor.text = text
                }
            }
            FileMessage::CancelRename => self.rename_editor = None,
            FileMessage::CancelTransfer => {
                if let Some(cancel) = &self.transfer_cancel {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.status = "Cancelling transfer…".into();
                }
            }
            FileMessage::TransferKeepBoth => {
                self.resolve_transfer_conflicts(ConflictPolicy::KeepBoth)
            }
            FileMessage::TransferSkipConflicts => {
                self.resolve_transfer_conflicts(ConflictPolicy::Skip)
            }
            FileMessage::TransferCancelConflicts => {
                self.pending_transfer_conflict = None;
                self.status = "Transfer cancelled".into();
            }
            FileMessage::CommitRename => self.commit_rename(),
            FileMessage::CopySelection => self.capture_file_clipboard(TransferIntent::Copy),
            FileMessage::CutSelection => self.capture_file_clipboard(TransferIntent::Move),
            FileMessage::Paste => self.paste_file_clipboard(),
            FileMessage::ContextEntry(index) => {
                self.context_anchor = Some(self.cursor);
                self.context_target = self
                    .browser
                    .entries()
                    .get(index)
                    .map(|entry| entry.path.clone());
                if !self.is_index_selected(index) {
                    let Some(identity) = self.identity_at(index) else {
                        return;
                    };
                    self.selected_entries.clear();
                    self.selected_entries.insert(identity);
                    self.selected = Some(identity);
                    self.selection_anchor = Some(identity);
                }
                self.context_selection = self.ordered_selection_snapshot();
                self.selection_drag = None;
            }
            FileMessage::ContextBackground => {
                self.context_anchor = Some(self.cursor);
                self.context_target = None;
                self.context_selection.clear();
                self.selection_drag = None;
            }
            FileMessage::ContextOpen => {
                self.activate_context_target();
            }
            FileMessage::ContextOpenNewTab => {
                let entry = match self.context_target.as_ref() {
                    Some(path) => self
                        .browser
                        .entries()
                        .iter()
                        .find(|entry| &entry.path == path)
                        .cloned(),
                    None => self
                        .selected_index()
                        .and_then(|index| self.browser.entries().get(index))
                        .cloned(),
                };
                if let Some(entry) = entry
                    && (entry.is_directory || entry.path.is_dir())
                {
                    self.new_tab_at(entry.path);
                } else {
                    self.status = "The selected folder is no longer available".into();
                }
            }
            FileMessage::ContextCut => self
                .capture_paths_for_clipboard(TransferIntent::Move, self.context_selection.clone()),
            FileMessage::ContextCopy => self
                .capture_paths_for_clipboard(TransferIntent::Copy, self.context_selection.clone()),
            FileMessage::ContextPasteInto => {
                let destination = self
                    .context_target
                    .clone()
                    .unwrap_or_else(|| self.browser.current().to_path_buf());
                self.paste_file_clipboard_into(destination);
            }
            FileMessage::ContextNewFolder => {
                let parent = self
                    .context_target
                    .clone()
                    .unwrap_or_else(|| self.browser.current().to_path_buf());
                self.create_new_folder(&parent);
            }
            FileMessage::ContextCopyPath => {
                if let Some(path) = &self.context_target {
                    match crate::platform::publish_text_clipboard(&path.display().to_string()) {
                        Ok(()) => self.status = "Copied path".into(),
                        Err(error) => self.status = format!("Could not copy path: {error}"),
                    }
                }
            }
            FileMessage::ContextProperties => {
                let target = if let Some(path) = self.context_target.as_ref() {
                    self.browser
                        .entries()
                        .iter()
                        .enumerate()
                        .find(|(_, entry)| &entry.path == path)
                } else {
                    (self.selected_entries.len() == 1)
                        .then(|| {
                            self.selected_index().and_then(|index| {
                                self.browser
                                    .entries()
                                    .get(index)
                                    .map(|entry| (index, entry))
                            })
                        })
                        .flatten()
                };
                if let Some((index, entry)) = target.map(|(index, entry)| (index, entry.clone())) {
                    self.open_properties(&entry, self.browser.identity_at(index));
                } else {
                    self.status = "The selected item is no longer available".into();
                }
            }
            FileMessage::ContextCurrentFolderProperties => {
                let path = self.browser.current().to_path_buf();
                let entry = FileEntry {
                    display_name_override: None,
                    name: path.file_name().unwrap_or(path.as_os_str()).to_os_string(),
                    path: path.clone(),
                    is_directory: true,
                    size: None,
                    modified: None,
                };
                let identity = crate::file_identity(&path).ok();
                self.open_properties(&entry, identity);
            }
            FileMessage::CloseProperties => {
                let dirty = self
                    .properties
                    .as_ref()
                    .zip(self.properties_edits)
                    .is_some_and(|(properties, edits)| {
                        edits.readonly != properties.readonly || edits.hidden != properties.hidden
                    });
                if dirty {
                    self.properties_confirm_close = true;
                    return;
                }
                self.close_properties();
            }
            FileMessage::DiscardProperties => self.close_properties(),
            FileMessage::PropertiesSelectHandler(index) => {
                if self
                    .properties_association
                    .as_ref()
                    .is_some_and(|snapshot| index < snapshot.handlers.len())
                {
                    self.properties_handler = Some(index);
                }
            }
            FileMessage::PropertiesOpenOnce => {
                if let (Some(properties), Some(snapshot), Some(index)) = (
                    self.properties.as_ref(),
                    self.properties_association.as_ref(),
                    self.properties_handler,
                ) && let Some(handler) = snapshot.handlers.get(index)
                    && let Err(error) = nickel_platform::open_once_with(&properties.path, handler)
                {
                    self.status = format!("Could not open with {}: {error}", handler.name);
                }
            }
            FileMessage::PropertiesRequestDefault => self.properties_confirm_default = true,
            FileMessage::PropertiesConfirmDefault => {
                self.properties_confirm_default = false;
                if let (Some(snapshot), Some(index)) = (
                    self.properties_association.as_ref(),
                    self.properties_handler,
                ) && let Some(handler) = snapshot.handlers.get(index)
                {
                    match nickel_platform::association_service()
                        .request_change(&snapshot.target, &handler.id)
                    {
                        Ok(nickel_platform::ChangeOutcome::Confirmed(updated)) => {
                            self.properties_association = Some(updated);
                            self.icons.clear();
                            self.refresh_icons();
                        }
                        Ok(nickel_platform::ChangeOutcome::NativeConsentRequired { detail })
                        | Ok(nickel_platform::ChangeOutcome::Rejected { detail }) => {
                            self.status = detail
                        }
                        Err(error) => {
                            self.status = format!("Could not change default application: {error}")
                        }
                    }
                }
            }
            FileMessage::PropertiesDefaultSettings => {
                if let Err(error) = nickel_platform::open_default_application_settings() {
                    self.status = format!("Could not open default application settings: {error}");
                }
            }
            FileMessage::PropertiesCalculateSize => {
                if let Some(properties) = self
                    .properties
                    .as_ref()
                    .filter(|value| value.kind == "Folder")
                {
                    self.properties_size_job = Some(crate::properties::calculate_recursive_size(
                        properties.path.clone(),
                    ));
                    self.properties_size_progress = Some("Calculating…".into());
                }
            }
            FileMessage::PropertiesCancelSize => {
                if let Some(job) = self.properties_size_job.take() {
                    job.cancel();
                }
                self.properties_size_progress = Some("Calculation cancelled".into());
            }
            FileMessage::PropertiesToggleReadonly => {
                if let Some(edits) = self.properties_edits.as_mut() {
                    edits.readonly = !edits.readonly;
                }
            }
            FileMessage::PropertiesToggleHidden => {
                if let Some(edits) = self.properties_edits.as_mut() {
                    edits.hidden = !edits.hidden;
                }
            }
            FileMessage::PropertiesApply => {
                if let (Some(properties), Some(edits)) =
                    (self.properties.as_ref(), self.properties_edits)
                {
                    let outcome = crate::properties::apply_edits(properties, edits);
                    if outcome.readonly.is_ok() && outcome.hidden.is_ok() {
                        self.status = "Properties applied".into();
                        let entry = FileEntry {
                            display_name_override: None,
                            name: outcome.path.file_name().unwrap_or_default().to_owned(),
                            path: outcome.path.clone(),
                            is_directory: outcome.path.is_dir(),
                            size: None,
                            modified: None,
                        };
                        self.properties =
                            crate::properties::EntryProperties::load(&entry, properties.identity)
                                .ok();
                        self.properties_edits = self.properties.as_ref().map(|value| {
                            crate::properties::PropertyEdits {
                                readonly: value.readonly,
                                hidden: value.hidden,
                            }
                        });
                        self.refresh_directory(self.browser.show_hidden());
                    } else {
                        self.status = format!(
                            "Some properties were not applied: read-only: {:?}; hidden: {:?}",
                            outcome.readonly.err(),
                            outcome.hidden.err()
                        );
                    }
                }
            }
            FileMessage::PropertiesOk => {
                if let (Some(properties), Some(edits)) =
                    (self.properties.as_ref(), self.properties_edits)
                {
                    let outcome = crate::properties::apply_edits(properties, edits);
                    if outcome.readonly.is_ok() && outcome.hidden.is_ok() {
                        self.close_properties();
                        self.status = "Properties applied".into();
                        self.refresh_directory(self.browser.show_hidden());
                    } else {
                        self.status = format!(
                            "Some properties were not applied: read-only: {:?}; hidden: {:?}",
                            outcome.readonly.err(),
                            outcome.hidden.err()
                        );
                    }
                }
            }
            FileMessage::PropertiesScroll(offset) => self.properties_scroll = offset.max(0.0),
            FileMessage::ContextRefresh => {
                self.refresh_directory(self.browser.show_hidden());
            }
            FileMessage::ContextSelectAll => {
                self.select_all();
            }
            FileMessage::ToggleCommandSurface => {
                self.context_target = None;
                self.context_anchor = None;
                self.command_surface_open = !self.command_surface_open;
                self.command_query.clear();
                self.command_scroll_offset = 0.0;
                if self.command_surface_open {
                    self.places_open = false;
                    self.pending_focus = Some(UiId::from("file-command-query"));
                }
            }
            FileMessage::CommandQueryChanged(query) => {
                self.command_query = query;
                self.command_scroll_offset = 0.0;
            }
            FileMessage::CommandScroll(offset) => self.command_scroll_offset = offset.max(0.0),
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
            FileMessage::AdjustTileWidth(direction) => {
                self.tile_width = (self.tile_width + f32::from(direction.signum()) * 12.0)
                    .clamp(MIN_TILE_WIDTH, MAX_TILE_WIDTH);
                self.ensure_selection_visible();
            }
            FileMessage::SortBy(key) => self.sort_by(key),
            FileMessage::CloseTab(index) => self.close_tab(index),
            FileMessage::SwitchTab(index) => self.switch_tab(index),
            FileMessage::ToggleFolder(path) => {
                self.toggle_sidebar_folder(path);
            }
            FileMessage::OpenFolder(path) | FileMessage::Breadcrumb(path) => {
                self.places_open = false;
                self.navigate_to(path);
            }
            FileMessage::Entry(index) => {
                self.context_target = None;
                self.context_anchor = None;
                let now = Instant::now();
                let entry_path = self
                    .browser
                    .entries()
                    .get(index)
                    .map(|entry| entry.path.clone());
                // Revalidate at the input boundary: a native watch may not have
                // reconciled a remove-and-recreate burst before the second press.
                let entry_identity = entry_path
                    .as_deref()
                    .and_then(|path| crate::file_identity(path).ok())
                    .or_else(|| self.browser.identity_at(index));
                let activate = entry_path.as_ref().is_some_and(|entry_path| {
                    self.last_click.as_ref().is_some_and(|previous| {
                        previous.path == *entry_path
                            // A path can be removed and replaced between presses. Stable
                            // provider identity prevents the replacement from inheriting a
                            // click transaction. Fixtures/providers without identities retain
                            // the conservative path contract.
                            && (entry_identity.is_none()
                                || previous.identity.is_none()
                                || previous.identity == entry_identity)
                            && now.duration_since(previous.when) <= DOUBLE_CLICK_INTERVAL
                            && (self.cursor.x - previous.position.x).abs() <= DOUBLE_CLICK_DISTANCE
                            && (self.cursor.y - previous.position.y).abs() <= DOUBLE_CLICK_DISTANCE
                    })
                });
                if self.shift_down {
                    self.select_range(index, self.control_down);
                } else if self.control_down {
                    let Some(identity) = self.identity_at(index) else {
                        return;
                    };
                    if !self.selected_entries.remove(&identity) {
                        self.selected_entries.insert(identity);
                        self.selected = Some(identity);
                        self.selection_anchor = Some(identity);
                    } else {
                        self.selected = self.selected_entries.iter().copied().min_by_key(|id| {
                            self.browser.index_of_identity(*id).unwrap_or(usize::MAX)
                        });
                        if self.selection_anchor == Some(identity) {
                            self.selection_anchor = self.selected;
                        }
                    }
                } else {
                    self.select_only(index);
                }
                self.last_click = entry_path.map(|path| FileClick {
                    path,
                    identity: entry_identity,
                    position: self.cursor,
                    when: now,
                });
                if activate && !self.control_down && !self.shift_down {
                    self.activate_selected();
                    self.last_click = None;
                }
            }
            FileMessage::SelectionSurface => {
                self.selection_drag = Some(self.cursor);
                if !self.control_down && !self.shift_down {
                    self.clear_selection();
                }
            }
            FileMessage::FileScroll(offset) => self.file_scroll_offset = offset.max(0.0),
        }
    }

    fn capture_file_clipboard(&mut self, intent: TransferIntent) {
        let paths = self.ordered_selection_snapshot();
        self.capture_paths_for_clipboard(intent, paths);
    }

    fn capture_paths_for_clipboard(&mut self, intent: TransferIntent, paths: Vec<PathBuf>) {
        let sources = paths
            .into_iter()
            .filter_map(|path| {
                let index = self
                    .browser
                    .entries()
                    .iter()
                    .position(|entry| entry.path == path)?;
                Some(TransferSource {
                    provider: "local".into(),
                    identity: self.browser.identity_at(index)?,
                    path,
                    capabilities: ItemCapabilities {
                        readable: true,
                        removable: true,
                    },
                })
            })
            .collect();
        match ClipboardOffer::new(intent, sources) {
            Ok(offer) => {
                self.status = if intent == TransferIntent::Move {
                    "Ready to move selection"
                } else {
                    "Copied selection"
                }
                .into();
                let paths = offer
                    .sources
                    .iter()
                    .map(|source| source.path.clone())
                    .collect::<Vec<_>>();
                if let Err(error) =
                    crate::platform::publish_file_clipboard(&paths, intent == TransferIntent::Move)
                {
                    tracing::debug!(%error, "native file clipboard unavailable");
                }
                self.file_clipboard = Some(offer);
            }
            Err(_) => self.status = "Nothing eligible is selected".into(),
        }
    }

    pub(crate) fn begin_file_drag_if_threshold(&mut self, cursor: Point) {
        if !self.primary_down || self.outbound_drag.is_some() {
            return;
        }
        let Some(click) = &self.last_click else {
            return;
        };
        let distance =
            ((click.position.x - cursor.x).powi(2) + (click.position.y - cursor.y).powi(2)).sqrt();
        if distance < 6.0 {
            return;
        }
        let Some(clicked) = self
            .browser
            .entries()
            .iter()
            .position(|entry| entry.path == click.path)
        else {
            return;
        };
        if !self.is_index_selected(clicked) {
            return;
        }
        let sources = self
            .ordered_selection_snapshot()
            .into_iter()
            .filter_map(|path| {
                let index = self
                    .browser
                    .entries()
                    .iter()
                    .position(|entry| entry.path == path)?;
                Some(TransferSource {
                    provider: "local".into(),
                    identity: self.browser.identity_at(index)?,
                    path,
                    capabilities: ItemCapabilities {
                        readable: true,
                        removable: true,
                    },
                })
            })
            .collect();
        self.outbound_drag = DragOffer::bounded(sources).ok();
    }

    fn commit_rename(&mut self) {
        if self.rename_rx.is_some() {
            return;
        }
        let names = self
            .browser
            .entries()
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>();
        let Some(editor) = &mut self.rename_editor else {
            return;
        };
        match editor.commit(names) {
            Ok(None) => self.rename_editor = None,
            Ok(Some(OperationEffect::Rename { identity, from, to })) => {
                let current = self
                    .browser
                    .entries()
                    .iter()
                    .position(|entry| entry.path == from)
                    .and_then(|index| self.browser.identity_at(index));
                if current != Some(identity) {
                    self.status = "Rename cancelled because the file changed".into();
                    return;
                }
                let (sender, receiver) = mpsc::channel();
                self.rename_rx = Some(receiver);
                self.status = "Renaming…".into();
                std::thread::spawn(move || {
                    let result = std::fs::rename(&from, &to).map_err(|error| error.to_string());
                    let _ = sender.send((identity, to, result));
                });
            }
            Ok(Some(_)) => {}
            Err(error) => self.status = format!("Invalid name: {error:?}"),
        }
    }

    fn paste_file_clipboard(&mut self) {
        self.paste_file_clipboard_into(self.browser.current().to_path_buf());
    }

    fn paste_file_clipboard_into(&mut self, destination: PathBuf) {
        let offer = self.file_clipboard.clone().or_else(|| {
            crate::platform::read_file_clipboard()
                .ok()
                .and_then(|(cut, paths)| {
                    let sources = paths
                        .into_iter()
                        .enumerate()
                        .map(|(index, path)| TransferSource {
                            provider: "native".into(),
                            identity: crate::FileIdentity(0, index as u64),
                            path,
                            capabilities: ItemCapabilities {
                                readable: true,
                                removable: cut,
                            },
                        })
                        .collect();
                    ClipboardOffer::new(
                        if cut {
                            TransferIntent::Move
                        } else {
                            TransferIntent::Copy
                        },
                        sources,
                    )
                    .ok()
                })
        });
        let Some(offer) = offer else {
            self.status = "The file clipboard is empty".into();
            return;
        };
        self.start_transfer(offer, destination);
    }

    fn start_transfer(&mut self, offer: ClipboardOffer, destination: PathBuf) {
        if self.transfer_rx.is_some() {
            self.status = "Wait for the current file operation to finish".into();
            return;
        }
        if self.pending_transfer_conflict.is_some() {
            self.status = "Resolve or cancel the current file conflicts first".into();
            return;
        }
        let writable = crate::directory_is_writable(&destination);
        if !writable {
            self.status = "Paste target is not writable".into();
            return;
        }
        let conflicts = offer
            .sources
            .iter()
            .filter_map(|source| source.path.file_name().map(|name| destination.join(name)))
            .filter(|target| target.exists())
            .collect::<Vec<_>>();
        if !conflicts.is_empty() {
            self.status = format!(
                "{} item{} already exist{} in the destination",
                conflicts.len(),
                if conflicts.len() == 1 { "" } else { "s" },
                if conflicts.len() == 1 { "s" } else { "" },
            );
            self.pending_transfer_conflict = Some(PendingTransferConflict {
                offer,
                destination,
                conflicts,
            });
            return;
        }
        self.start_transfer_with_policy(offer, destination, ConflictPolicy::Ask);
    }

    fn resolve_transfer_conflicts(&mut self, policy: ConflictPolicy) {
        let Some(pending) = self.pending_transfer_conflict.take() else {
            return;
        };
        self.start_transfer_with_policy(pending.offer, pending.destination, policy);
    }

    fn start_transfer_with_policy(
        &mut self,
        offer: ClipboardOffer,
        destination: PathBuf,
        conflict_policy: ConflictPolicy,
    ) {
        let total = offer.sources.len();
        let writable = crate::directory_is_writable(&destination);
        let Ok(effect) =
            crate::operations::plan_paste(&offer, "local", &destination, writable, conflict_policy)
        else {
            self.status = "Paste target is not valid".into();
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.transfer_rx = Some(receiver);
        self.status = format!("Transferring 0 of {total} items…");
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.transfer_cancel = Some(cancelled.clone());
        std::thread::spawn(move || {
            let progress_sender = sender.clone();
            let report =
                crate::operations::execute_local_transfer(&effect, &cancelled, |done, _| {
                    let _ = progress_sender.send((offer.intent, total, done, None));
                });
            let _ = sender.send((offer.intent, total, total, Some(report)));
        });
    }

    fn create_new_folder(&mut self, parent: &std::path::Path) {
        match nickel_file::create_new_folder(parent) {
            Ok(path) => {
                self.status = format!(
                    "Created {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                );
                self.refresh_directory(self.browser.show_hidden());
            }
            Err(error) => {
                self.status = format!("Could not create folder: {error}");
            }
        }
    }

    fn poll_transfer(&mut self) -> bool {
        let Some(receiver) = &self.transfer_rx else {
            return false;
        };
        let Ok((intent, total, completed_progress, report)) = receiver.try_recv() else {
            return false;
        };
        let Some(report) = report else {
            self.status = format!("Transferring {completed_progress} of {total} items…");
            return true;
        };
        let completed = report.affected.len();
        if intent == TransferIntent::Move && completed == total && report.failed.is_empty() {
            self.file_clipboard = None;
        }
        self.status = if report.cancelled {
            "Transfer cancelled".into()
        } else if report.failed.is_empty() {
            format!("Transferred {completed} items")
        } else {
            format!(
                "Transferred {completed} of {total} items; {} failed",
                report.failed.len()
            )
        };
        self.transfer_rx = None;
        self.transfer_cancel = None;
        self.refresh_directory(self.browser.show_hidden());
        true
    }

    fn poll_rename(&mut self) -> bool {
        let Some(receiver) = &self.rename_rx else {
            return false;
        };
        let (identity, to, result) = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => {
                self.rename_rx = None;
                self.status = "Rename worker stopped unexpectedly".into();
                return true;
            }
        };
        self.rename_rx = None;
        match result {
            Ok(()) => {
                self.rename_editor = None;
                self.refresh_directory(self.browser.show_hidden());
                if let Some(index) = self
                    .browser
                    .entries()
                    .iter()
                    .position(|entry| entry.path == to)
                {
                    self.selected = self.identity_at(index);
                    self.selected_entries.clear();
                    self.selected_entries.extend(self.identity_at(index));
                    self.selection_anchor = self.identity_at(index);
                }
                self.status = format!(
                    "Renamed to {}",
                    to.file_name().unwrap_or_default().to_string_lossy()
                );
            }
            Err(error) => {
                if self
                    .rename_editor
                    .as_ref()
                    .is_some_and(|editor| editor.identity == identity)
                {
                    self.status = format!("Could not rename: {error}");
                }
            }
        }
        true
    }

    fn poll_native_drop(&mut self) -> bool {
        if self
            .native_drop_deadline
            .is_none_or(|deadline| Instant::now() < deadline)
        {
            return false;
        }
        self.native_drop_deadline = None;
        let sources = std::mem::take(&mut self.native_drop_batch)
            .into_iter()
            .enumerate()
            .map(|(index, path)| TransferSource {
                provider: "native-drag".into(),
                identity: crate::FileIdentity(0, index as u64),
                path,
                capabilities: ItemCapabilities {
                    readable: true,
                    removable: self.native_drop_intent == TransferIntent::Move,
                },
            })
            .collect();
        let intent = self.native_drop_intent;
        self.native_drop_intent = TransferIntent::Copy;
        match ClipboardOffer::new(intent, sources) {
            Ok(offer) => {
                let destination = self
                    .native_drop_batch_destination
                    .take()
                    .unwrap_or_else(|| self.browser.current().to_path_buf());
                self.start_transfer(offer, destination);
            }
            Err(error) => self.status = format!("Could not accept drop: {error:?}"),
        }
        true
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

    fn file_drag_event(&mut self, event: nickel_ui::FileDragEvent) -> bool {
        match event {
            nickel_ui::FileDragEvent::Hovered(path) => self.drag_hover = Some(path),
            nickel_ui::FileDragEvent::HoverCancelled => {
                self.drag_hover = None;
                self.native_drop_destination = None;
                self.native_drop_hover_started = None;
                self.native_drop_intent = TransferIntent::Copy;
            }
            nickel_ui::FileDragEvent::ActionChanged(action) => {
                // Native backends can negotiate an action before URI payload
                // transfer; this also marks the hover lifecycle active so
                // semantic target affordances and delayed navigation work.
                self.drag_hover.get_or_insert_with(PathBuf::new);
                self.native_drop_intent = match action {
                    nickel_ui::FileDragAction::Copy => TransferIntent::Copy,
                    nickel_ui::FileDragAction::Move => TransferIntent::Move,
                };
            }
            nickel_ui::FileDragEvent::Dropped(path) => {
                self.drag_hover = None;
                if path.file_name().is_none() {
                    return false;
                }
                if self.native_drop_batch.is_empty() {
                    self.native_drop_batch_destination = self.native_drop_destination.clone();
                }
                self.native_drop_batch.push(path);
                self.native_drop_destination = None;
                self.native_drop_hover_started = None;
                // Winit reports one DroppedFile event per path. Coalesce the
                // burst so a native multi-file drag becomes one bounded operation.
                self.native_drop_deadline = Some(Instant::now() + Duration::from_millis(25));
            }
        }
        true
    }

    fn take_outbound_file_drag(&mut self) -> Option<nickel_ui::OutboundFileDrag> {
        let offer = self.outbound_drag.take()?;
        Some(nickel_ui::OutboundFileDrag {
            paths: offer
                .sources
                .into_iter()
                .map(|source| source.path)
                .collect(),
        })
    }

    fn frame_overlays(&self, context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        let file_surface_is_mounted = !(self.command_surface_open
            || context.viewport.size.width < NARROW_WORKSPACE_BREAKPOINT && self.places_open);
        if !file_surface_is_mounted {
            return Vec::new();
        }
        let appearance =
            ShellSettings::load_default().resolve_appearance(nickel_platform::appearance());
        let palette = ThemePalette::from_appearance(appearance);
        let invocation_anchor = |target: UiId| match context.modality {
            nickel_ui::InputModality::Pointer => OverlayAnchor::Point {
                invocation_target: target,
                point: self.context_anchor.unwrap_or(self.cursor),
            },
            nickel_ui::InputModality::Keyboard
            | nickel_ui::InputModality::Controller
            | nickel_ui::InputModality::Accessibility => {
                OverlayAnchor::InvocationTargetCenter(target)
            }
        };
        let configure = |mut menu: OverlayMenu<FileMessage>| {
            menu.width = 250.0;
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
            .map(|(index, entry)| {
                let menu = OverlayMenu::new(
                    entry_context_menu_id(&entry.path),
                    invocation_anchor(UiId::new(format!("file-entry-{index}"))),
                )
                .item(
                    OverlayMenuItem::action(
                        "open",
                        self.localizer.text("file-command-open"),
                        FileMessage::ContextOpen,
                    )
                    .shortcut("Enter"),
                )
                .item(OverlayMenuItem::action(
                    "open-with",
                    "Open With",
                    FileMessage::ContextProperties,
                ));
                let menu = if entry.is_directory {
                    menu.item(
                        OverlayMenuItem::action(
                            "open-new-tab",
                            self.localizer.text("file-command-open-new-tab"),
                            FileMessage::ContextOpenNewTab,
                        )
                        .shortcut("Ctrl+Enter"),
                    )
                    .item(OverlayMenuItem::disabled_with_reason(
                        "bookmark",
                        "Add to Bookmarks",
                        "Bookmark editing is not implemented yet",
                    ))
                } else {
                    menu
                };
                let menu = menu
                    .item(
                        OverlayMenuItem::action("cut", "Cut", FileMessage::ContextCut)
                            .shortcut("Ctrl+X")
                            .separator_before(true),
                    )
                    .item(
                        OverlayMenuItem::action("copy", "Copy", FileMessage::ContextCopy)
                            .shortcut("Ctrl+C"),
                    );
                let menu = if entry.is_directory {
                    menu.item(
                        OverlayMenuItem::action(
                            "paste-into",
                            "Paste into Folder",
                            FileMessage::ContextPasteInto,
                        )
                        .shortcut("Ctrl+V"),
                    )
                    .item(OverlayMenuItem::action(
                        "new-folder",
                        "New Folder",
                        FileMessage::ContextNewFolder,
                    ))
                } else {
                    menu
                };
                FrameOverlay::Menu(configure(
                    menu.item(
                        OverlayMenuItem::action("rename", "Rename", FileMessage::ContextRename)
                            .shortcut("F2")
                            .separator_before(true),
                    )
                    .item(OverlayMenuItem::disabled_with_reason(
                        "trash",
                        "Move to Trash",
                        "Trash integration is not implemented yet",
                    ))
                    .item(
                        OverlayMenuItem::action(
                            "copy-path",
                            "Copy Path",
                            FileMessage::ContextCopyPath,
                        )
                        .separator_before(true),
                    )
                    .item(OverlayMenuItem::disabled_with_reason(
                        "open-terminal",
                        "Open in Terminal",
                        "Terminal integration is not implemented yet",
                    ))
                    .item(OverlayMenuItem::action(
                        "properties",
                        "Properties",
                        FileMessage::ContextProperties,
                    )),
                ))
            })
            .collect::<Vec<_>>();
        if let Some(editor) = &self.rename_editor {
            let selected = self.selected_index().unwrap_or(0);
            let surface = TransientSurface::dialog(
                "file-rename",
                OverlayAnchor::InvocationTargetCenter(UiId::new(format!("file-entry-{selected}"))),
                Size::new(360.0, 92.0),
                OverlayStyle {
                    background: palette.surface,
                    foreground: palette.text,
                    border: palette.accent,
                    selected: palette.accent_soft,
                    radius: 8,
                },
            );
            overlays.push(FrameOverlay::surface(surface, ui! {
                <Column gap={8.0} padding={Insets::all(10.0)}>
                    {TextField::on_change(&editor.text, FileMessage::RenameChanged).id("file-rename-field").color(palette.text)}
                    <Row gap={8.0}>
                        <Button on_press={FileMessage::CommitRename}>{"Rename"}</Button>
                        <Button on_press={FileMessage::CancelRename}>{"Cancel"}</Button>
                    </Row>
                </Column>
            }));
        }
        if let Some(pending) = &self.pending_transfer_conflict {
            let surface = TransientSurface::dialog(
                "file-transfer-conflicts",
                OverlayAnchor::Node(UiId::from("file-content")),
                Size::new(440.0, 180.0),
                OverlayStyle {
                    background: palette.surface,
                    foreground: palette.text,
                    border: palette.muted,
                    selected: palette.accent_soft,
                    radius: 8,
                },
            );
            let count = pending.conflicts.len();
            overlays.push(FrameOverlay::surface(surface, ui! {
                <Container semantic_role={nickel_ui::SemanticRole::Dialog}
                    accessibility_label={format!("Resolve {count} file transfer conflicts")}
                    background={palette.surface} padding={Insets::all(18.0)}>
                    <Column gap={12.0}>
                        <Text color={palette.text} scale={1.25}>{format!("{count} item{} already exist{}", if count == 1 { "" } else { "s" }, if count == 1 { "s" } else { "" })}</Text>
                        <Text color={palette.muted}> {"Choose how Nickel File should handle every conflicting name in this transfer."} </Text>
                        <Row gap={8.0}>
                            <Button on_press={FileMessage::TransferKeepBoth}> {"Keep both"} </Button>
                            <Button on_press={FileMessage::TransferSkipConflicts}> {"Skip conflicts"} </Button>
                            <Button on_press={FileMessage::TransferCancelConflicts}> {"Cancel transfer"} </Button>
                        </Row>
                    </Column>
                </Container>
            }));
        }
        overlays.push(FrameOverlay::Menu(configure(
            OverlayMenu::new(
                "file-background-context",
                invocation_anchor(UiId::from("file-content")),
            )
            .item(
                OverlayMenuItem::action(
                    "refresh",
                    self.localizer.text("file-command-refresh"),
                    FileMessage::ContextRefresh,
                )
                .shortcut("F5"),
            )
            .item(
                OverlayMenuItem::action(
                    "select-all",
                    self.localizer.text("file-command-select-all"),
                    FileMessage::ContextSelectAll,
                )
                .shortcut("Ctrl+A"),
            )
            .item(
                OverlayMenuItem::action("paste", "Paste", FileMessage::ContextPasteInto)
                    .shortcut("Ctrl+V")
                    .separator_before(true),
            )
            .item(OverlayMenuItem::action(
                "new-folder",
                "New Folder",
                FileMessage::ContextNewFolder,
            ))
            .item(OverlayMenuItem::action(
                "properties",
                "Properties",
                FileMessage::ContextCurrentFolderProperties,
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
        if let Some(properties) = self.properties.as_ref() {
            let surface = TransientSurface::dialog(
                "file-properties-dialog",
                OverlayAnchor::Node(UiId::from("file-content")),
                Size::new(500.0, 620.0),
                OverlayStyle {
                    background: palette.surface,
                    foreground: palette.text,
                    border: palette.muted,
                    selected: palette.accent_soft,
                    radius: 8,
                },
            );
            overlays.push(FrameOverlay::surface(
                surface,
                crate::components::properties_dialog(self, properties, palette),
            ));
        }
        overlays
    }

    fn poll(&mut self) -> bool {
        let settings_changed = self.sync_icon_settings();
        let before = self.next_icon_id;
        self.poll_icons();
        let properties_changed = if let Some(job) = self.properties_size_job.as_ref() {
            match job.receiver.try_recv() {
                Ok(crate::properties::RecursiveSizeUpdate::Progress { entries, bytes }) => {
                    self.properties_size_progress = Some(format!(
                        "{entries} entries · {}",
                        self.localizer.bytes(bytes)
                    ));
                    true
                }
                Ok(crate::properties::RecursiveSizeUpdate::Complete(bytes)) => {
                    self.properties_size_progress =
                        Some(format!("Folder contents: {}", self.localizer.bytes(bytes)));
                    self.properties_size_job = None;
                    true
                }
                Ok(crate::properties::RecursiveSizeUpdate::Failed(error)) => {
                    self.properties_size_progress =
                        Some(format!("Could not calculate size: {error}"));
                    self.properties_size_job = None;
                    true
                }
                Ok(crate::properties::RecursiveSizeUpdate::Cancelled) => {
                    self.properties_size_progress = Some("Calculation cancelled".into());
                    self.properties_size_job = None;
                    true
                }
                Err(TryRecvError::Disconnected) => {
                    self.properties_size_job = None;
                    true
                }
                Err(TryRecvError::Empty) => false,
            }
        } else {
            false
        };
        let association_changed = if let Some(receiver) = self.properties_association_rx.as_ref() {
            match receiver.try_recv() {
                Ok(Ok(snapshot)) => {
                    self.properties_handler = snapshot.effective.as_ref().and_then(|effective| {
                        snapshot
                            .handlers
                            .iter()
                            .position(|handler| handler.id == effective.id)
                    });
                    self.properties_association = Some(snapshot);
                    self.properties_association_status.clear();
                    self.properties_association_rx = None;
                    true
                }
                Ok(Err(error)) => {
                    self.properties_association_status = error;
                    self.properties_association_rx = None;
                    true
                }
                Err(TryRecvError::Disconnected) => {
                    self.properties_association_rx = None;
                    true
                }
                Err(TryRecvError::Empty) => false,
            }
        } else {
            false
        };
        settings_changed
            || properties_changed
            || association_changed
            || self.poll_activation()
            || self.poll_rename()
            || self.poll_native_drop()
            || self.poll_transfer()
            || self.poll_navigation()
            || self.poll_directory_watch()
            || self.poll_sidebar_children()
            || self.poll_location_groups()
            || before != self.next_icon_id
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        [
            Some(SETTINGS_SYNC_INTERVAL),
            (self.directory_watch.is_some() || self.directory_watch_retry_at.is_some())
                .then_some(DIRECTORY_WATCH_POLL_INTERVAL),
            self.icon_rx.as_ref().map(|_| self.icon_poll_delay),
            self.navigation_rx
                .as_ref()
                .map(|_| self.navigation_poll_delay),
            self.activation_rx
                .as_ref()
                .map(|_| Duration::from_millis(16)),
            self.transfer_rx.as_ref().map(|_| Duration::from_millis(16)),
            self.rename_rx.as_ref().map(|_| Duration::from_millis(16)),
            self.native_drop_deadline
                .map(|deadline| deadline.saturating_duration_since(Instant::now())),
            self.native_drop_hover_started.as_ref().map(|(_, started)| {
                (*started + Duration::from_millis(700)).saturating_duration_since(Instant::now())
            }),
            (!self.sidebar_loading.is_empty()).then_some(Duration::from_millis(16)),
            self.location_groups_rx
                .as_ref()
                .map(|_| Duration::from_millis(16)),
            self.properties_size_job
                .as_ref()
                .map(|_| Duration::from_millis(16)),
            self.properties_association_rx
                .as_ref()
                .map(|_| Duration::from_millis(16)),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn scale_factor_changed(&mut self, scale_factor: f32) -> bool {
        let scale_milli = (scale_factor.clamp(0.25, 8.0) * 1_000.0).round() as u16;
        if scale_milli == self.artwork_scale_milli {
            return false;
        }
        self.artwork_scale_milli = scale_milli;
        self.icons.clear();
        self.tab_icon = None;
        self.refresh_icons();
        true
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
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments.next();
    let application = if first.as_deref() == Some(std::ffi::OsStr::new("--properties")) {
        FileApp::launch_properties(
            arguments
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(home_directory),
        )
    } else if first.as_deref() == Some(std::ffi::OsStr::new("--rename")) {
        FileApp::launch_rename(
            arguments
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(home_directory),
        )
    } else {
        FileApp::launch(first.map(PathBuf::from).unwrap_or_else(home_directory))
    };
    nickel_ui::run_with_adapter(application, FileHostAdapter::default())
}

#[cfg(test)]
mod live_reconciliation_tests {
    use std::{
        collections::HashSet,
        fs, thread,
        time::{Duration, Instant},
    };

    use nickel_file::DirectoryBrowser;

    use crate::operations::{ClipboardOffer, ItemCapabilities, TransferIntent, TransferSource};

    use super::{FileApp, FileMessage};

    fn settle_live_change(app: &mut FileApp) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            app.poll_directory_watch();
            app.poll_navigation();
            if app.navigation_rx.is_none()
                && app
                    .directory_watch
                    .as_ref()
                    .is_none_or(|watch| !watch.take_invalidation())
            {
                return;
            }
            assert!(Instant::now() < deadline, "live reconciliation timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_listing(app: &mut FileApp, matches: impl Fn(&DirectoryBrowser) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !matches(&app.browser) || app.navigation_rx.is_some() {
            app.poll_directory_watch();
            app.poll_navigation();
            assert!(Instant::now() < deadline, "live listing did not settle");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn native_invalidation_reconciles_the_visible_directory() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = FileApp::new(directory.path().to_path_buf());
        settle_live_change(&mut app);

        fs::write(directory.path().join("appeared.txt"), b"new").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !app
            .browser
            .entries()
            .iter()
            .any(|entry| entry.display_name() == "appeared.txt")
        {
            app.poll_directory_watch();
            app.poll_navigation();
            assert!(Instant::now() < deadline, "created file did not appear");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn live_reconciliation_tracks_move_in_metadata_move_out_and_delete() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mut app = FileApp::new(directory.path().to_path_buf());
        settle_live_change(&mut app);

        let source = outside.path().join("moving.txt");
        let inside = directory.path().join("moving.txt");
        fs::write(&source, b"one").unwrap();
        fs::rename(&source, &inside).unwrap();
        wait_for_listing(&mut app, |browser| browser.entries().len() == 1);
        assert_eq!(app.browser.entries()[0].size, Some(3));

        fs::write(&inside, b"expanded").unwrap();
        wait_for_listing(&mut app, |browser| {
            browser.entries().first().and_then(|entry| entry.size) == Some(8)
        });
        assert_eq!(app.browser.entries()[0].size, Some(8));

        fs::rename(&inside, &source).unwrap();
        wait_for_listing(&mut app, |browser| browser.entries().is_empty());
        assert!(app.browser.entries().is_empty());

        fs::write(directory.path().join("deleted.txt"), b"gone").unwrap();
        wait_for_listing(&mut app, |browser| browser.entries().len() == 1);
        fs::remove_file(directory.path().join("deleted.txt")).unwrap();
        wait_for_listing(&mut app, |browser| browser.entries().is_empty());
        assert!(app.browser.entries().is_empty());
    }

    #[test]
    fn external_rename_preserves_focus_selection_and_anchor() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.txt");
        let renamed = directory.path().join("renamed.txt");
        fs::write(&original, b"content").unwrap();
        let mut app = FileApp::new(directory.path().to_path_buf());
        let identity = app.browser.identity_at(0).unwrap();
        app.selected = Some(identity);
        app.selected_entries = HashSet::from([identity]);
        app.selection_anchor = Some(identity);

        fs::rename(&original, &renamed).unwrap();
        let refreshed = DirectoryBrowser::open(directory.path()).unwrap();
        app.browser = refreshed;
        app.reconciliation_changed(
            Some((Some(identity), original.clone())),
            vec![(Some(identity), original.clone())],
            Some((Some(identity), original)),
        );

        assert_eq!(app.selected, Some(identity));
        assert_eq!(app.selected_entries, HashSet::from([identity]));
        assert_eq!(app.selection_anchor, Some(identity));
        assert_eq!(app.browser.entries()[0].path, renamed);
    }

    #[test]
    fn removed_selection_is_repaired_without_selecting_a_neighbor() {
        let directory = tempfile::tempdir().unwrap();
        let removed = directory.path().join("a.txt");
        fs::write(&removed, b"a").unwrap();
        fs::write(directory.path().join("b.txt"), b"b").unwrap();
        let mut app = FileApp::new(directory.path().to_path_buf());
        let identity = app.browser.identity_at(0).unwrap();

        fs::remove_file(&removed).unwrap();
        app.browser = DirectoryBrowser::open(directory.path()).unwrap();
        app.reconciliation_changed(
            Some((Some(identity), removed.clone())),
            vec![(Some(identity), removed.clone())],
            Some((Some(identity), removed)),
        );

        assert_eq!(app.selected, None);
        assert!(app.selected_entries.is_empty());
        assert_eq!(app.selection_anchor, None);
    }

    #[test]
    fn atomic_replacement_does_not_transfer_selection_to_a_new_identity() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("document.txt");
        let replacement = directory.path().join("replacement.tmp");
        fs::write(&target, b"old").unwrap();
        let mut app = FileApp::new(directory.path().to_path_buf());
        let identity = app.browser.identity_at(0).unwrap();
        app.selected = Some(identity);
        app.selected_entries = HashSet::from([identity]);
        app.selection_anchor = Some(identity);

        fs::write(&replacement, b"new").unwrap();
        fs::remove_file(&target).unwrap();
        fs::rename(&replacement, &target).unwrap();
        app.browser = DirectoryBrowser::open(directory.path()).unwrap();
        app.reconciliation_changed(
            Some((Some(identity), target.clone())),
            vec![(Some(identity), target.clone())],
            Some((Some(identity), target)),
        );

        assert_eq!(app.selected, None);
        assert!(app.selected_entries.is_empty());
        assert_eq!(app.selection_anchor, None);
    }

    #[test]
    fn watcher_failure_keeps_listing_and_exposes_retry_state() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("visible.txt"), b"visible").unwrap();
        let mut app = FileApp::new(directory.path().to_path_buf());
        let watch = crate::watch::DirectoryWatch::fixture(directory.path().to_path_buf());
        watch.inject_failure("queue overflow");
        app.directory_watch = Some(watch);

        assert!(app.poll_directory_watch());
        assert_eq!(app.browser.entries().len(), 1);
        assert!(app.directory_watch.is_none());
        assert!(app.directory_watch_retry_at.is_some());
        assert!(app.status.contains("queue overflow"));
        assert!(app.status.contains("Refresh remains available"));
    }

    #[test]
    fn rename_and_multi_file_copy_use_production_messages() {
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("a.txt"), b"a").unwrap();
        fs::write(source.path().join("b.txt"), b"b").unwrap();
        let mut app = FileApp::new(source.path().to_path_buf());
        let identity = app.browser.identity_at(0).unwrap();
        app.selected_entries = HashSet::from([identity]);
        app.selected = Some(identity);
        app.update_message(FileMessage::BeginRename);
        app.update_message(FileMessage::RenameChanged("renamed.txt".into()));
        app.update_message(FileMessage::CommitRename);
        while app.rename_rx.is_some() {
            app.poll_rename();
            thread::sleep(Duration::from_millis(1));
        }
        assert!(source.path().join("renamed.txt").exists());

        app.browser = DirectoryBrowser::open(source.path()).unwrap();
        app.selected_entries = (0..app.browser.entries().len())
            .filter_map(|index| app.browser.identity_at(index))
            .collect();
        app.update_message(FileMessage::CopySelection);
        let destination = tempfile::tempdir().unwrap();
        app.browser = DirectoryBrowser::open(destination.path()).unwrap();
        app.update_message(FileMessage::Paste);
        while app.transfer_rx.is_some() {
            app.poll_transfer();
            thread::sleep(Duration::from_millis(1));
        }
        assert!(destination.path().join("renamed.txt").exists());
        assert!(destination.path().join("b.txt").exists());
    }

    #[test]
    fn native_drop_burst_is_coalesced_into_one_async_multi_item_transfer() {
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("a.txt"), b"a").unwrap();
        fs::write(source.path().join("b.txt"), b"b").unwrap();
        let destination = tempfile::tempdir().unwrap();
        let mut app = FileApp::new(destination.path().to_path_buf());
        <FileApp as nickel_ui::Application>::file_drag_event(
            &mut app,
            nickel_ui::FileDragEvent::Dropped(source.path().join("a.txt")),
        );
        <FileApp as nickel_ui::Application>::file_drag_event(
            &mut app,
            nickel_ui::FileDragEvent::Dropped(source.path().join("b.txt")),
        );
        app.native_drop_deadline = Some(Instant::now());
        assert!(app.poll_native_drop());
        while app.transfer_rx.is_some() {
            app.poll_transfer();
            thread::sleep(Duration::from_millis(1));
        }
        assert!(destination.path().join("a.txt").exists());
        assert!(destination.path().join("b.txt").exists());
    }

    #[test]
    fn native_drop_honors_the_compositor_negotiated_move_action() {
        let source = tempfile::tempdir().unwrap();
        let source_path = source.path().join("move.txt");
        fs::write(&source_path, b"move").unwrap();
        let destination = tempfile::tempdir().unwrap();
        let mut app = FileApp::new(destination.path().to_path_buf());
        <FileApp as nickel_ui::Application>::file_drag_event(
            &mut app,
            nickel_ui::FileDragEvent::ActionChanged(nickel_ui::FileDragAction::Move),
        );
        <FileApp as nickel_ui::Application>::file_drag_event(
            &mut app,
            nickel_ui::FileDragEvent::Dropped(source_path.clone()),
        );
        app.native_drop_deadline = Some(Instant::now());
        assert!(app.poll_native_drop());
        while app.transfer_rx.is_some() {
            app.poll_transfer();
            thread::sleep(Duration::from_millis(1));
        }
        assert!(!source_path.exists());
        assert_eq!(
            fs::read(destination.path().join("move.txt")).unwrap(),
            b"move"
        );
    }

    #[test]
    fn conflicting_transfer_waits_for_an_explicit_bounded_policy() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let source_path = source.path().join("report.txt");
        fs::write(&source_path, b"new").unwrap();
        fs::write(destination.path().join("report.txt"), b"old").unwrap();
        let offer = ClipboardOffer::new(
            TransferIntent::Copy,
            vec![TransferSource {
                provider: "local".into(),
                identity: crate::file_identity(&source_path).unwrap(),
                path: source_path,
                capabilities: ItemCapabilities {
                    readable: true,
                    removable: true,
                },
            }],
        )
        .unwrap();
        let mut app = FileApp::new(destination.path().to_path_buf());

        app.start_transfer(offer, destination.path().to_path_buf());
        assert!(app.transfer_rx.is_none());
        assert_eq!(
            app.pending_transfer_conflict
                .as_ref()
                .unwrap()
                .conflicts
                .len(),
            1
        );
        app.update_message(FileMessage::TransferKeepBoth);
        while app.transfer_rx.is_some() {
            app.poll_transfer();
            thread::sleep(Duration::from_millis(1));
        }

        assert_eq!(
            fs::read(destination.path().join("report.txt")).unwrap(),
            b"old"
        );
        assert_eq!(
            fs::read(destination.path().join("report (2).txt")).unwrap(),
            b"new"
        );
    }
}

#[cfg(test)]
#[path = "ui_layout_tests.rs"]
mod ui_layout_tests;
