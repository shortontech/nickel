use std::{
    fs,
    path::{Path, PathBuf},
};

use nickel_markdown::MarkdownDocument;
#[cfg(feature = "application")]
use nickel_markdown::{MarkdownPalette, markdown_view};
#[cfg(feature = "application")]
use nickel_ui::{
    Align, AnyView, Application, Button, ComponentBuilderExt, Container, Insets, Justify, Length,
    Row, Shortcut, Spacer, Text, UiId, VerticalScroll, View, ui,
};
#[cfg(feature = "application")]
use std::sync::mpsc::{self, Receiver, Sender};
use url::Url;

pub const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_HISTORY: usize = 50;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedDocument {
    pub path: PathBuf,
    pub title: String,
    pub source: String,
    pub document: MarkdownDocument,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewerStatus {
    Empty,
    Loading { path: PathBuf },
    Ready,
    ChangedOnDisk,
    Error(ViewerError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewerError {
    pub kind: ViewerErrorKind,
    pub path: PathBuf,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewerErrorKind {
    Missing,
    Directory,
    UnsupportedExtension,
    Oversized,
    InvalidUtf8,
    PermissionDenied,
    Io,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadRequest {
    pub generation: u64,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadCompletion {
    pub generation: u64,
    pub result: Result<LoadedDocument, ViewerError>,
}

#[derive(Clone, Debug, PartialEq)]
struct HistoryEntry {
    path: PathBuf,
    scroll: f32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PendingNavigation {
    Open { fragment: Option<String> },
    Reload,
    History(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewerModel {
    current: Option<LoadedDocument>,
    status: ViewerStatus,
    generation: u64,
    pending: Option<(u64, PendingNavigation)>,
    history: Vec<HistoryEntry>,
    history_index: Option<usize>,
}

impl Default for ViewerModel {
    fn default() -> Self {
        Self {
            current: None,
            status: ViewerStatus::Empty,
            generation: 0,
            pending: None,
            history: Vec::new(),
            history_index: None,
        }
    }
}

impl ViewerModel {
    #[must_use]
    pub fn current(&self) -> Option<&LoadedDocument> {
        self.current.as_ref()
    }

    #[must_use]
    pub fn status(&self) -> &ViewerStatus {
        &self.status
    }

    #[must_use]
    pub fn can_go_back(&self) -> bool {
        self.history_index.is_some_and(|index| index > 0)
    }

    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        self.history_index
            .is_some_and(|index| index + 1 < self.history.len())
    }

    #[must_use]
    pub fn is_loading(&self) -> bool {
        matches!(self.status, ViewerStatus::Loading { .. })
    }

    #[must_use]
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn begin_open(&mut self, path: impl Into<PathBuf>) -> LoadRequest {
        self.begin(path.into(), PendingNavigation::Open { fragment: None })
    }

    pub fn begin_open_at(
        &mut self,
        path: impl Into<PathBuf>,
        fragment: Option<String>,
    ) -> LoadRequest {
        self.begin(path.into(), PendingNavigation::Open { fragment })
    }

    pub fn begin_reload(&mut self) -> Option<LoadRequest> {
        let path = self.current.as_ref()?.path.clone();
        Some(self.begin(path, PendingNavigation::Reload))
    }

    pub fn begin_back(&mut self) -> Option<LoadRequest> {
        let index = self.history_index?.checked_sub(1)?;
        let path = self.history.get(index)?.path.clone();
        Some(self.begin(path, PendingNavigation::History(index)))
    }

    pub fn begin_forward(&mut self) -> Option<LoadRequest> {
        let index = self.history_index?.checked_add(1)?;
        let path = self.history.get(index)?.path.clone();
        Some(self.begin(path, PendingNavigation::History(index)))
    }

    pub fn set_scroll_position(&mut self, scroll: f32) {
        let Some(index) = self.history_index else {
            return;
        };
        if let Some(entry) = self.history.get_mut(index) {
            entry.scroll = scroll.max(0.0);
        }
    }

    #[must_use]
    pub fn scroll_position(&self) -> f32 {
        self.history_index
            .and_then(|index| self.history.get(index))
            .map_or(0.0, |entry| entry.scroll)
    }

    pub fn mark_changed_on_disk(&mut self) {
        if self.current.is_some() && !matches!(self.status, ViewerStatus::Loading { .. }) {
            self.status = ViewerStatus::ChangedOnDisk;
        }
    }

    pub fn dismiss_status(&mut self) {
        self.status = if self.current.is_some() {
            ViewerStatus::Ready
        } else {
            ViewerStatus::Empty
        };
    }

    pub fn complete(&mut self, completion: LoadCompletion) -> bool {
        let Some((generation, _)) = self.pending.as_ref() else {
            return false;
        };
        if completion.generation != *generation {
            return false;
        }
        let (_, navigation) = self.pending.take().expect("matched pending load");
        match completion.result {
            Ok(document) => {
                match navigation {
                    PendingNavigation::Open { ref fragment } => {
                        self.push_history(document.path.clone());
                        if let Some(fragment) = fragment
                            && let Some(offset) =
                                heading_scroll_position(&document.document, fragment)
                        {
                            self.set_scroll_position(offset);
                        }
                    }
                    PendingNavigation::Reload => {}
                    PendingNavigation::History(index) => self.history_index = Some(index),
                }
                self.current = Some(document);
                self.status = ViewerStatus::Ready;
            }
            Err(error) => self.status = ViewerStatus::Error(error),
        }
        true
    }

    fn begin(&mut self, path: PathBuf, navigation: PendingNavigation) -> LoadRequest {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.pending = Some((self.generation, navigation));
        self.status = ViewerStatus::Loading { path: path.clone() };
        LoadRequest {
            generation: self.generation,
            path,
        }
    }

    fn push_history(&mut self, path: PathBuf) {
        if let Some(index) = self.history_index {
            self.history.truncate(index + 1);
        }
        if self.history.last().is_some_and(|entry| entry.path == path) {
            self.history_index = Some(self.history.len() - 1);
            return;
        }
        self.history.push(HistoryEntry { path, scroll: 0.0 });
        if self.history.len() > MAX_HISTORY {
            let overflow = self.history.len() - MAX_HISTORY;
            self.history.drain(..overflow);
        }
        self.history_index = Some(self.history.len() - 1);
    }
}

/// Return the stable reading offset used to bring a heading into view.
#[must_use]
pub fn heading_scroll_position(document: &MarkdownDocument, fragment: &str) -> Option<f32> {
    let fragment = fragment.trim_start_matches('#');
    document
        .blocks
        .iter()
        .position(|block| {
            matches!(block, nickel_markdown::Block::Heading { anchor, .. } if anchor == fragment)
        })
        .map(|index| index as f32 * 72.0)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Destination {
    Fragment(String),
    Local {
        path: PathBuf,
        fragment: Option<String>,
    },
    External(Url),
    UnsupportedScheme(String),
    Invalid(String),
}

#[must_use]
pub fn classify_destination(current: &Path, destination: &str) -> Destination {
    if let Some(fragment) = destination.strip_prefix('#') {
        return Destination::Fragment(fragment.to_owned());
    }
    if let Ok(url) = Url::parse(destination) {
        return match url.scheme() {
            "http" | "https" => Destination::External(url),
            "file" => url.to_file_path().map_or_else(
                |_| Destination::Invalid(destination.to_owned()),
                |path| Destination::Local {
                    path,
                    fragment: url.fragment().map(str::to_owned),
                },
            ),
            scheme => Destination::UnsupportedScheme(scheme.to_owned()),
        };
    }
    let (path, fragment) = destination
        .split_once('#')
        .map_or((destination, None), |(path, fragment)| {
            (path, Some(fragment.to_owned()))
        });
    if path.is_empty() {
        return fragment.map_or_else(
            || Destination::Invalid(destination.to_owned()),
            Destination::Fragment,
        );
    }
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        current
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    };
    Destination::Local { path, fragment }
}

pub fn load_document(request: &LoadRequest) -> LoadCompletion {
    LoadCompletion {
        generation: request.generation,
        result: read_document(&request.path),
    }
}

/// Compare the current file bytes with the last successfully loaded UTF-8 source.
pub fn document_changed_on_disk(document: &LoadedDocument) -> Result<bool, ViewerError> {
    let metadata =
        fs::metadata(&document.path).map_err(|error| file_error(&document.path, error))?;
    if metadata.len() > MAX_DOCUMENT_BYTES {
        return Ok(true);
    }
    let bytes = fs::read(&document.path).map_err(|error| file_error(&document.path, error))?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    Ok(bytes != document.source.as_bytes())
}

fn read_document(path: &Path) -> Result<LoadedDocument, ViewerError> {
    let canonical = fs::canonicalize(path).map_err(|error| file_error(path, error))?;
    let metadata = fs::metadata(&canonical).map_err(|error| file_error(&canonical, error))?;
    if metadata.is_dir() {
        return Err(error(
            ViewerErrorKind::Directory,
            &canonical,
            "path is a directory",
        ));
    }
    if !is_markdown_path(&canonical) {
        return Err(error(
            ViewerErrorKind::UnsupportedExtension,
            &canonical,
            "expected a .md or .markdown file",
        ));
    }
    if metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(error(
            ViewerErrorKind::Oversized,
            &canonical,
            format!(
                "document is {} bytes; maximum is {MAX_DOCUMENT_BYTES} bytes",
                metadata.len()
            ),
        ));
    }
    let bytes = fs::read(&canonical).map_err(|error| file_error(&canonical, error))?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let source = String::from_utf8(bytes.to_vec()).map_err(|error| ViewerError {
        kind: ViewerErrorKind::InvalidUtf8,
        path: canonical.clone(),
        detail: format!("invalid UTF-8 at byte {}", error.utf8_error().valid_up_to()),
    })?;
    let title = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Markdown")
        .to_owned();
    Ok(LoadedDocument {
        path: canonical,
        title,
        document: MarkdownDocument::parse(source.clone()),
        source,
    })
}

fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(extension.to_ascii_lowercase().as_str(), "md" | "markdown")
        })
}

fn file_error(path: &Path, error_value: std::io::Error) -> ViewerError {
    let kind = match error_value.kind() {
        std::io::ErrorKind::NotFound => ViewerErrorKind::Missing,
        std::io::ErrorKind::PermissionDenied => ViewerErrorKind::PermissionDenied,
        _ => ViewerErrorKind::Io,
    };
    error(kind, path, error_value.to_string())
}

fn error(kind: ViewerErrorKind, path: &Path, detail: impl Into<String>) -> ViewerError {
    ViewerError {
        kind,
        path: path.to_owned(),
        detail: detail.into(),
    }
}

#[cfg(feature = "application")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewerPalette {
    background: u32,
    panel: u32,
    text: u32,
    muted: u32,
    accent: u32,
    border: u32,
    error: u32,
}

#[cfg(feature = "application")]
impl ViewerPalette {
    #[must_use]
    pub fn from_appearance(appearance: nickel_core::theme::Appearance) -> Self {
        let palette = nickel_core::theme::ThemePalette::from_appearance(appearance);
        Self {
            background: palette.background,
            panel: palette.surface,
            text: palette.text,
            muted: palette.muted,
            accent: palette.accent,
            border: palette.surface_hover,
            error: match appearance.mode {
                nickel_core::theme::ThemeMode::Dark => 0x7f3038,
                nickel_core::theme::ThemeMode::Light => 0xf2c7cb,
            },
        }
    }
}

#[cfg(feature = "application")]
impl Default for ViewerPalette {
    fn default() -> Self {
        Self::from_appearance(nickel_core::theme::Appearance::default())
    }
}

#[cfg(feature = "application")]
#[derive(Clone, Debug, PartialEq)]
pub enum ViewerMessage {
    Back,
    Forward,
    Reload,
    DismissStatus,
    Link(String),
    Scroll(f32),
}

#[cfg(feature = "application")]
pub struct ViewerApplication {
    model: ViewerModel,
    sender: Sender<LoadCompletion>,
    receiver: Receiver<LoadCompletion>,
    title: String,
    runtime_error: Option<String>,
    palette: ViewerPalette,
}

#[cfg(feature = "application")]
impl ViewerApplication {
    #[must_use]
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let mut application = Self {
            model: ViewerModel::default(),
            sender,
            receiver,
            title: "Nickel Markdown".into(),
            runtime_error: None,
            palette: ViewerPalette::from_appearance(nickel_platform::appearance()),
        };
        let request = application.model.begin_open(path);
        application.queue(request);
        application
    }

    #[cfg(test)]
    fn loaded(document: LoadedDocument) -> Self {
        let (sender, receiver) = mpsc::channel();
        let mut model = ViewerModel::default();
        let request = model.begin_open(document.path.clone());
        model.complete(LoadCompletion {
            generation: request.generation,
            result: Ok(document),
        });
        Self {
            title: model.current().map_or_else(
                || "Nickel Markdown".into(),
                |document| document.title.clone(),
            ),
            model,
            sender,
            receiver,
            runtime_error: None,
            palette: ViewerPalette::default(),
        }
    }

    #[must_use]
    pub fn model(&self) -> &ViewerModel {
        &self.model
    }

    fn queue(&self, request: LoadRequest) {
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send(load_document(&request));
        });
    }

    fn open_link(&mut self, destination: &str) {
        let Some(current) = self.model.current() else {
            return;
        };
        match classify_destination(&current.path, destination) {
            Destination::Fragment(fragment) => {
                if let Some(offset) = heading_scroll_position(&current.document, &fragment) {
                    self.model.set_scroll_position(offset);
                    self.runtime_error = None;
                } else {
                    self.runtime_error = Some(format!("Section not found: #{fragment}"));
                }
            }
            Destination::Local { path, fragment } => {
                let request = self.model.begin_open_at(path, fragment);
                self.queue(request);
            }
            Destination::External(url) => {
                if let Err(error) = nickel_platform::open_external_url(url.as_str()) {
                    self.runtime_error = Some(error);
                }
            }
            Destination::UnsupportedScheme(scheme) => {
                self.runtime_error = Some(format!("Unsupported link scheme: {scheme}"));
            }
            Destination::Invalid(value) => {
                self.runtime_error = Some(format!("Invalid link: {value}"));
            }
        }
    }
}

