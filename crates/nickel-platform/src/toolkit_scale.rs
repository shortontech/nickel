//! Optional Linux toolkit scale integrations. These are compatibility knobs,
//! deliberately separate from the compositor's per-output scale.

use nickel_core::dpi::{ApplicationScalePolicy, Scale120};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolkitFamily {
    Gtk,
    Qt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolkitCapability {
    pub family: ToolkitFamily,
    pub available: bool,
    pub live: bool,
    pub restart_required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolkitWrite {
    pub family: ToolkitFamily,
    pub previous: String,
    pub applied: String,
    pub restart_required: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolkitApplyReport {
    pub writes: Vec<ToolkitWrite>,
    pub failures: Vec<(ToolkitFamily, String)>,
}

pub trait ToolkitScaleBackend {
    fn capabilities(&self) -> Vec<ToolkitCapability>;
    fn read(&self, family: ToolkitFamily) -> Result<String, String>;
    fn write(&self, family: ToolkitFamily, value: &str) -> Result<(), String>;
}

fn policy_value(policy: ApplicationScalePolicy, family: ToolkitFamily) -> Option<String> {
    match policy {
        ApplicationScalePolicy::FollowNickel => Some("follow-nickel".into()),
        ApplicationScalePolicy::Unchanged => None,
        ApplicationScalePolicy::Custom(scale) => Some(match family {
            ToolkitFamily::Gtk => scale.integer_buffer_scale().to_string(),
            ToolkitFamily::Qt => format!("{:.6}", scale.factor()),
        }),
    }
}

pub fn apply_toolkit_scale(
    backend: &dyn ToolkitScaleBackend,
    policy: ApplicationScalePolicy,
) -> ToolkitApplyReport {
    let mut report = ToolkitApplyReport::default();
    for capability in backend
        .capabilities()
        .into_iter()
        .filter(|value| value.available)
    {
        let Some(value) = policy_value(policy, capability.family) else {
            continue;
        };
        let previous = match backend.read(capability.family) {
            Ok(previous) => previous,
            Err(error) => {
                report.failures.push((capability.family, error));
                continue;
            }
        };
        match backend.write(capability.family, &value) {
            Ok(()) => {
                let applied = backend.read(capability.family).unwrap_or(value);
                report.writes.push(ToolkitWrite {
                    family: capability.family,
                    previous,
                    applied,
                    restart_required: capability.restart_required,
                });
            }
            Err(error) => report.failures.push((capability.family, error)),
        }
    }
    report
}

pub fn reset_owned_toolkit_scale(
    backend: &dyn ToolkitScaleBackend,
    writes: &[ToolkitWrite],
) -> ToolkitApplyReport {
    let mut report = ToolkitApplyReport::default();
    for owned in writes {
        // Do not erase an external change made since Nickel's write.
        match backend.read(owned.family) {
            Ok(current) if current != owned.applied => continue,
            Err(error) => {
                report.failures.push((owned.family, error));
                continue;
            }
            Ok(_) => {}
        }
        match backend.write(owned.family, &owned.previous) {
            Ok(()) => report.writes.push(ToolkitWrite {
                family: owned.family,
                previous: owned.applied.clone(),
                applied: owned.previous.clone(),
                restart_required: owned.restart_required,
            }),
            Err(error) => report.failures.push((owned.family, error)),
        }
    }
    report
}

/// Launch-time fallback snapshot. Empty values mean remove an inherited
/// override, ensuring compositor and toolkit scales are never multiplied.
pub fn toolkit_launch_environment(
    policy: ApplicationScalePolicy,
) -> std::collections::BTreeMap<String, String> {
    nickel_core::dpi::ApplicationScaleSettings {
        policy,
        ..Default::default()
    }
    .launch_environment(cfg!(target_os = "linux"))
}

pub fn supported_custom_scales() -> Vec<Scale120> {
    (60..=480).step_by(30).filter_map(Scale120::new).collect()
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Default)]
pub struct LinuxToolkitScaleBackend {
    gsettings: Option<std::path::PathBuf>,
    kreadconfig: Option<std::path::PathBuf>,
    kwriteconfig: Option<std::path::PathBuf>,
}

#[cfg(target_os = "linux")]
fn find_program(names: &[&str]) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.is_file())
}

#[cfg(target_os = "linux")]
impl LinuxToolkitScaleBackend {
    pub fn detect() -> Self {
        Self {
            gsettings: find_program(&["gsettings"]),
            kreadconfig: find_program(&["kreadconfig6", "kreadconfig5"]),
            kwriteconfig: find_program(&["kwriteconfig6", "kwriteconfig5"]),
        }
    }

    fn output(command: &mut std::process::Command) -> Result<String, String> {
        let output = command.output().map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
}

#[cfg(target_os = "linux")]
impl ToolkitScaleBackend for LinuxToolkitScaleBackend {
    fn capabilities(&self) -> Vec<ToolkitCapability> {
        vec![
            ToolkitCapability {
                family: ToolkitFamily::Gtk,
                available: self.gsettings.is_some(),
                live: true,
                restart_required: true,
            },
            ToolkitCapability {
                family: ToolkitFamily::Qt,
                available: self.kreadconfig.is_some() && self.kwriteconfig.is_some(),
                live: false,
                restart_required: true,
            },
        ]
    }

