//! Independent native component fixtures, shared by ordinary previews and downstream workbenches.
mod fixture_inventory;
pub use fixture_inventory::register;

pub fn registry()
-> Result<Vec<twinkle_testkit::FixtureRegistryEntry>, twinkle_testkit::RegistryError> {
    let mut registry = twinkle_testkit::FixtureRegistry::new();
    register(&mut registry)?;
    Ok(registry.finish())
}

#[cfg(feature = "jsx")]
pub mod jsx_preview;
