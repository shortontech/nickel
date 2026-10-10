//! Nickel-owned package composition and provider lifecycle integration.
//! Generic execution and native transaction identities remain in Twinkle.
pub use twinkle_jsx_runtime::*;
mod composition;
pub mod composition_runtime;
pub use composition::ComposedShellGraph;

pub mod settings;
pub use settings::{SettingsRuntimeExt, SettingsValueSnapshot};
#[cfg(test)]
mod plugins_page_tests;
#[cfg(test)]
mod preferences_page_tests;
#[cfg(test)]
mod wallpaper_page_tests;

pub mod capabilities;
pub use capabilities::{CapabilityRuntimeExt, create_module_runtime, create_runtime};
#[cfg(test)]
mod capability_tests;

#[cfg(test)]
mod package_contract_tests;

#[cfg(test)]
mod service_tests;

pub mod stores;
pub use stores::DomainStoreRuntimeExt;
#[cfg(test)]
mod store_tests;

mod module_bindings;
pub use module_bindings::NickelModuleBindings;