#[cfg(feature = "application")]
impl Application for ViewerApplication {
    type Message = ViewerMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            ViewerMessage::Back => {
                if let Some(request) = self.model.begin_back() {
                    self.queue(request);
                }
            }
            ViewerMessage::Forward => {
                if let Some(request) = self.model.begin_forward() {
                    self.queue(request);
                }
            }
            ViewerMessage::Reload => {
                if let Some(request) = self.model.begin_reload() {
                    self.queue(request);
                }
            }
            ViewerMessage::DismissStatus => {
                self.runtime_error = None;
                self.model.dismiss_status();
            }
            ViewerMessage::Link(destination) => self.open_link(&destination),
            ViewerMessage::Scroll(offset) => self.model.set_scroll_position(offset),
        }
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(completion) = self.receiver.try_recv() {
            changed |= self.model.complete(completion);
        }
        if let Some(document) = self.model.current() {
            self.title.clone_from(&document.title);
        }
        changed
    }

    fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        match shortcut {
            Shortcut::Escape
                if self.runtime_error.is_some()
                    || matches!(self.model.status(), ViewerStatus::Error(_)) =>
            {
                self.update(ViewerMessage::DismissStatus);
                true
            }
            Shortcut::Reload if self.model.current().is_some() && !self.model.is_loading() => {
                self.update(ViewerMessage::Reload);
                true
            }
            Shortcut::Back if self.model.can_go_back() && !self.model.is_loading() => {
                self.update(ViewerMessage::Back);
                true
            }
            Shortcut::Forward if self.model.can_go_forward() && !self.model.is_loading() => {
                self.update(ViewerMessage::Forward);
                true
            }
            Shortcut::DocumentStart if self.model.current().is_some() => {
                self.model.set_scroll_position(0.0);
                true
            }
            Shortcut::DocumentEnd if self.model.current().is_some() => {
                self.model.set_scroll_position(f32::MAX);
                true
            }
            _ => false,
        }
    }

    fn view(&self) -> impl View<Self::Message> {
        viewer_view_with_palette(&self.model, self.runtime_error.as_deref(), self.palette)
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn initial_size(&self) -> (u32, u32) {
        (960, 720)
    }
}

