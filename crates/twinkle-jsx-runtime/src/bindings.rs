//! Rust-owned, immutable initialization for bounded host extensions.
use crate::JsxRuntime;
use serde_json::Value;
pub type DataPublisher = fn(&mut JsxRuntime, &Value, &[&str]) -> Result<(), String>;
#[derive(Default)]
pub struct HostBindings {
    pub(crate) bootstrap: String,
    pub(crate) publisher: Option<DataPublisher>,
    pub(crate) projections: Vec<String>,
}
impl HostBindings {
    pub const ABI_VERSION: u32 = 1;
    pub fn new(
        abi: u32,
        bootstrap: &str,
        publisher: DataPublisher,
        projections: &[&str],
    ) -> Result<Self, String> {
        if abi != Self::ABI_VERSION {
            return Err("unsupported host binding ABI".into());
        }
        if bootstrap.len() > 256 * 1024 || projections.len() > 128 {
            return Err("host bindings exceed limit".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for name in projections {
            if name.is_empty() || name.len() > 64 || !names.insert(*name) {
                return Err("invalid or duplicate projection binding".into());
            }
        }
        Ok(Self {
            bootstrap: bootstrap.into(),
            publisher: Some(publisher),
            projections: projections.iter().map(|name| (*name).into()).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn publisher(_: &mut JsxRuntime, _: &Value, _: &[&str]) -> Result<(), String> {
        Ok(())
    }
    #[test]
    fn registration_rejects_unsupported_abi_duplicate_names_and_bounds() {
        assert!(HostBindings::new(2, "", publisher, &[]).is_err());
        assert!(HostBindings::new(1, "", publisher, &["same", "same"]).is_err());
        assert!(HostBindings::new(1, "", publisher, &[""]).is_err());
        assert!(HostBindings::new(1, &"x".repeat(256 * 1024 + 1), publisher, &[]).is_err());
        assert!(HostBindings::new(1, "", publisher, &["valid"]).is_ok());
    }
    #[test]
    fn checkpoint_registration_is_sealed_before_package_initialization() {
        assert!(
            JsxRuntime::new(
                "__twinkleRegisterCheckpointParticipant({capture:()=>null,restore:()=>{}})",
                None
            )
            .is_err()
        );
    }
}
