use std::collections::{BTreeSet, HashSet};

const LEDGER: &str = include_str!("../../../docs/settings-control-dispositions.tsv");
const MAIN: &str = include_str!("../src/main.rs");
const PAGES: &str = include_str!("../src/view/pages.rs");
const SHELL: &str = include_str!("../src/view/shell.rs");

const HEADER: [&str; 10] = [
    "page",
    "identity",
    "value_shape",
    "mutability",
    "application_timing",
    "current_component",
    "intended_component",
    "disposition",
    "source_marker",
    "evidence",
];

fn rows() -> Vec<Vec<&'static str>> {
    LEDGER
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split('\t').collect())
        .collect()
}

fn settings_messages(source: &str) -> BTreeSet<&str> {
    let prefix = "SettingsMessage::";
    let mut remaining = source;
    let mut messages = BTreeSet::new();
    while let Some(offset) = remaining.find(prefix) {
        remaining = &remaining[offset + prefix.len()..];
        let end = remaining
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .unwrap_or(remaining.len());
        if end > 0 {
            messages.insert(&remaining[..end]);
        }
        remaining = &remaining[end..];
    }
    messages
}

#[test]
fn every_disposition_is_complete_unique_and_tied_to_production_source() {
    assert_eq!(
        LEDGER
            .lines()
            .next()
            .unwrap()
            .split('\t')
            .collect::<Vec<_>>(),
        HEADER
    );
    let source = format!("{MAIN}\n{PAGES}\n{SHELL}");
    let mut identities = HashSet::new();
    let expected_pages = [
        "Shell",
        "Display",
        "Network",
        "Bluetooth",
        "Nickel Bar",
        "Appearance",
        "Default Apps",
        "Optional Features",
        "Keyboard Shortcuts",
        "About Nickel",
    ];
    let mut seen_pages = HashSet::new();

    for (line, row) in rows().into_iter().enumerate() {
        assert_eq!(
            row.len(),
            HEADER.len(),
            "invalid ledger row {}: {row:?}",
            line + 2
        );
        assert!(
            row.iter().all(|field| !field.trim().is_empty()),
            "empty field in row {}",
            line + 2
        );
        assert!(
            identities.insert(row[1]),
            "duplicate control identity `{}`",
            row[1]
        );
        assert!(
            matches!(row[7], "accepted" | "keep-custom"),
            "unresolved disposition `{}` for {}",
            row[7],
            row[1]
        );
        assert!(
            source.contains(row[8]),
            "source marker `{}` for {} is no longer in production Settings views",
            row[8],
            row[1]
        );
        seen_pages.insert(row[0]);
    }
    for page in expected_pages {
        assert!(
            seen_pages.contains(page),
            "Settings page `{page}` has no disposition row"
        );
    }
}

#[test]
fn every_view_action_has_a_checked_in_disposition() {
    let source = format!("{PAGES}\n{SHELL}");
    let missing = settings_messages(&source)
        .into_iter()
        .filter(|message| !LEDGER.contains(message))
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "Settings view actions missing from docs/settings-control-dispositions.tsv: {missing:?}"
    );
}

#[test]
fn low_level_click_composites_are_exactly_the_documented_exceptions() {
    let click_targets = PAGES
        .lines()
        .filter(|line| line.contains("on_press={"))
        .collect::<Vec<_>>();
    let allowed = [
        ("SelectDisplay", "display-card-*", true),
        ("WifiNetwork", "wifi-network-*", true),
        ("BluetoothDevice", "bluetooth-device-*", true),
    ];
    assert_eq!(
        click_targets.len(),
        allowed.len(),
        "undisposed low-level click target: {click_targets:#?}"
    );
    for (message, identity, custom) in allowed {
        assert!(
            click_targets.iter().any(|line| line.contains(message)),
            "missing production composite `{message}`"
        );
        let row = rows()
            .into_iter()
            .find(|row| row[1] == identity)
            .unwrap_or_else(|| panic!("missing disposition for `{identity}`"));
        assert_eq!(
            row[7] == "keep-custom",
            custom,
            "incorrect custom/shared disposition for low-level composite `{identity}`"
        );
    }
}
