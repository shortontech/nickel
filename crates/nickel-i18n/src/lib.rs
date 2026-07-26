use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

const DEFAULT_LOCALE: &str = "en-US";
const EN_US: &str = include_str!("../locales/en-US/settings.ftl");
const ES: &str = include_str!("../locales/es/settings.ftl");

pub struct Localizer {
    selected: FluentBundle<FluentResource>,
    fallback: Option<FluentBundle<FluentResource>>,
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
        let (locale, source, fallback) = if requested.language.as_str() == "es" {
            (requested, ES, Some(bundle(default_language(), EN_US)))
        } else {
            (default_language(), EN_US, None)
        };
        Self {
            selected: bundle(locale, source),
            fallback,
        }
    }

    pub fn text(&self, id: &str) -> String {
        self.format(id, None)
    }

    pub fn number(&self, id: &str, name: &str, value: i64) -> String {
        let args = args([(name, FluentValue::from(value))]);
        self.format(id, Some(&args))
    }

    pub fn value(&self, id: &str, name: &str, value: &str) -> String {
        let args = args([(name, FluentValue::from(value))]);
        self.format(id, Some(&args))
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
}
