//! Nickel-owned remote settings policy shared by the Linux and Windows owners.

use nickel_core::{
    dpi::{ApplicationScalePolicy, ApplicationScaleSettings, Scale120},
    launcher_preferences::LauncherPreferences,
    terminal_settings::{TerminalCursorStyle, TerminalSettings},
};
use nickel_remote_control::{application_scale, launcher_favorites, terminal_presentation};
use std::collections::HashSet;

pub(crate) trait FavoriteCatalog {
    fn resolve(&self, stored: &str) -> Option<&str>;
    fn exact(&self, requested: &str) -> Option<&str>;
}

pub(crate) fn favorite_projection(
    preferences: &LauncherPreferences,
    catalog: &impl FavoriteCatalog,
) -> (Vec<String>, usize) {
    let mut favorites = Vec::new();
    let mut unavailable = 0;
    for stored in preferences.favorites() {
        match catalog.resolve(stored) {
            Some(id)
                if !id.is_empty()
                    && id.len() <= launcher_favorites::MAX_APPLICATION_ID_BYTES
                    && !id.chars().any(char::is_control)
                    && !favorites.iter().any(|entry| entry == id) =>
            {
                favorites.push(id.to_owned());
            }
            _ => unavailable += 1,
        }
    }
    (favorites, unavailable)
}

pub(crate) fn changed_favorites(
    prior: &LauncherPreferences,
    catalog: &impl FavoriteCatalog,
    change: &launcher_favorites::Change,
    stale: &str,
) -> Result<LauncherPreferences, String> {
    use launcher_favorites::Change;

    let mut favorites = prior.favorites().to_vec();
    match change {
        Change::Add { application_id } => {
            let id = catalog
                .exact(application_id)
                .ok_or("installed application is unavailable")?;
            if !favorites
                .iter()
                .any(|stored| catalog.resolve(stored) == Some(id))
            {
                if favorites.len() == launcher_favorites::MAX_FAVORITES {
                    return Err("favorite limit reached".into());
                }
                favorites.push(id.to_owned());
            }
        }
        Change::Remove { application_id } => {
            let id = catalog
                .exact(application_id)
                .ok_or("installed application is unavailable")?;
            favorites.retain(|stored| catalog.resolve(stored) != Some(id));
        }
        Change::Reorder { application_ids } => {
            if !launcher_favorites::valid_ids(application_ids) {
                return Err("invalid favorites reorder".into());
            }
            let (visible, _) = favorite_projection(prior, catalog);
            if application_ids.len() != visible.len()
                || !application_ids.iter().all(|id| visible.contains(id))
            {
                return Err(
                    "reorder must contain each current installed favorite exactly once".into(),
                );
            }
            let mut ordered = application_ids.iter();
            let mut emitted = HashSet::new();
            for stored in &mut favorites {
                if let Some(id) = catalog.resolve(stored)
                    && visible.iter().any(|visible| visible == id)
                    && emitted.insert(id.to_owned())
                {
                    *stored = ordered.next().ok_or(stale)?.clone();
                }
            }
        }
    }
    let mut requested = prior.clone();
    requested.replace_favorites(favorites);
    Ok(requested)
}

pub(crate) fn terminal_preferences(
    settings: &TerminalSettings,
) -> terminal_presentation::Preferences {
    terminal_presentation::Preferences {
        font_family: settings.font_family.clone(),
        font_size_tenths: settings.font_size_tenths,
        scrollback_lines: settings.scrollback_lines,
        cursor_style: match settings.cursor_style {
            TerminalCursorStyle::Block => terminal_presentation::CursorStyle::Block,
            TerminalCursorStyle::Beam => terminal_presentation::CursorStyle::Beam,
            TerminalCursorStyle::Underline => terminal_presentation::CursorStyle::Underline,
        },
        foreground: settings.foreground,
        background: settings.background,
        close_on_successful_exit: settings.close_on_successful_exit,
    }
}

pub(crate) fn apply_terminal_preferences(
    settings: &mut TerminalSettings,
    requested: &terminal_presentation::Preferences,
) -> Result<(), String> {
    if !requested.valid() {
        return Err("terminal presentation value is outside its supported range".into());
    }
    settings.font_family.clone_from(&requested.font_family);
    settings.font_size_tenths = requested.font_size_tenths;
    settings.scrollback_lines = requested.scrollback_lines;
    settings.cursor_style = match requested.cursor_style {
        terminal_presentation::CursorStyle::Block => TerminalCursorStyle::Block,
        terminal_presentation::CursorStyle::Beam => TerminalCursorStyle::Beam,
        terminal_presentation::CursorStyle::Underline => TerminalCursorStyle::Underline,
    };
    settings.foreground = requested.foreground;
    settings.background = requested.background;
    settings.close_on_successful_exit = requested.close_on_successful_exit;
    Ok(())
}

pub(crate) fn application_scale_policy(value: ApplicationScalePolicy) -> application_scale::Policy {
    match value {
        ApplicationScalePolicy::FollowNickel => application_scale::Policy::FollowNickel,
        ApplicationScalePolicy::Unchanged => application_scale::Policy::Unchanged,
        ApplicationScalePolicy::Custom(scale) => application_scale::Policy::Custom {
            scale_120: scale.units(),
        },
    }
}

pub(crate) fn requested_application_scale_policy(
    value: application_scale::Policy,
) -> Result<ApplicationScalePolicy, String> {
    Ok(match value {
        application_scale::Policy::FollowNickel => ApplicationScalePolicy::FollowNickel,
        application_scale::Policy::Unchanged => ApplicationScalePolicy::Unchanged,
        application_scale::Policy::Custom { scale_120 } => {
            if !(60..=480).contains(&scale_120) || scale_120 % 30 != 0 {
                return Err("unsupported application scale".into());
            }
            ApplicationScalePolicy::Custom(
                Scale120::new(scale_120).ok_or("unsupported application scale")?,
            )
        }
    })
}

