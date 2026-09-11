//! Subscription admission retains only authenticated opaque client identities.
use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
};
pub const MAX_SUBSCRIPTIONS: usize = 4;
pub const MAX_SUBSCRIPTION_SECONDS: u64 = 60;
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());
fn clients() -> &'static Mutex<HashSet<String>> {
    static CLIENTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CLIENTS.get_or_init(Default::default)
}
pub(crate) struct SubscriptionAdmission {
    client: String,
}
impl SubscriptionAdmission {
    pub fn acquire(client: &str) -> Result<Self, String> {
        let mut active = clients()
            .lock()
            .map_err(|_| "event subscriptions unavailable")?;
        if active.len() >= MAX_SUBSCRIPTIONS || active.contains(client) {
            return Err("event subscription capacity reached".into());
        }
        active.insert(client.to_owned());
        Ok(Self {
            client: client.to_owned(),
        })
    }
}
impl Drop for SubscriptionAdmission {
    fn drop(&mut self) {
        if let Ok(mut active) = clients().lock() {
            active.remove(&self.client);
        }
    }
}
pub(crate) fn resource_lease(uri: &str) -> Result<u64, String> {
    let id: u64 = uri
        .strip_prefix("nickel://desktop-events/")
        .and_then(|value| value.parse().ok())
        .filter(|id| *id != 0)
        .ok_or_else(|| "invalid desktop event resource".to_owned())?;
    if uri != format!("nickel://desktop-events/{id}") {
        return Err("invalid desktop event resource".into());
    }
    Ok(id)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subscription_capacity_is_global_per_client_and_released_on_drop() {
        let _test_guard = TEST_LOCK.lock().unwrap();
        let mut guards = Vec::new();
        for id in 0..MAX_SUBSCRIPTIONS {
            guards.push(SubscriptionAdmission::acquire(&format!("fixture-{id}")).unwrap());
        }
        assert!(SubscriptionAdmission::acquire("fixture-0").is_err());
        assert!(SubscriptionAdmission::acquire("fixture-extra").is_err());
        guards.pop();
        assert!(SubscriptionAdmission::acquire("fixture-extra").is_ok());
    }
    #[test]
    fn event_resource_identity_has_one_canonical_spelling() {
        assert_eq!(resource_lease("nickel://desktop-events/42").unwrap(), 42);
        for uri in [
            "nickel://desktop-events/0",
            "nickel://desktop-events/042",
            "nickel://desktop-events/+42",
            "nickel://desktop-events/42?x=1",
            "nickel://desktop-events/42/",
            "https://desktop-events/42",
        ] {
            assert!(resource_lease(uri).is_err());
        }
    }
}