    fn read(&self, family: ToolkitFamily) -> Result<String, String> {
        match family {
            ToolkitFamily::Gtk => Self::output(
                std::process::Command::new(
                    self.gsettings.as_ref().ok_or("GTK settings unavailable")?,
                )
                .args(["get", "org.gnome.desktop.interface", "scaling-factor"]),
            ),
            ToolkitFamily::Qt => Self::output(
                std::process::Command::new(
                    self.kreadconfig.as_ref().ok_or("Qt settings unavailable")?,
                )
                .args([
                    "--file",
                    "kdeglobals",
                    "--group",
                    "KScreen",
                    "--key",
                    "ScaleFactor",
                ]),
            ),
        }
    }

    fn write(&self, family: ToolkitFamily, value: &str) -> Result<(), String> {
        let status = match family {
            ToolkitFamily::Gtk => {
                let mut command = std::process::Command::new(
                    self.gsettings.as_ref().ok_or("GTK settings unavailable")?,
                );
                if value == "follow-nickel" {
                    command.args(["reset", "org.gnome.desktop.interface", "scaling-factor"]);
                } else {
                    command.args([
                        "set",
                        "org.gnome.desktop.interface",
                        "scaling-factor",
                        &format!("uint32 {value}"),
                    ]);
                }
                command.status()
            }
            ToolkitFamily::Qt => {
                let mut command = std::process::Command::new(
                    self.kwriteconfig
                        .as_ref()
                        .ok_or("Qt settings unavailable")?,
                );
                command.args([
                    "--file",
                    "kdeglobals",
                    "--group",
                    "KScreen",
                    "--key",
                    "ScaleFactor",
                ]);
                if value == "follow-nickel" {
                    command.arg("--delete");
                } else {
                    command.arg(value);
                }
                command.status()
            }
        }
        .map_err(|error| error.to_string())?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| "toolkit settings command failed".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::BTreeMap};

    struct Fake {
        caps: Vec<ToolkitCapability>,
        values: RefCell<BTreeMap<u8, String>>,
        fail: Option<ToolkitFamily>,
    }
    fn key(family: ToolkitFamily) -> u8 {
        match family {
            ToolkitFamily::Gtk => 0,
            ToolkitFamily::Qt => 1,
        }
    }
    impl ToolkitScaleBackend for Fake {
        fn capabilities(&self) -> Vec<ToolkitCapability> {
            self.caps.clone()
        }
        fn read(&self, family: ToolkitFamily) -> Result<String, String> {
            self.values
                .borrow()
                .get(&key(family))
                .cloned()
                .ok_or_else(|| "read failed".into())
        }
        fn write(&self, family: ToolkitFamily, value: &str) -> Result<(), String> {
            if self.fail == Some(family) {
                return Err("permission denied".into());
            }
            self.values.borrow_mut().insert(key(family), value.into());
            Ok(())
        }
    }
    fn capability(family: ToolkitFamily) -> ToolkitCapability {
        ToolkitCapability {
            family,
            available: true,
            live: family == ToolkitFamily::Gtk,
            restart_required: family == ToolkitFamily::Qt,
        }
    }

    #[test]
    fn absent_gtk_only_qt_only_and_both_are_independent() {
        for caps in [
            vec![],
            vec![capability(ToolkitFamily::Gtk)],
            vec![capability(ToolkitFamily::Qt)],
            vec![
                capability(ToolkitFamily::Gtk),
                capability(ToolkitFamily::Qt),
            ],
        ] {
            let fake = Fake {
                caps: caps.clone(),
                values: RefCell::new(BTreeMap::from([(0, "1".into()), (1, "1.0".into())])),
                fail: None,
            };
            let report = apply_toolkit_scale(
                &fake,
                ApplicationScalePolicy::Custom(Scale120::new(150).unwrap()),
            );
            assert_eq!(report.writes.len(), caps.len());
            assert!(report.failures.is_empty());
        }
    }

    #[test]
    fn partial_failure_and_external_change_are_reported_and_preserved() {
        let fake = Fake {
            caps: vec![
                capability(ToolkitFamily::Gtk),
                capability(ToolkitFamily::Qt),
            ],
            values: RefCell::new(BTreeMap::from([(0, "1".into()), (1, "1.0".into())])),
            fail: Some(ToolkitFamily::Qt),
        };
        let report = apply_toolkit_scale(&fake, ApplicationScalePolicy::FollowNickel);
        assert_eq!(report.writes.len(), 1);
        assert_eq!(
            report.failures,
            vec![(ToolkitFamily::Qt, "permission denied".into())]
        );
        fake.values.borrow_mut().insert(0, "external".into());
        assert!(
            reset_owned_toolkit_scale(&fake, &report.writes)
                .writes
                .is_empty()
        );
    }

    #[test]
    fn unchanged_policy_never_writes_and_custom_environment_does_not_double_scale() {
        let fake = Fake {
            caps: vec![capability(ToolkitFamily::Gtk)],
            values: RefCell::new(BTreeMap::from([(0, "1".into())])),
            fail: None,
        };
        assert_eq!(
            apply_toolkit_scale(&fake, ApplicationScalePolicy::Unchanged),
            ToolkitApplyReport::default()
        );
        let env =
            toolkit_launch_environment(ApplicationScalePolicy::Custom(Scale120::new(150).unwrap()));
        assert_eq!(env["QT_SCALE_FACTOR"], "1.250000");
        assert_eq!(env["GDK_SCALE"], "2");
        assert_eq!(env["GDK_DPI_SCALE"], "0.625000");
    }
}
