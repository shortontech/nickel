use std::sync::Arc;

use nickel_core::theme::ThemePalette;
use nickel_ui::{AnyView, Component, ComponentBuilderExt, Insets, SemanticRole, ui};

use crate::{
    DirectoryBrowser, FileEntry,
    app::{FileApp, FileMessage},
};

pub(crate) fn status_text(app: &FileApp) -> String {
    if !app.status.is_empty() {
        return app.status.clone();
    }
    let total = app.browser.entries().len();
    let total_label = format!("{total} item{}", if total == 1 { "" } else { "s" });
    let selected = app.selected_entries.len();
    if selected == 0 {
        total_label
    } else {
        format!("{selected} selected · {total_label}")
    }
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
                    focus_border={palette.accent} controller_focus_border={palette.complement} padding={Insets {
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
    rows: Vec<AnyView<FileMessage>>,
    palette: ThemePalette,
) -> AnyView<FileMessage> {
    AnyView::new(ui! {
        <Sidebar width={width} background={palette.panel} padding={Insets {
            top: 14.0, right: 10.0, bottom: 12.0, left: 10.0,
        }} gap={3.0}>
            <Text height={34.0} scale={1.55} color={palette.text}>{"Nickel File"}</Text>
            <HorizontalRule color={palette.muted} spacing_pair={(5.0, 8.0)} />
            <SidebarSection title={"Places"} color={palette.muted}>{rows.into_iter()}</SidebarSection>
        </Sidebar>
    })
}

pub(crate) fn status_bar(text: String, palette: ThemePalette) -> AnyView<FileMessage> {
    AnyView::new(ui! {
        <Container id={"file-footer"} height={30.0} shrink={0.0} background={palette.surface} padding={Insets {
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
            if light_mode { 0xffffff } else { palette.background }
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

pub(crate) fn grid_item(
    index: usize,
    entry: &FileEntry,
    selected: bool,
    icon: Option<(u16, Arc<image::RgbaImage>)>,
    palette: ThemePalette,
    icon_size: f32,
    light_mode: bool,
) -> impl Component<FileMessage> + use<> {
    let (icon_id, icon_image) = icon.unwrap_or_else(empty_artwork);
    ui! {
        <FileGridItem on_press={FileMessage::Entry(index)} label={entry.display_name()}
            asset_id={icon_id} image={icon_image} generation={u64::from(icon_id)}
            borderless_palette={(if selected { palette.accent_soft } else if light_mode { 0xffffff } else { palette.background }, palette.text)}
            icon_size={icon_size} focus_border={palette.accent} controller_focus_border={palette.complement}
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
    light_mode: bool,
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
    let size = entry.size.map(format_file_size).unwrap_or_default();
    let modified = entry.modified.map(format_modified).unwrap_or_default();
    ui! {
        <Container id={format!("file-entry-{index}")} height={58.0}
            background={if selected { palette.accent_soft } else if light_mode { 0xffffff } else { palette.background }}
            hover_background={palette.surface_hover} pressed_background={palette.accent_soft}
            padding={Insets { top: 7.0, right: 10.0, bottom: 7.0, left: 10.0 }}
            on_press={FileMessage::Entry(index)} context_message={FileMessage::ContextEntry(index)}
            semantic_role={SemanticRole::Button} accessibility_label={entry.display_name()}
            focus_border={palette.accent} controller_focus_border={palette.complement}>
            <Row gap={12.0}>
                <Image asset_id={icon_id} image={icon_image} generation={u64::from(icon_id)} width={28.0} height={28.0} />
                <Container id={format!("details-name-{index}")} grow={1.0} min_width={120.0} height={44.0}>
                    <Text color={palette.text} wrap={true} max_lines={2} ellipsis={true} line_height={18.0}>{entry.display_name()}</Text>
                </Container>
                <Text width={100.0} color={palette.muted}>{kind}</Text>
                <Text width={140.0} color={palette.muted}>{modified}</Text>
                <Text width={80.0} color={palette.muted}>{size}</Text>
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

fn format_modified(time: std::time::SystemTime) -> String {
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
