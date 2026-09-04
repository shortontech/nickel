use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

const DEFAULT_LOCALE: &str = "en-US";
const EN_US: &str = include_str!("../locales/en-US/settings.ftl");
const ES: &str = include_str!("../locales/es/settings.ftl");
const DE: &str = include_str!("../locales/de/settings.ftl");
const ZH: &str = include_str!("../locales/zh/settings.ftl");
const AR: &str = include_str!("../locales/ar/settings.ftl");

/// Typed, shared labels for semantic controller actions.
///
/// Keeping these keys here makes the Fluent catalogs the single translation
/// authority while UI components remain responsible only for presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionLabel {
    Open,
    Select,
    Close,
    Back,
    Actions,
    Pin,
    Unpin,
    PreviousSection,
    NextSection,
    Launcher,
    Sidebar,
    Content,
}

impl ActionLabel {
    const fn message_id(self) -> &'static str {
        match self {
            Self::Open => "action-open",
            Self::Select => "action-select",
            Self::Close => "action-close",
            Self::Back => "action-back",
            Self::Actions => "action-actions",
            Self::Pin => "action-pin",
            Self::Unpin => "action-unpin",
            Self::PreviousSection => "action-previous-section",
            Self::NextSection => "action-next-section",
            Self::Launcher => "action-launcher",
            Self::Sidebar => "action-sidebar",
            Self::Content => "action-content",
        }
    }
}

pub struct Localizer {
    selected: FluentBundle<FluentResource>,
    fallback: Option<FluentBundle<FluentResource>>,
    right_to_left: bool,
}

impl Default for Localizer {
    fn default() -> Self {
        Self::system()
    }
}

impl Localizer {
    pub fn system() -> Self {
        let requested = std::env::var("NICKEL_LOCALE")
            .ok()
            .or_else(sys_locale::get_locale);
        Self::for_locale(requested.as_deref())
    }

    pub fn for_locale(locale: Option<&str>) -> Self {
        let requested = locale
            .and_then(normalize_locale)
            .unwrap_or_else(default_language);
        let language = requested.language.as_str();
        let right_to_left = matches!(language, "ar" | "fa" | "he" | "ur");
        let (locale, source, fallback) = match language {
            "es" => (requested, ES, Some(bundle(default_language(), EN_US))),
            "de" => (requested, DE, Some(bundle(default_language(), EN_US))),
            "zh" => (requested, ZH, Some(bundle(default_language(), EN_US))),
            "ar" => (requested, AR, Some(bundle(default_language(), EN_US))),
            _ => (default_language(), EN_US, None),
        };
        Self {
            selected: bundle(locale, source),
            fallback,
            right_to_left,
        }
    }

    pub fn is_right_to_left(&self) -> bool {
        self.right_to_left
    }

    pub fn text(&self, id: &str) -> String {
        self.format(id, None)
    }

    pub fn action_label(&self, label: ActionLabel) -> String {
        self.text(label.message_id())
    }

    pub fn number(&self, id: &str, name: &str, value: i64) -> String {
        let args = args([(name, FluentValue::from(value))]);
        self.format(id, Some(&args))
    }

    pub fn value(&self, id: &str, name: &str, value: &str) -> String {
        let args = args([(name, FluentValue::from(value))]);
        self.format(id, Some(&args))
    }

    /// Formats a logical byte count through the active locale. The exact count
    /// remains available to callers; this is only the compact visible label.
    pub fn bytes(&self, bytes: u64) -> String {
        let (id, value) = if bytes < 1_024 {
            ("size-bytes", bytes as f64)
        } else if bytes < 1_048_576 {
            ("size-kibibytes", bytes as f64 / 1_024.0)
        } else if bytes < 1_073_741_824 {
            ("size-mebibytes", bytes as f64 / 1_048_576.0)
        } else if bytes < 1_099_511_627_776 {
            ("size-gibibytes", bytes as f64 / 1_073_741_824.0)
        } else {
            ("size-tebibytes", bytes as f64 / 1_099_511_627_776.0)
        };
        let args = args([("value", FluentValue::from(value))]);
        self.format(id, Some(&args))
    }

    pub fn file_selection_summary(&self, count: usize, size: Option<&str>) -> String {
        let count_label = self.number("file-selection-count", "count", count as i64);
        let Some(size) = size else { return count_label };
        let values = args([
            ("count", FluentValue::from(count_label)),
            ("size", FluentValue::from(size)),
        ]);
        self.format("file-selection-summary", Some(&values))
    }

