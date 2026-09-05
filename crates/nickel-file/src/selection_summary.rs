use nickel_i18n::Localizer;

use crate::FileEntry;

/// Truthful, filesystem-I/O-free summary derived from current selected metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectionSummary {
    pub count: usize,
    /// Present only when every selected entry is a regular file with reliable
    /// metadata and the sum can be represented exactly.
    pub exact_bytes: Option<u64>,
}

impl SelectionSummary {
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = &'a FileEntry>) -> Self {
        let mut count = 0usize;
        let mut exact_bytes = Some(0u64);
        for entry in entries {
            count = count.saturating_add(1);
            exact_bytes = match (exact_bytes, entry.is_directory, entry.size) {
                (Some(total), false, Some(size)) => total.checked_add(size),
                _ => None,
            };
        }
        Self { count, exact_bytes }
    }

    pub fn visible_label(self, localizer: &Localizer) -> String {
        match self.exact_bytes {
            Some(bytes) if self.count > 0 => {
                let size = localizer.bytes(bytes);
                localizer.file_selection_summary(self.count, Some(&size))
            }
            _ => localizer.file_selection_summary(self.count, None),
        }
    }

    pub fn accessible_label(self, localizer: &Localizer) -> String {
        match self.exact_bytes {
            Some(bytes) if self.count > 0 => {
                localizer.file_selection_accessible_bytes(self.count, Some(bytes))
            }
            _ => localizer.file_selection_accessible_bytes(self.count, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, path::PathBuf};

    fn entry(name: &str, directory: bool, size: Option<u64>) -> FileEntry {
        FileEntry {
            display_name_override: None,
            name: OsString::from(name),
            path: PathBuf::from(name),
            is_directory: directory,
            size,
            modified: None,
        }
    }

    #[test]
    fn sums_only_complete_regular_file_metadata() {
        let zero = entry("zero", false, Some(0));
        let large = entry("large", false, Some(u64::MAX));
        assert_eq!(
            SelectionSummary::from_entries([]),
            SelectionSummary {
                count: 0,
                exact_bytes: Some(0)
            }
        );
        assert_eq!(
            SelectionSummary::from_entries([&zero]),
            SelectionSummary {
                count: 1,
                exact_bytes: Some(0)
            }
        );
        assert_eq!(
            SelectionSummary::from_entries([&large, &zero]).exact_bytes,
            Some(u64::MAX)
        );

        let overflow = entry("overflow", false, Some(1));
        assert_eq!(
            SelectionSummary::from_entries([&large, &overflow]).exact_bytes,
            None
        );
        let unknown = entry("unknown", false, None);
        let folder = entry("folder", true, Some(42));
        assert_eq!(
            SelectionSummary::from_entries([&zero, &unknown]).exact_bytes,
            None
        );
        assert_eq!(
            SelectionSummary::from_entries([&zero, &folder]).exact_bytes,
            None
        );
    }

    #[test]
    fn accessible_label_preserves_exact_bytes() {
        let file = entry("movie", false, Some(1_048_577));
        let summary = SelectionSummary::from_entries([&file]);
        let localizer = Localizer::for_locale(Some("en-US"));
        assert!(summary.visible_label(&localizer).contains("MiB"));
        assert!(
            summary
                .accessible_label(&localizer)
                .contains("1048577 bytes")
        );
    }
}
