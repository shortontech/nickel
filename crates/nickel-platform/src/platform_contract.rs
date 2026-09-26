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
    RemoteDisplayObservation,
    RemoteDisplayTransactions,
    RemotePeripheralObservations,
    RemotePeripheralControls,
    RemoteSurfacePointer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractEvidence {
    FixtureOnly,
    NativeReadVerified,
    NativeInputVerified,
    NativeMutationVerified,
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

const fn native_read(
    platform: PlatformFamily,
    capability: AdapterCapability,
    adapter: &'static str,
    fixture: &'static str,
    native_evidence: &'static str,
) -> PlatformContract {
    PlatformContract {
        platform,
        capability,
        adapter,
        fixture,
        evidence: ContractEvidence::NativeReadVerified,
        live_evidence: Some(native_evidence),
        available: true,
    }
}

const fn native_input(
    platform: PlatformFamily,
    capability: AdapterCapability,
    adapter: &'static str,
    fixture: &'static str,
    native_evidence: &'static str,
) -> PlatformContract {
    PlatformContract {
        platform,
        capability,
        adapter,
        fixture,
        evidence: ContractEvidence::NativeInputVerified,
        live_evidence: Some(native_evidence),
        available: true,
    }
}

const fn native_mutation(
    platform: PlatformFamily,
    capability: AdapterCapability,
    adapter: &'static str,
    fixture: &'static str,
    native_evidence: &'static str,
) -> PlatformContract {
    PlatformContract {
        platform,
        capability,
        adapter,
        fixture,
        evidence: ContractEvidence::NativeMutationVerified,
        live_evidence: Some(native_evidence),
        available: true,
    }
}

/// Declarative adapter coverage. Native read, input, and mutation evidence may exercise
/// an in-process authenticated owner; neither implies that the network listener
/// accepted a complete remote request.
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
        AdapterCapability::RemoteDisplayObservation,
        "Linux compositor output layout observation owner",
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
        AdapterCapability::RemotePeripheralObservations,
        "Linux bounded peripheral observation owner",
        "remote_peripheral_controls::tests::projection_scrubs_native_text_paths_and_clamps_capacity",
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
    native_read(
        PlatformFamily::Windows,
        AdapterCapability::RemoteDisplayObservation,
        "Windows DisplayConfig and native output observation owner",
        "windows_remote_display_topology::tests::incomplete_native_topology_still_exposes_active_layout_without_transactions",
        "windows_remote_control::tests::native_display_owner_read_reports_transaction_prerequisites",
    ),
    native_mutation(
        PlatformFamily::Windows,
        AdapterCapability::RemoteDisplayTransactions,
        "Windows temporary DisplayConfig transaction and guarded recovery owner",
        "windows_remote_display_topology::tests::supplied_configuration_moves_only_validated_source_positions",
        "windows_remote_control::tests::native_display_owner_apply_keep_and_revert_restore_prior_layout [NICKEL_WINDOWS_DISPLAY_OWNER_MUTATION_TEST=1]",
    ),
    native_read(
        PlatformFamily::Windows,
        AdapterCapability::RemotePeripheralObservations,
        "Windows bounded printer and volume observation owner",
        "remote_peripheral_controls::tests::projection_scrubs_native_text_paths_and_clamps_capacity",
        "windows_remote_control::tests::native_peripheral_owner_read_requires_live_debug_authority",
    ),
    unavailable(
        PlatformFamily::Windows,
        AdapterCapability::RemotePeripheralControls,
        "native Windows peripheral mutations lack a cancellable owner",
        "platform_contract::tests::matrix_is_complete_and_truthful",
    ),
    native_input(
        PlatformFamily::Windows,
        AdapterCapability::RemoteSurfacePointer,
        "Windows shell surface pointer owner",
        "windows_resource_owner::tests::shell_surface_pointer_coordinates_use_logical_client_space",
        "windows_remote_control::tests::native_shell_surface_pointer_moves_through_desktop_owner [NICKEL_WINDOWS_OWNER_SURFACE_POINTER_MOVE_TEST=1]",
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
                ContractEvidence::NativeReadVerified
                | ContractEvidence::NativeInputVerified
                | ContractEvidence::NativeMutationVerified
                | ContractEvidence::LiveVerified => {
                    assert!(contract.live_evidence.is_some())
                }
            }
        }
        assert_eq!(keys.len(), 2 * 10);
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
                AdapterCapability::RemoteDisplayObservation => "remote_display_observation",
                AdapterCapability::RemoteDisplayTransactions => "remote_display_transactions",
                AdapterCapability::RemotePeripheralObservations => "remote_peripheral_observations",
                AdapterCapability::RemotePeripheralControls => "remote_peripheral_controls",
                AdapterCapability::RemoteSurfacePointer => "remote_surface_pointer",
            };
            assert_eq!(row[0], platform);
            assert_eq!(row[1], capability);
            assert_eq!(row[3], contract.fixture);
            assert_eq!(
                row[5],
                match contract.evidence {
                    ContractEvidence::FixtureOnly => "fixture_only",
                    ContractEvidence::NativeReadVerified => "native_read_verified",
                    ContractEvidence::NativeInputVerified => "native_input_verified",
                    ContractEvidence::NativeMutationVerified => "native_mutation_verified",
                    ContractEvidence::LiveVerified => "live_verified",
                }
            );
            assert_eq!(row[6], contract.live_evidence.unwrap_or("none"));
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
