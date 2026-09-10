//! Bounded projection policy for an already verified, retained executable file.
//! This does not authenticate an image: the Windows caller must first prove the
//! process mapping and keep its deny-write/delete file handle alive. Identities
//! are host-local, not signatures, publisher trust or persisted launch grants.

use std::sync::Arc;

/// File identity never escapes this pin-owning value. A raw ID can be reused
/// after file retirement; retaining an Arc prevents that lifetime gap.
pub(crate) struct RetainedExecutable<P> {
    inner: Arc<ExecutableFile<P>>,
}
struct ExecutableFile<P> {
    identity: String,
    _pin: P,
}
impl<P> Clone for RetainedExecutable<P> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
impl<P> RetainedExecutable<P> {
    pub(crate) fn new(identity: String, pin: P) -> Self {
        Self {
            inner: Arc::new(ExecutableFile {
                identity,
                _pin: pin,
            }),
        }
    }
    pub(crate) fn same_file(&self, other: &Self) -> bool {
        self.inner.identity == other.inner.identity
    }
}

/// Only OS package evidence currently establishes normalized application
/// membership. Executable pins intentionally cannot enter this projection.
pub(crate) fn verified_application(package_family: Option<&str>) -> Option<String> {
    package_family.map(|family| format!("windows:package-family:{}", family.to_ascii_lowercase()))
}

pub(crate) fn project(volume_path: &str, serial: u64, file: [u8; 16]) -> Option<String> {
    // A real local volume GUID is required. UNC, DOS drive letters, NT device
    // paths and unsupported providers provide no fallback identity.
    let guid = volume_path.strip_prefix(r"\\?\Volume{")?.get(..36)?;
    let suffix = volume_path.get(r"\\?\Volume{".len() + 36..)?;
    if !suffix.starts_with("}\\")
        || volume_path.contains('\0')
        || serial == 0
        || serial == u64::MAX
        || file == [0; 16]
        || file == [u8::MAX; 16]
    {
        return None;
    }
    for (index, byte) in guid.bytes().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if byte != b'-' {
                return None;
            }
        } else if !byte.is_ascii_hexdigit() {
            return None;
        }
    }
    let file = u128::from_be_bytes(file);
    Some(format!(
        "windows:executable-file:{}:{serial:016x}:{file:032x}",
        guid.to_ascii_lowercase()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    const PATH: &str = r"\\?\Volume{01234567-89AB-CDEF-0123-456789ABCDEF}\apps\example.exe";

    #[test]
    fn shared_runtime_file_equality_never_establishes_application_membership() {
        // Distinct script applications can share exactly the same verified image.
        // Only executable equality is available; there is no catalog/runtime rule
        // by which this evidence can be promoted to an application lease.
        let runtime = project(PATH, 7, [1; 16]).unwrap();
        let script_a = RetainedExecutable::new(runtime.clone(), ());
        let script_b = RetainedExecutable::new(runtime, ());
        assert!(script_a.same_file(&script_b));
        assert_eq!(verified_application(None), None);
        assert_eq!(
            verified_application(Some("Example.Package")),
            Some("windows:package-family:example.package".into())
        );
    }

    #[test]
    fn lease_clone_retains_pin_and_retired_identity_cannot_be_resurrected() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Pin(Arc<AtomicUsize>);
        impl Drop for Pin {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let retired = Arc::new(AtomicUsize::new(0));
        let id = project(PATH, 7, [1; 16]).unwrap();
        let process = RetainedExecutable::new(id.clone(), Pin(retired.clone()));
        let old_lifetime = Arc::downgrade(&process.inner);
        let lease = process.clone();
        drop(process);
        assert_eq!(retired.load(Ordering::SeqCst), 0);
        assert!(old_lifetime.upgrade().is_some());
        drop(lease);
        assert_eq!(retired.load(Ordering::SeqCst), 1);
        assert!(old_lifetime.upgrade().is_none());
        // A filesystem may reuse the same numbers after every pin is released.
        // A fresh evidence object cannot resurrect the retired object's lifetime.
        let replacement = RetainedExecutable::new(id, Pin(retired.clone()));
        assert!(old_lifetime.upgrade().is_none());
        drop(replacement);
        assert_eq!(retired.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn same_file_aliases_share_identity_but_volume_and_file_changes_do_not() {
        let first = project(PATH, 7, [1; 16]).unwrap();
        assert_eq!(
            first,
            project(
                &PATH
                    .replace("apps\\example.exe", "renamed\\alias.exe")
                    .to_ascii_lowercase()
                    .replace("volume{", "Volume{"),
                7,
                [1; 16]
            )
            .unwrap()
        );
        assert_ne!(Some(first.clone()), project(PATH, 8, [1; 16]));
        assert_ne!(Some(first.clone()), project(PATH, 7, [2; 16]));
        assert_ne!(
            Some(first.clone()),
            project(&PATH.replace("01234567", "11234567"), 7, [1; 16])
        );
        assert!(!first.contains("example"));
        assert!(first.len() < 128);
    }

    #[test]
    fn unsupported_and_ambiguous_file_identifiers_are_unavailable() {
        for serial in [0, u64::MAX] {
            assert_eq!(project(PATH, serial, [1; 16]), None);
        }
        for file in [[0; 16], [u8::MAX; 16]] {
            assert_eq!(project(PATH, 7, file), None);
        }
        for path in [
            r"C:\apps\example.exe",
            r"\\server\share\example.exe",
            r"\Device\HarddiskVolume1\example.exe",
            r"\\?\Volume{bogus}\example.exe",
        ] {
            assert_eq!(project(path, 7, [1; 16]), None);
        }
    }

    #[test]
    fn malformed_volume_evidence_never_becomes_a_valid_application_id() {
        for path in [
            PATH.replace('-', "x"),
            PATH.replace("ABCDEF", "ABCDEZ"),
            PATH.replace("}\\", "}/"),
            format!("{PATH}\0"),
            PATH.replace("01234567", "é1234567"),
        ] {
            assert_eq!(project(&path, 7, [1; 16]), None);
        }
    }
}
