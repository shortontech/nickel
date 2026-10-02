//! Reusable account/session capability policy, independent of presentation.
use nickel_core::plugins::PluginCapability;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Account {
    pub display_name: String,
    pub username: String,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Support {
    pub lock: bool,
    pub logout: bool,
    pub suspend: bool,
    pub reboot: bool,
    pub power_off: bool,
    pub restart_shell: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Snapshot {
    pub revision: String,
    pub account: Option<Account>,
    pub locked: bool,
    pub support: Support,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub revision: String,
    pub action: Action,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Lock,
    Logout,
    Suspend,
    Reboot,
    PowerOff,
    RestartShell,
}
impl Action {
    pub fn native(self) -> crate::platform::SessionAction {
        use crate::platform::SessionAction as Native;
        match self {
            Self::Lock => Native::Lock,
            Self::Logout => Native::LogOut,
            Self::Suspend => Native::Suspend,
            Self::Reboot => Native::Reboot,
            Self::PowerOff => Native::PowerOff,
            Self::RestartShell => Native::RestartShell,
        }
    }
}
impl Snapshot {
    pub fn validate(
        &self,
        request: &Request,
        grants: &[PluginCapability],
    ) -> Result<Action, String> {
        let granted = grants.contains(&PluginCapability::SessionControl)
            || (request.action == Action::Logout
                && grants.contains(&PluginCapability::SessionLogoutRequest));
        if !granted {
            return Err("session operation grant is unavailable".into());
        }
        if request.revision != self.revision {
            return Err("session snapshot is stale".into());
        }
        if self.locked {
            return Err("session operations are unavailable while locked".into());
        }
        let supported = match request.action {
            Action::Lock => self.support.lock,
            Action::Logout => self.support.logout,
            Action::Suspend => self.support.suspend,
            Action::Reboot => self.support.reboot,
            Action::PowerOff => self.support.power_off,
            Action::RestartShell => self.support.restart_shell,
        };
        if !supported {
            return Err("session operation is unsupported".into());
        }
        Ok(request.action)
    }
}
pub(crate) fn snapshot(locked: bool, grants: &[PluginCapability]) -> Snapshot {
    #[cfg(target_os = "linux")]
    let native = {
        let [suspend, reboot, power_off] = crate::session::session_services::power_support();
        Support {
            lock: true,
            logout: true,
            suspend,
            reboot,
            power_off,
            restart_shell: false,
        }
    };
    #[cfg(target_os = "windows")]
    let native = Support {
        lock: true,
        ..Support::default()
    };
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let native = Support::default();
    snapshot_with(
        locked,
        grants,
        native,
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .ok(),
    )
}
fn snapshot_with(
    locked: bool,
    grants: &[PluginCapability],
    native: Support,
    username: Option<String>,
) -> Snapshot {
    use std::hash::{Hash, Hasher};
    let control = grants.contains(&PluginCapability::SessionControl) && !locked;
    let logout = (control || grants.contains(&PluginCapability::SessionLogoutRequest)) && !locked;
    let username = username
        .filter(|name| !name.trim().is_empty())
        .map(|name| name.chars().take(120).collect::<String>());
    let mut snapshot = Snapshot {
        revision: String::new(),
        account: username.map(|username| Account {
            display_name: username.clone(),
            username,
        }),
        locked,
        support: Support {
            lock: control && native.lock,
            logout: logout && native.logout,
            suspend: control && native.suspend,
            reboot: control && native.reboot,
            power_off: control && native.power_off,
            restart_shell: control && native.restart_shell,
        },
    };
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&snapshot)
        .expect("serializable session snapshot")
        .hash(&mut hash);
    snapshot.revision = format!("{:016x}", hash.finish());
    snapshot
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_operations_recheck_grants_revision_lock_and_native_support() {
        let grants = [PluginCapability::SessionControl];
        let native = Support {
            lock: true,
            logout: true,
            suspend: true,
            ..Support::default()
        };
        let current = snapshot_with(false, &grants, native, Some("user".into()));
        let mut request = Request {
            revision: current.revision.clone(),
            action: Action::Suspend,
        };
        assert_eq!(current.validate(&request, &grants), Ok(Action::Suspend));
        assert!(current.validate(&request, &[]).is_err());
        request.revision = "stale".into();
        assert!(current.validate(&request, &grants).is_err());
        request.revision = current.revision.clone();
        request.action = Action::RestartShell;
        assert!(current.validate(&request, &grants).is_err());
        let locked = snapshot_with(true, &grants, native, None);
        request.revision = locked.revision.clone();
        request.action = Action::Lock;
        assert!(locked.validate(&request, &grants).is_err());
        let logout = snapshot_with(
            false,
            &[PluginCapability::SessionLogoutRequest],
            native,
            None,
        );
        assert!(logout.support.logout);
        assert!(!logout.support.lock);
        assert!(!logout.support.suspend);
    }
}