pub(crate) type ScaleObservation<R> =
    (R, ApplicationScaleSettings, Vec<application_scale::Toolkit>);

pub(crate) fn scale_observation_generation<R: PartialEq>(
    generation: u64,
    previous: Option<&ScaleObservation<R>>,
    current: &ScaleObservation<R>,
) -> Result<u64, String> {
    if previous == Some(current) {
        Ok(generation)
    } else {
        generation
            .checked_add(1)
            .ok_or_else(|| "scale generation exhausted".into())
    }
}

pub(crate) fn scale_observation_matches<R: PartialEq>(
    generation: u64,
    previous: Option<&ScaleObservation<R>>,
    current: &ScaleObservation<R>,
    transaction: &application_scale::Transaction,
) -> bool {
    generation != 0
        && generation != u64::MAX
        && transaction.generation == generation
        && transaction.prior == application_scale_policy(current.1.policy)
        && previous == Some(current)
}

pub(crate) fn scale_snapshot(
    generation: u64,
    observation_started_at_us: u64,
    observed_at_us: u64,
    settings: &ApplicationScaleSettings,
    toolkits: Vec<application_scale::Toolkit>,
) -> application_scale::Snapshot {
    application_scale::Snapshot {
        generation,
        observation_started_at_us,
        observed_at_us,
        atomic: false,
        policy: application_scale_policy(settings.policy),
        toolkits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Catalog;

    impl FavoriteCatalog for Catalog {
        fn resolve(&self, stored: &str) -> Option<&str> {
            match stored {
                "one" | "One" => Some("one"),
                "two" | "Two" => Some("two"),
                _ => None,
            }
        }

        fn exact(&self, requested: &str) -> Option<&str> {
            match requested {
                "one" => Some("one"),
                "two" => Some("two"),
                _ => None,
            }
        }
    }

    #[test]
    fn favorite_edits_preserve_hidden_entries_and_resolve_aliases() {
        let mut prior = LauncherPreferences::default();
        prior.replace_favorites(vec!["One".into(), "hidden".into(), "two".into()]);
        assert_eq!(
            favorite_projection(&prior, &Catalog),
            (vec!["one".into(), "two".into()], 1)
        );
        let changed = changed_favorites(
            &prior,
            &Catalog,
            &launcher_favorites::Change::Reorder {
                application_ids: vec!["two".into(), "one".into()],
            },
            "stale",
        )
        .unwrap();
        assert_eq!(
            changed.favorites(),
            &["two".to_owned(), "hidden".to_owned(), "one".to_owned()]
        );
        assert!(
            changed_favorites(
                &prior,
                &Catalog,
                &launcher_favorites::Change::Reorder {
                    application_ids: vec!["one".into()],
                },
                "stale",
            )
            .is_err()
        );
    }

    #[test]
    fn terminal_presentation_keeps_private_launch_fields() {
        let mut settings = TerminalSettings {
            default_shell: Some("private-shell".into()),
            initial_working_directory: Some("private-directory".into()),
            ..TerminalSettings::default()
        };
        let mut requested = terminal_preferences(&settings);
        requested.font_size_tenths += 10;
        apply_terminal_preferences(&mut settings, &requested).unwrap();
        assert_eq!(terminal_preferences(&settings), requested);
        assert_eq!(settings.default_shell.as_deref(), Some("private-shell"));
        assert_eq!(
            settings.initial_working_directory.as_deref(),
            Some(std::path::Path::new("private-directory"))
        );
    }

    #[test]
    fn application_scale_accepts_only_supported_steps() {
        for scale_120 in [59, 61, 481, 75] {
            assert!(
                requested_application_scale_policy(application_scale::Policy::Custom { scale_120 })
                    .is_err()
            );
        }
        for scale_120 in [60, 120, 480] {
            let requested = application_scale::Policy::Custom { scale_120 };
            assert_eq!(
                application_scale_policy(requested_application_scale_policy(requested).unwrap()),
                requested
            );
        }
    }

    #[test]
    fn scale_observation_generation_and_staleness_are_shared() {
        let current = (1_u8, ApplicationScaleSettings::default(), Vec::new());
        let next = (2_u8, ApplicationScaleSettings::default(), Vec::new());
        assert_eq!(scale_observation_generation(0, None, &current), Ok(1));
        assert_eq!(
            scale_observation_generation(1, Some(&current), &current),
            Ok(1)
        );
        assert_eq!(
            scale_observation_generation(1, Some(&current), &next),
            Ok(2)
        );
        assert!(scale_observation_generation(u64::MAX, None, &current).is_err());

        let transaction = application_scale::Transaction {
            generation: 1,
            prior: application_scale_policy(current.1.policy),
            requested: application_scale::Policy::Unchanged,
        };
        assert!(scale_observation_matches(
            1,
            Some(&current),
            &current,
            &transaction
        ));
        assert!(!scale_observation_matches(
            1,
            Some(&current),
            &next,
            &transaction
        ));
        assert!(!scale_observation_matches(
            0,
            Some(&current),
            &current,
            &transaction
        ));
        let snapshot = scale_snapshot(1, 10, 20, &current.1, Vec::new());
        assert_eq!(snapshot.generation, 1);
        assert_eq!(snapshot.observation_started_at_us, 10);
        assert_eq!(snapshot.observed_at_us, 20);
        assert!(!snapshot.atomic);
    }
}
