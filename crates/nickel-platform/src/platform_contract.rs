#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PlatformFamily {
    Linux,
    Windows,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AdapterCapability {
    ImageFileDialog,
    ExternalUrl,
    PathIcon,
    Appearance,
    HiddenFilesPreference,
    RemoteDisplayTransactions,
    RemotePeripheralControls,
    RemoteSurfacePointer,
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
    pub available: bool,
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
        available: true,
    }
}

const fn unavailable(
    platform: PlatformFamily,
    capability: AdapterCapability,
    reason: &'static str,
    fixture: &'static str,
) -> PlatformContract {
    PlatformContract {
        platform,
        capability,
        adapter: reason,
        fixture,
        evidence: ContractEvidence::FixtureOnly,
        live_evidence: None,
        available: false,
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
        "freedesktop icon themes and desktop-entry application identity",
        "linux::tests::desktop_entry_declared_icon_precedes_application_identity",
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
        PlatformFamily::Linux,
        AdapterCapability::RemoteDisplayTransactions,
        "Linux compositor output layout owner",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::Linux,
        AdapterCapability::RemotePeripheralControls,
        "Linux bounded peripheral control owner",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::Linux,
        AdapterCapability::RemoteSurfacePointer,
        "Linux compositor surface pointer owner",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::Windows,
        AdapterCapability::ImageFileDialog,
        "Windows common item dialog",
        "windows_file_dialog::tests::image_chooser_distinguishes_cancel_from_native_failure",
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
        "windows::tests::installed_shortcut_icon_has_visible_pixels",
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
        PlatformFamily::Windows,
        AdapterCapability::RemoteDisplayTransactions,
        "Windows temporary DisplayConfig transaction and guarded recovery owner",
        "windows_remote_display_topology::tests::supplied_configuration_moves_only_validated_source_positions",
    ),
    unavailable(
        PlatformFamily::Windows,
        AdapterCapability::RemotePeripheralControls,
        "native Windows peripheral mutations lack a cancellable owner",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    fixture(
        PlatformFamily::Windows,
        AdapterCapability::RemoteSurfacePointer,
        "Windows shell surface pointer owner",
        "windows_resource_owner::tests::shell_surface_pointer_coordinates_use_logical_client_space",
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
        assert_eq!(keys.len(), 2 * 8);
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
                "platform\tcapability\tadapter\tfixture_evidence\tcompile_evidence\tevidence_level\tlive_evidence\tlive_status\tavailable"
            )
        );
        let rows = lines
            .map(|line| line.split('\t').collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), PLATFORM_CONTRACTS.len());
        for (row, contract) in rows.into_iter().zip(PLATFORM_CONTRACTS) {
            assert_eq!(row.len(), 9);
            let platform = match contract.platform {
                PlatformFamily::Linux => "linux",
                PlatformFamily::Windows => "windows",
            };
            let capability = match contract.capability {
                AdapterCapability::ImageFileDialog => "image_file_dialog",
                AdapterCapability::ExternalUrl => "external_url",
                AdapterCapability::PathIcon => "path_icon",
                AdapterCapability::Appearance => "appearance",
                AdapterCapability::HiddenFilesPreference => "hidden_files_preference",
                AdapterCapability::RemoteDisplayTransactions => "remote_display_transactions",
                AdapterCapability::RemotePeripheralControls => "remote_peripheral_controls",
                AdapterCapability::RemoteSurfacePointer => "remote_surface_pointer",
            };
            assert_eq!(row[0], platform);
            assert_eq!(row[1], capability);
            assert_eq!(row[3], contract.fixture);
            assert_eq!(row[5], "fixture_only");
            assert_eq!(row[6], "none");
            assert!(!row[3].is_empty());
            assert!(!row[4].is_empty());
            assert!(
                row[7].starts_with("pending_") || row[7].starts_with("not_applicable_"),
                "unexpected live status: {}",
                row[7]
            );
            assert!(matches!(row[8], "true" | "false"));
            assert_eq!(row[8] == "true", contract.available);
        }
    }
}