#[cfg(feature = "application")]
pub fn viewer_view(model: &ViewerModel, runtime_error: Option<&str>) -> AnyView<ViewerMessage> {
    viewer_view_with_palette(model, runtime_error, ViewerPalette::default())
}

#[cfg(feature = "application")]
pub fn viewer_view_with_palette(
    model: &ViewerModel,
    runtime_error: Option<&str>,
    palette: ViewerPalette,
) -> AnyView<ViewerMessage> {
    let actions_enabled = !model.is_loading();
    let back = toolbar_button(
        "←",
        ViewerMessage::Back,
        actions_enabled && model.can_go_back(),
        palette,
    );
    let forward = toolbar_button(
        "→",
        ViewerMessage::Forward,
        actions_enabled && model.can_go_forward(),
        palette,
    );
    let reload = toolbar_button(
        "Reload",
        ViewerMessage::Reload,
        actions_enabled && model.current().is_some(),
        palette,
    );
    let title = model
        .current()
        .map_or("Nickel Markdown", |document| &document.title);
    let status = status_view(model.status(), runtime_error, palette);
    let content = model.current().map_or_else(
        || {
            AnyView::new(
                Text::new(if matches!(model.status(), ViewerStatus::Loading { .. }) {
                    "Loading Markdown…"
                } else {
                    "No Markdown document is open."
                })
                .color(palette.muted)
                .width_length(Length::Fill)
                .wrap(true),
            )
        },
        |loaded| {
            markdown_view(&loaded.document, markdown_palette(palette), |destination| {
                ViewerMessage::Link(destination.to_owned())
            })
        },
    );
    let scroll_id = model.current().map_or_else(
        || UiId::new("markdown-empty-scroll"),
        |document| UiId::new(format!("markdown-scroll/{}", document.path.display())),
    );
    let toolbar = Row::new()
        .fill_width()
        .padding(Insets::all(12.0))
        .gap(8.0)
        .align_items(Align::Center)
        .background(palette.panel)
        .child(back)
        .child(forward)
        .child(reload)
        .child(Text::new(title).color(palette.text).bold(true))
        .child(Spacer::new().grow(1.0));
    let document = VerticalScroll::new(
        ViewerMessage::Scroll(model.scroll_position()),
        model.scroll_position(),
    )
    .on_scroll(ViewerMessage::Scroll)
    .controlled(true)
    .id(scroll_id)
    .grow(1.0)
    .child(
        Row::new()
            .fill_width()
            .justify_content(Justify::Center)
            .child(
                Container::new()
                    .fill_width()
                    .max_width(900.0)
                    .padding(Insets::all(28.0))
                    .child(content),
            ),
    );
    AnyView::new(ui! {
        <Column fill_width grow={1.0} background={palette.background}>
            {toolbar}
            {status}
            {document}
        </Column>
    })
}

