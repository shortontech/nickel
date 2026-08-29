use std::ffi::OsString;

use crate::SettingsPage;

pub(super) const HELP: &str = "Nickel Settings\n\nUsage: nickel-settings [OPTIONS]\n\nOptions:\n  -s, --screen <SCREEN>  Screen to show initially [default: display]\n                         [values: display, nickel-bar, appearance, network, bluetooth, keyboard-shortcuts, about]\n  -h, --help             Print help\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Action {
    Run(SettingsPage),
    Help,
}

pub(super) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action, String> {
    let mut args = args.into_iter();
    let mut screen = None;

    while let Some(argument) = args.next() {
        let argument = argument
            .into_string()
            .map_err(|_| "arguments must be valid Unicode".to_owned())?;

        match argument.as_str() {
            "-h" | "--help" => return Ok(Action::Help),
            "-s" | "--screen" => {
                if screen.is_some() {
                    return Err("--screen may only be specified once".to_owned());
                }
                let value = args
                    .next()
                    .ok_or_else(|| "--screen requires a value".to_owned())?
                    .into_string()
                    .map_err(|_| "the --screen value must be valid Unicode".to_owned())?;
                screen = Some(parse_screen(&value)?);
            }
            _ if argument.starts_with("--screen=") => {
                if screen.is_some() {
                    return Err("--screen may only be specified once".to_owned());
                }
                screen = Some(parse_screen(&argument["--screen=".len()..])?);
            }
            _ => return Err(format!("unexpected argument '{argument}'")),
        }
    }

    Ok(Action::Run(screen.unwrap_or(SettingsPage::Display)))
}

fn parse_screen(value: &str) -> Result<SettingsPage, String> {
    match value {
        "display" => Ok(SettingsPage::Display),
        "bar" | "nickel-bar" => Ok(SettingsPage::Bar),
        "appearance" => Ok(SettingsPage::Appearance),
        "network" => Ok(SettingsPage::Network),
        "bluetooth" => Ok(SettingsPage::Bluetooth),
        "keyboard" | "keyboard-shortcuts" => Ok(SettingsPage::KeyboardShortcuts),
        "about" => Ok(SettingsPage::About),
        _ => Err(format!(
            "unknown screen '{value}'; expected display, nickel-bar, appearance, network, bluetooth, keyboard-shortcuts, or about"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strings(args: &[&str]) -> Result<Action, String> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_selects_display() {
        assert_eq!(parse_strings(&[]), Ok(Action::Run(SettingsPage::Display)));
    }

    #[test]
    fn screen_option_selects_every_page() {
        let cases = [
            ("display", SettingsPage::Display),
            ("nickel-bar", SettingsPage::Bar),
            ("bar", SettingsPage::Bar),
            ("appearance", SettingsPage::Appearance),
            ("network", SettingsPage::Network),
            ("bluetooth", SettingsPage::Bluetooth),
            ("keyboard", SettingsPage::KeyboardShortcuts),
            ("keyboard-shortcuts", SettingsPage::KeyboardShortcuts),
            ("about", SettingsPage::About),
        ];

        for (name, page) in cases {
            assert_eq!(
                parse_strings(&["--screen", name]),
                Ok(Action::Run(page)),
                "failed to parse {name}"
            );
        }
    }

    #[test]
    fn short_and_equals_forms_are_supported() {
        assert_eq!(
            parse_strings(&["-s", "network"]),
            Ok(Action::Run(SettingsPage::Network))
        );
        assert_eq!(
            parse_strings(&["--screen=appearance"]),
            Ok(Action::Run(SettingsPage::Appearance))
        );
    }

    #[test]
    fn help_is_available() {
        assert_eq!(parse_strings(&["--help"]), Ok(Action::Help));
        assert!(HELP.contains("--screen <SCREEN>"));
        assert!(HELP.contains("nickel-bar"));
    }

    #[test]
    fn invalid_arguments_have_specific_errors() {
        assert_eq!(
            parse_strings(&["--screen"]),
            Err("--screen requires a value".to_owned())
        );
        assert!(
            parse_strings(&["--screen", "sound"])
                .unwrap_err()
                .contains("unknown screen 'sound'")
        );
        assert_eq!(
            parse_strings(&["--bogus"]),
            Err("unexpected argument '--bogus'".to_owned())
        );
        assert_eq!(
            parse_strings(&["--screen=display", "--screen", "network"]),
            Err("--screen may only be specified once".to_owned())
        );
    }
}
