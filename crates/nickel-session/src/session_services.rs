//! Narrow, authorization-preserving adapters for system session actions.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SystemAction {
    Suspend,
    Reboot,
    PowerOff,
}

trait LoginManager {
    fn call(&mut self, method: &'static str, interactive: bool) -> Result<(), String>;
}

struct Logind;

impl LoginManager for Logind {
    fn call(&mut self, method: &'static str, interactive: bool) -> Result<(), String> {
        let connection = zbus::blocking::Connection::system().map_err(|error| error.to_string())?;
        let proxy = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .map_err(|error| error.to_string())?;
        proxy
            .call_method(method, &(interactive,))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn perform(action: SystemAction, manager: &mut impl LoginManager) -> Result<(), String> {
    let method = match action {
        SystemAction::Suspend => "Suspend",
        SystemAction::Reboot => "Reboot",
        SystemAction::PowerOff => "PowerOff",
    };
    // `true` preserves logind/Polkit as the authorization authority. Nickel
    // neither predicts authorization nor substitutes its own credential UI.
    manager.call(method, true)
}

pub(crate) fn request(action: SystemAction) {
    let _ = std::thread::Builder::new()
        .name(format!("nickel-{}", action.label()))
        .spawn(move || {
            if let Err(error) = perform(action, &mut Logind) {
                tracing::error!(action = action.label(), %error, "system session action failed");
            }
        });
}

impl SystemAction {
    fn label(self) -> &'static str {
        match self {
            Self::Suspend => "suspend",
            Self::Reboot => "reboot",
            Self::PowerOff => "power-off",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LoginManager, SystemAction, perform};

    #[derive(Default)]
    struct RecordingManager {
        calls: Vec<(&'static str, bool)>,
    }

    impl LoginManager for RecordingManager {
        fn call(&mut self, method: &'static str, interactive: bool) -> Result<(), String> {
            self.calls.push((method, interactive));
            Ok(())
        }
    }

    #[test]
    fn every_power_action_delegates_authorization_to_logind() {
        let mut manager = RecordingManager::default();
        for action in [
            SystemAction::Suspend,
            SystemAction::Reboot,
            SystemAction::PowerOff,
        ] {
            perform(action, &mut manager).unwrap();
        }
        assert_eq!(
            manager.calls,
            [("Suspend", true), ("Reboot", true), ("PowerOff", true)]
        );
    }
}