#[cfg(feature = "application")]
fn toolbar_button(
    label: &str,
    message: ViewerMessage,
    enabled: bool,
    palette: ViewerPalette,
) -> AnyView<ViewerMessage> {
    if enabled {
        AnyView::new(
            Button::new(message, label)
                .background(palette.panel)
                .border(palette.border, 1.0)
                .color(palette.text),
        )
    } else {
        AnyView::new(
            Container::new()
                .height(42.0)
                .padding(Insets::all(11.0))
                .child(Text::new(label).color(palette.muted)),
        )
    }
}

#[cfg(feature = "application")]
fn status_view(
    status: &ViewerStatus,
    runtime_error: Option<&str>,
    palette: ViewerPalette,
) -> AnyView<ViewerMessage> {
    let message = runtime_error.map(str::to_owned).or_else(|| match status {
        ViewerStatus::Error(error) => Some(format!("{}: {}", error.path.display(), error.detail)),
        ViewerStatus::ChangedOnDisk => {
            Some("This document changed on disk. Reload to update it.".into())
        }
        _ => None,
    });
    message.map_or_else(
        || AnyView::new(Container::new().height(0.0)),
        |message| {
            AnyView::new(
                Row::new()
                    .fill_width()
                    .padding(Insets::all(10.0))
                    .gap(8.0)
                    .background(palette.error)
                    .child(Text::new(message).color(palette.text).wrap(true).grow(1.0))
                    .child(Button::new(ViewerMessage::DismissStatus, "Dismiss")),
            )
        },
    )
}

