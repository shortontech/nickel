use super::*;

#[cfg(any(test, feature = "workbench-fixtures"))]
pub struct FileFixtureProvider;

#[cfg(any(test, feature = "workbench-fixtures"))]
pub struct FileWorkbenchFixture;

#[cfg(any(test, feature = "workbench-fixtures"))]
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

#[cfg(any(test, feature = "workbench-fixtures"))]
pub(crate) const FILE_FIXTURE_VARIANTS: &[nickel_ui_testkit::FixtureVariant] = &[
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

#[cfg(any(test, feature = "workbench-fixtures"))]
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

#[cfg(any(test, feature = "workbench-fixtures"))]
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

#[cfg(any(test, feature = "workbench-fixtures"))]
pub(crate) const FILE_FIXTURE_ASSETS: &[nickel_ui_testkit::FixtureAsset] = &[
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

#[cfg(any(test, feature = "workbench-fixtures"))]
impl nickel_ui_testkit::FixtureProvider for FileFixtureProvider {
    fn register(
        &self,
        registry: &mut nickel_ui_testkit::FixtureRegistry,
    ) -> Result<(), nickel_ui_testkit::RegistryError> {
        registry.register::<FileWorkbenchFixture>()
    }
}