    pub fn file_selection_accessible_bytes(&self, count: usize, bytes: Option<u64>) -> String {
        let count_label = self.number("file-selection-count", "count", count as i64);
        let Some(bytes) = bytes else {
            return count_label;
        };
        let bytes = bytes.to_string();
        let values = args([
            ("count", FluentValue::from(count_label)),
            ("bytes", FluentValue::from(bytes)),
        ]);
        self.format("file-selection-accessible-bytes", Some(&values))
    }

    pub fn format(&self, id: &str, args: Option<&FluentArgs<'_>>) -> String {
        format_from(&self.selected, id, args)
            .or_else(|| {
                self.fallback
                    .as_ref()
                    .and_then(|bundle| format_from(bundle, id, args))
            })
            .unwrap_or_else(|| id.to_owned())
    }
}

pub fn args<'a, I>(values: I) -> FluentArgs<'a>
where
    I: IntoIterator<Item = (&'a str, FluentValue<'a>)>,
{
    let mut args = FluentArgs::new();
    for (key, value) in values {
        args.set(key, value);
    }
    args
}

fn normalize_locale(locale: &str) -> Option<LanguageIdentifier> {
    let locale = locale.split(['.', '@']).next()?.replace('_', "-");
    locale.parse().ok()
}

fn default_language() -> LanguageIdentifier {
    DEFAULT_LOCALE
        .parse()
        .expect("default locale must be valid")
}

fn bundle(locale: LanguageIdentifier, source: &str) -> FluentBundle<FluentResource> {
    let resource = FluentResource::try_new(source.to_owned())
        .unwrap_or_else(|(_, errors)| panic!("invalid embedded Fluent catalog: {errors:?}"));
    let mut bundle = FluentBundle::new(vec![locale]);
    bundle.set_use_isolating(false);
    bundle
        .add_resource(resource)
        .expect("embedded Fluent message identifiers must be unique");
    bundle
}

fn format_from(
    bundle: &FluentBundle<FluentResource>,
    id: &str,
    args: Option<&FluentArgs<'_>>,
) -> Option<String> {
    let message = bundle.get_message(id)?;
    let pattern = message.value()?;
    let mut errors = Vec::new();
    Some(
        bundle
            .format_pattern(pattern, args, &mut errors)
            .into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_locale_uses_english() {
        let localizer = Localizer::for_locale(Some("zz-ZZ"));
        assert_eq!(localizer.text("settings-nav-display"), "Display");
    }

    #[test]
    fn locale_with_posix_suffix_is_normalized() {
        let localizer = Localizer::for_locale(Some("es_ES.UTF-8"));
        assert_eq!(localizer.text("settings-nav-display"), "Pantallas");
    }

    #[test]
    fn missing_translation_falls_back_to_english() {
        let localizer = Localizer::for_locale(Some("es"));
        assert_eq!(localizer.text("settings-swatch-hover"), "Hover");
    }

    #[test]
    fn fluent_selects_plural_form() {
        let localizer = Localizer::for_locale(Some("en-US"));
        let one = args([("count", FluentValue::from(1))]);
        let many = args([("count", FluentValue::from(4))]);
        assert_eq!(
            localizer.format("settings-bar-desktop-count", Some(&one)),
            "1 desktop"
        );
        assert_eq!(
            localizer.format("settings-bar-desktop-count", Some(&many)),
            "4 desktops"
        );
    }

    #[test]
    fn appearance_fixture_locales_select_native_copy_and_direction() {
        assert_eq!(
            Localizer::for_locale(Some("de-DE")).text("settings-appearance-title"),
            "Darstellung"
        );
        assert_eq!(
            Localizer::for_locale(Some("zh-CN")).text("settings-appearance-title"),
            "外观"
        );
        assert!(!Localizer::for_locale(Some("es-MX")).is_right_to_left());
        assert!(Localizer::for_locale(Some("ar")).is_right_to_left());
    }

    #[test]
    fn file_sizes_and_selection_counts_use_the_selected_locale() {
        let spanish = Localizer::for_locale(Some("es"));
        assert_eq!(
            spanish.file_selection_summary(2, Some(&spanish.bytes(2048))),
            "2 seleccionados · 2 KiB"
        );
        let arabic = Localizer::for_locale(Some("ar"));
        assert!(arabic.file_selection_summary(2, None).contains("تم تحديد"));
    }
}