#[cfg(feature = "application")]
fn markdown_palette(palette: ViewerPalette) -> MarkdownPalette {
    MarkdownPalette {
        foreground: palette.text,
        muted: palette.muted,
        accent: palette.accent,
        surface: palette.panel,
        border: palette.border,
        code: palette.text,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::tempdir;

    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        let mut file = fs::File::create(path).unwrap();
        file.write_all(bytes).unwrap();
    }

    #[test]
    fn strict_file_loading_accepts_bom_and_classifies_failures() {
        let directory = tempdir().unwrap();
        let valid = directory.path().join("guide.md");
        write(&valid, b"\xef\xbb\xbf# Guide");
        let request = LoadRequest {
            generation: 7,
            path: valid.clone(),
        };
        let loaded = load_document(&request).result.unwrap();
        assert_eq!(loaded.source, "# Guide");
        assert!(loaded.path.is_absolute());

        let invalid = directory.path().join("invalid.md");
        write(&invalid, &[0xff, 0xfe]);
        assert_eq!(
            read_document(&invalid).unwrap_err().kind,
            ViewerErrorKind::InvalidUtf8
        );
        assert_eq!(
            read_document(directory.path()).unwrap_err().kind,
            ViewerErrorKind::Directory
        );
        assert_eq!(
            read_document(&directory.path().join("missing.md"))
                .unwrap_err()
                .kind,
            ViewerErrorKind::Missing
        );
        let text = directory.path().join("plain.txt");
        write(&text, b"plain");
        assert_eq!(
            read_document(&text).unwrap_err().kind,
            ViewerErrorKind::UnsupportedExtension
        );

        let oversized = directory.path().join("oversized.md");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_DOCUMENT_BYTES + 1)
            .unwrap();
        let oversized_error = read_document(&oversized).unwrap_err();
        assert_eq!(oversized_error.kind, ViewerErrorKind::Oversized);
        assert!(oversized_error.detail.contains("16777217 bytes"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_load_uses_the_canonical_document_identity() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().unwrap();
        let target = directory.path().join("target.md");
        let link = directory.path().join("link.md");
        write(&target, b"target");
        symlink(&target, &link).unwrap();
        let loaded = read_document(&link).unwrap();
        assert_eq!(loaded.path, fs::canonicalize(&target).unwrap());
    }

    #[test]
    fn stale_completion_cannot_replace_current_document() {
        let directory = tempdir().unwrap();
        let first = directory.path().join("first.md");
        let second = directory.path().join("second.md");
        write(&first, b"first");
        write(&second, b"second");
        let mut model = ViewerModel::default();
        let old = model.begin_open(&first);
        let current = model.begin_open(&second);
        assert!(!model.complete(load_document(&old)));
        assert!(model.complete(load_document(&current)));
        assert_eq!(model.current().unwrap().source, "second");
    }

    #[test]
    fn failed_reload_preserves_last_successful_document() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("guide.md");
        write(&path, b"kept");
        let mut model = ViewerModel::default();
        let open = model.begin_open(&path);
        model.complete(load_document(&open));
        fs::remove_file(&path).unwrap();
        let reload = model.begin_reload().unwrap();
        model.complete(load_document(&reload));
        assert_eq!(model.current().unwrap().source, "kept");
        assert!(matches!(
            model.status(),
            ViewerStatus::Error(ViewerError {
                kind: ViewerErrorKind::Missing,
                ..
            })
        ));
    }

    #[test]
    fn changed_on_disk_compares_exact_loaded_bytes() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("guide.md");
        write(&path, b"\xef\xbb\xbforiginal");
        let loaded = read_document(&path).unwrap();
        assert!(!document_changed_on_disk(&loaded).unwrap());
        write(&path, b"changed");
        assert!(document_changed_on_disk(&loaded).unwrap());
    }

    #[test]
    fn permission_errors_have_a_distinct_classification() {
        let path = Path::new("/private/guide.md");
        let error = file_error(
            path,
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied by fixture"),
        );
        assert_eq!(error.kind, ViewerErrorKind::PermissionDenied);
        assert_eq!(error.path, path);
    }

    #[test]
    fn destinations_are_typed_without_opening_them() {
        let current = Path::new("/docs/readme.md");
        assert_eq!(
            classify_destination(current, "#start"),
            Destination::Fragment("start".into())
        );
        assert_eq!(
            classify_destination(current, "guide.md#install"),
            Destination::Local {
                path: PathBuf::from("/docs/guide.md"),
                fragment: Some("install".into())
            }
        );
        assert!(matches!(
            classify_destination(current, "https://example.com/a"),
            Destination::External(url) if url.as_str() == "https://example.com/a"
        ));
        assert_eq!(
            classify_destination(current, "javascript:alert(1)"),
            Destination::UnsupportedScheme("javascript".into())
        );
    }

    #[test]
    #[cfg(feature = "application")]
    fn fragment_navigation_targets_headings_without_adding_history() {
        let document = MarkdownDocument::parse("# Start\n\nBody\n\n## Details\n\nMore");
        assert_eq!(heading_scroll_position(&document, "start"), Some(0.0));
        assert_eq!(heading_scroll_position(&document, "#details"), Some(144.0));
        assert_eq!(heading_scroll_position(&document, "missing"), None);

        let loaded = LoadedDocument {
            path: PathBuf::from("/tmp/guide.md"),
            title: "guide.md".into(),
            source: document.source.clone(),
            document,
        };
        let mut application = ViewerApplication::loaded(loaded);
        application.update(ViewerMessage::Link("#details".into()));
        assert_eq!(application.model.scroll_position(), 144.0);
        assert_eq!(application.model.history_len(), 1);
    }

    #[test]
    fn local_fragment_is_applied_after_the_document_load_completes() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("guide.md");
        write(&path, b"# Start\n\nBody\n\n## Details");
        let mut model = ViewerModel::default();
        let request = model.begin_open_at(&path, Some("details".into()));
        assert!(model.complete(load_document(&request)));
        assert_eq!(model.scroll_position(), 144.0);
        assert_eq!(model.history_len(), 1);
    }

    #[test]
    fn navigation_is_bounded_and_new_open_discards_forward_branch() {
        let directory = tempdir().unwrap();
        let mut model = ViewerModel::default();
        for index in 0..55 {
            let path = directory.path().join(format!("{index}.md"));
            write(&path, index.to_string().as_bytes());
            let request = model.begin_open(path);
            assert!(model.complete(load_document(&request)));
        }
        assert_eq!(model.history_len(), 50);
        let back = model.begin_back().unwrap();
        assert!(model.complete(load_document(&back)));
        assert!(model.can_go_forward());
        let replacement = directory.path().join("replacement.md");
        write(&replacement, b"replacement");
        let request = model.begin_open(replacement);
        assert!(model.complete(load_document(&request)));
        assert!(!model.can_go_forward());
    }

    #[test]
    fn back_forward_restore_each_entry_reading_position() {
        let directory = tempdir().unwrap();
        let first = directory.path().join("first.md");
        let second = directory.path().join("second.md");
        write(&first, b"first");
        write(&second, b"second");
        let mut model = ViewerModel::default();
        let request = model.begin_open(&first);
        model.complete(load_document(&request));
        model.set_scroll_position(125.0);
        let request = model.begin_open(&second);
        model.complete(load_document(&request));
        model.set_scroll_position(275.0);

        let request = model.begin_back().unwrap();
        model.complete(load_document(&request));
        assert_eq!(model.current().unwrap().source, "first");
        assert_eq!(model.scroll_position(), 125.0);
        let request = model.begin_forward().unwrap();
        model.complete(load_document(&request));
        assert_eq!(model.current().unwrap().source, "second");
        assert_eq!(model.scroll_position(), 275.0);
    }

    #[test]
    fn loading_and_navigation_create_no_sidecar_files() {
        let directory = tempdir().unwrap();
        let first = directory.path().join("first.md");
        let second = directory.path().join("second.markdown");
        write(&first, b"[next](second.markdown)");
        write(&second, b"second");
        let names = || {
            let mut names = fs::read_dir(directory.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>();
            names.sort();
            names
        };
        let before = names();
        let mut model = ViewerModel::default();
        let request = model.begin_open(&first);
        model.complete(load_document(&request));
        let request = model.begin_open(&second);
        model.complete(load_document(&request));
        let request = model.begin_back().unwrap();
        model.complete(load_document(&request));
        let request = model.begin_reload().unwrap();
        model.complete(load_document(&request));
        assert_eq!(names(), before);
    }

    #[test]
    #[cfg(feature = "application")]
    fn viewer_states_have_finite_geometry_and_expected_toolbar_authority() {
        use nickel_ui::{Rect, UiTree};

        let document = LoadedDocument {
            path: PathBuf::from("/tmp/guide.md"),
            title: "guide.md".into(),
            source: "# Guide\n\nRead [more](https://example.com).".into(),
            document: MarkdownDocument::parse("# Guide\n\nRead [more](https://example.com)."),
        };
        let application = ViewerApplication::loaded(document);
        for bounds in [
            Rect::new(0.0, 0.0, 480.0, 360.0),
            Rect::new(0.0, 0.0, 960.0, 720.0),
            Rect::new(0.0, 0.0, 1920.0, 1440.0),
        ] {
            let tree = UiTree::layout(viewer_view(application.model(), None), bounds);
            assert!(tree.message_rect(&ViewerMessage::Reload).is_some());
            assert!(tree.message_rect(&ViewerMessage::Back).is_none());
            assert!(tree.resolved_layout().nodes().iter().all(|node| {
                let rect = node.allocated;
                rect.origin.x.is_finite()
                    && rect.origin.y.is_finite()
                    && rect.size.width.is_finite()
                    && rect.size.height.is_finite()
                    && rect.size.width >= 0.0
                    && rect.size.height >= 0.0
            }));
            let document_width = tree
                .resolved_layout()
                .nodes()
                .iter()
                .find(|node| node.id.as_str().ends_with("markdown-document"))
                .expect("shared Markdown document")
                .allocated
                .size
                .width;
            assert!(document_width <= 844.0, "prose width was {document_width}");
        }

        let mut loading = application.model().clone();
        loading.begin_reload().unwrap();
        let tree = UiTree::layout(
            viewer_view(&loading, None),
            Rect::new(0.0, 0.0, 960.0, 720.0),
        );
        assert!(tree.message_rect(&ViewerMessage::Reload).is_none());
    }

    #[test]
    #[cfg(feature = "application")]
    fn toolbar_activation_survives_reconstruction_between_press_and_release() {
        use nickel_ui::{Point, Rect, UiEvent, UiStateStore, UiTree};

        let document = LoadedDocument {
            path: PathBuf::from("/tmp/guide.md"),
            title: "guide.md".into(),
            source: "# Guide".into(),
            document: MarkdownDocument::parse("# Guide"),
        };
        let application = ViewerApplication::loaded(document);
        let bounds = Rect::new(0.0, 0.0, 960.0, 720.0);
        let mut state = UiStateStore::default();
        let tree =
            UiTree::layout_with_state(viewer_view(application.model(), None), bounds, &mut state);
        let rect = tree
            .message_rect(&ViewerMessage::Reload)
            .expect("reload action");
        let point = Point {
            x: rect.origin.x + rect.size.width * 0.5,
            y: rect.origin.y + rect.size.height * 0.5,
        };
        tree.handle_event(&mut state, UiEvent::PointerPressed(point));
        let rebuilt =
            UiTree::layout_with_state(viewer_view(application.model(), None), bounds, &mut state);
        assert_eq!(
            rebuilt
                .handle_event(&mut state, UiEvent::PointerReleased(point))
                .messages,
            vec![ViewerMessage::Reload]
        );
    }

    #[test]
    #[cfg(feature = "application")]
    fn link_reload_back_and_forward_survive_reconstruction() {
        use nickel_ui::{Point, Rect, UiEvent, UiStateStore, UiTree};

        let directory = tempdir().unwrap();
        let first = directory.path().join("first.md");
        let second = directory.path().join("second.md");
        let third = directory.path().join("third.md");
        write(&first, b"first");
        write(&second, b"[third](third.md)");
        write(&third, b"third");
        let mut model = ViewerModel::default();
        for path in [&first, &second, &third] {
            let request = model.begin_open(path);
            model.complete(load_document(&request));
        }
        let request = model.begin_back().unwrap();
        model.complete(load_document(&request));
        let bounds = Rect::new(0.0, 0.0, 960.0, 720.0);
        for expected in [
            ViewerMessage::Back,
            ViewerMessage::Forward,
            ViewerMessage::Reload,
            ViewerMessage::Link("third.md".into()),
        ] {
            let mut state = UiStateStore::default();
            let tree = UiTree::layout_with_state(viewer_view(&model, None), bounds, &mut state);
            let rect = tree.message_rect(&expected).expect("typed action");
            let point = Point {
                x: rect.origin.x + rect.size.width * 0.5,
                y: rect.origin.y + rect.size.height * 0.5,
            };
            tree.handle_event(&mut state, UiEvent::PointerPressed(point));
            let rebuilt = UiTree::layout_with_state(viewer_view(&model, None), bounds, &mut state);
            assert_eq!(
                rebuilt
                    .handle_event(&mut state, UiEvent::PointerReleased(point))
                    .messages,
                vec![expected]
            );
        }
    }

    #[test]
    #[cfg(feature = "application")]
    fn toolbar_modalities_converge_on_the_same_typed_action() {
        use nickel_ui::{Rect, UiEvent, UiStateStore, UiTree};

        let document = LoadedDocument {
            path: PathBuf::from("/tmp/guide.md"),
            title: "guide.md".into(),
            source: "# Guide".into(),
            document: MarkdownDocument::parse("# Guide"),
        };
        let application = ViewerApplication::loaded(document);
        let tree = UiTree::layout(
            viewer_view(application.model(), None),
            Rect::new(0.0, 0.0, 960.0, 720.0),
        );
        let id = tree
            .id_for_message(&ViewerMessage::Reload)
            .expect("reload identity")
            .clone();
        for event in [UiEvent::KeyboardActivate, UiEvent::ControllerActivate] {
            let mut state = UiStateStore::default();
            tree.reconcile_state(&mut state);
            tree.handle_event(&mut state, UiEvent::FocusNext);
            assert_eq!(
                tree.handle_event(&mut state, event).messages,
                vec![ViewerMessage::Reload]
            );
        }
        let mut state = UiStateStore::default();
        tree.reconcile_state(&mut state);
        assert_eq!(
            tree.handle_event(&mut state, UiEvent::AccessibilityActivate(id))
                .messages,
            vec![ViewerMessage::Reload]
        );
    }

    #[test]
    #[cfg(feature = "application")]
    fn recoverable_status_keeps_loaded_document_visible() {
        use nickel_ui::{PaintCommand, Rect, UiTree};

        let document = LoadedDocument {
            path: PathBuf::from("/tmp/guide.md"),
            title: "guide.md".into(),
            source: "# Still visible".into(),
            document: MarkdownDocument::parse("# Still visible"),
        };
        let application = ViewerApplication::loaded(document);
        let tree = UiTree::layout(
            viewer_view(application.model(), Some("Could not reload")),
            Rect::new(0.0, 0.0, 800.0, 600.0),
        );
        assert!(tree.commands().iter().any(|command| {
            matches!(command, PaintCommand::StyledText { text, .. } if text == "Still visible")
        }));
        assert!(tree.commands().iter().any(|command| {
            matches!(command, PaintCommand::Text { text, .. } if text == "Could not reload")
        }));
        assert!(tree.message_rect(&ViewerMessage::DismissStatus).is_some());
    }

    #[test]
    #[cfg(feature = "application")]
    fn startup_failure_renders_the_path_and_classified_reason() {
        use nickel_ui::{PaintCommand, Rect, UiTree};

        let mut model = ViewerModel::default();
        let missing = PathBuf::from("/definitely/missing/guide.md");
        let request = model.begin_open(&missing);
        model.complete(load_document(&request));
        let tree = UiTree::layout(viewer_view(&model, None), Rect::new(0.0, 0.0, 800.0, 600.0));
        let visible = tree
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(visible.contains("/definitely/missing/guide.md"));
        assert!(visible.contains("No such file") || visible.contains("not found"));
    }

    #[test]
    #[cfg(feature = "application")]
    fn light_and_dark_viewer_palettes_are_complete_and_distinct() {
        use nickel_core::theme::{Appearance, ThemeMode};

        let dark = ViewerPalette::from_appearance(Appearance::default());
        let light = ViewerPalette::from_appearance(Appearance {
            mode: ThemeMode::Light,
            ..Appearance::default()
        });
        assert_ne!(dark.background, light.background);
        assert_ne!(dark.text, light.text);
        for palette in [dark, light] {
            assert_ne!(palette.background, palette.text);
            assert_ne!(palette.panel, palette.text);
            assert_ne!(palette.accent, palette.background);
            assert_ne!(palette.error, palette.background);
        }
    }

    #[test]
    #[cfg(feature = "application")]
    fn equal_viewer_state_reconstructs_identical_paint_commands() {
        use nickel_ui::{Rect, UiTree};

        let document = LoadedDocument {
            path: PathBuf::from("/tmp/guide.md"),
            title: "guide.md".into(),
            source: "# Stable\n\nContent".into(),
            document: MarkdownDocument::parse("# Stable\n\nContent"),
        };
        let application = ViewerApplication::loaded(document);
        let render = || {
            UiTree::layout(
                viewer_view(application.model(), Some("Stable status")),
                Rect::new(0.0, 0.0, 960.0, 720.0),
            )
            .commands()
            .to_vec()
        };
        assert_eq!(render(), render());
    }

    #[test]
    #[cfg(feature = "application")]
    fn document_boundary_and_escape_shortcuts_preserve_the_document() {
        let document = LoadedDocument {
            path: PathBuf::from("/tmp/guide.md"),
            title: "guide.md".into(),
            source: "# Guide".into(),
            document: MarkdownDocument::parse("# Guide"),
        };
        let mut application = ViewerApplication::loaded(document);
        application.model.set_scroll_position(42.0);
        assert!(application.shortcut(Shortcut::DocumentEnd));
        assert_eq!(application.model.scroll_position(), f32::MAX);
        assert!(application.shortcut(Shortcut::DocumentStart));
        assert_eq!(application.model.scroll_position(), 0.0);
        application.runtime_error = Some("Recoverable".into());
        assert!(application.shortcut(Shortcut::Escape));
        assert!(application.runtime_error.is_none());
        assert_eq!(application.model.current().unwrap().source, "# Guide");
    }
}
