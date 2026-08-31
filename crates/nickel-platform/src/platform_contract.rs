#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PlatformFamily {
    Linux,
    Windows,
    MacOs,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AdapterCapability {
    ImageFileDialog,
    ExternalUrl,
    PathIcon,
    Appearance,
    HiddenFilesPreference,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractEvidence {
    FixtureOnly,
    LiveVerified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformContract {
    pub platform: PlatformFamily,
    pub capability: AdapterCapability,
    pub adapter: &'static str,
    pub fixture: &'static str,
    pub evidence: ContractEvidence,
    pub live_evidence: Option<&'static str>,
}

const fn fixture(
    platform: PlatformFamily,
    capability: AdapterCapability,
    adapter: &'static str,
    fixture: &'static str,
) -> PlatformContract {
    PlatformContract {
        platform,
        capability,
        adapter,
        fixture,
        evidence: ContractEvidence::FixtureOnly,
        live_evidence: None,
    }
}

/// Declarative adapter coverage. `FixtureOnly` is deliberate: a cross-compiled
/// test or pure parsing fixture is not evidence that a native portal, registry,
/// shell, window manager, or physical display was exercised.
pub const PLATFORM_CONTRACTS: &[PlatformContract] = &[
    fixture(
        PlatformFamily::Linux,
        AdapterCapability::ImageFileDialog,
        "xdg-desktop-portal FileChooser",
        "linux::tests::portal_file_uris_preserve_unix_paths_and_percent_escapes",
    ),
    fixture(
        PlatformFamily::Linux,
        AdapterCapability::ExternalUrl,
        "xdg-desktop-portal OpenURI",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::Linux,
        AdapterCapability::PathIcon,
        "freedesktop icon themes",
        "linux::tests::icon_names_follow_path_kind",
    ),
    fixture(
        PlatformFamily::Linux,
        AdapterCapability::Appearance,
        "portable default appearance",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::Linux,
        AdapterCapability::HiddenFilesPreference,
        "portable false fallback",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::Windows,
        AdapterCapability::ImageFileDialog,
        "SDL native file dialog",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::Windows,
        AdapterCapability::ExternalUrl,
        "ShellExecuteW",
        "windows::tests::utf16_helpers_terminate_and_measure_paths",
    ),
    fixture(
        PlatformFamily::Windows,
        AdapterCapability::PathIcon,
        "Windows Shell and shortcut icon resolver",
        "windows::tests::utf16_helpers_terminate_and_measure_paths",
    ),
    fixture(
        PlatformFamily::Windows,
        AdapterCapability::Appearance,
        "Windows registry and winit chrome",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::Windows,
        AdapterCapability::HiddenFilesPreference,
        "Explorer registry preference",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::MacOs,
        AdapterCapability::ImageFileDialog,
        "SDL native file dialog",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::MacOs,
        AdapterCapability::ExternalUrl,
        "macOS open command",
        "macos::tests::external_url_command_preserves_the_argument",
    ),
    fixture(
        PlatformFamily::MacOs,
        AdapterCapability::PathIcon,
        "unsupported fallback",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::MacOs,
        AdapterCapability::Appearance,
        "portable default appearance",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::MacOs,
        AdapterCapability::HiddenFilesPreference,
        "portable false fallback",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn matrix_is_complete_and_truthful() {
        let mut keys = HashSet::new();
        for contract in PLATFORM_CONTRACTS {
            assert!(keys.insert((contract.platform, contract.capability)));
            assert!(!contract.adapter.is_empty());
            assert!(!contract.fixture.is_empty());
            match contract.evidence {
                ContractEvidence::FixtureOnly => assert!(contract.live_evidence.is_none()),
                ContractEvidence::LiveVerified => assert!(contract.live_evidence.is_some()),
            }
        }
        assert_eq!(keys.len(), 3 * 5);
    }

    #[test]
    fn checked_in_evidence_keeps_fixture_and_live_claims_separate() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/ui-platform-contracts.tsv");
        let contents = std::fs::read_to_string(path).unwrap();
        let mut lines = contents.lines();
        assert_eq!(
            lines.next(),
            Some(
                "platform\tcapability\tadapter\tfixture_evidence\tcompile_evidence\tevidence_level\tlive_evidence\tlive_status"
            )
        );
        let rows = lines
            .map(|line| line.split('\t').collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), PLATFORM_CONTRACTS.len());
        for row in rows {
            assert_eq!(row.len(), 8);
            assert_eq!(row[5], "fixture_only");
            assert_eq!(row[6], "none");
            assert!(!row[3].is_empty());
            assert!(!row[4].is_empty());
            assert!(
                row[7].starts_with("pending_") || row[7].starts_with("not_applicable_"),
                "unexpected live status: {}",
                row[7]
            );
        }
    }
}
