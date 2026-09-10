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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolkitRejection {
    Failed,
    Unavailable,
    ExternalConflict,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolkitWriteError {
    NotAccepted(ToolkitRejection),
    Uncertain,
}
impl ToolkitWriteError {
    /// Use only for an error known to precede any native submission.
    pub fn not_accepted(_: impl std::fmt::Display) -> Self {
        Self::NotAccepted(ToolkitRejection::Failed)
    }
}

pub trait ToolkitScaleBackend {
    fn capabilities(&self) -> Vec<ToolkitCapability>;
    fn read(&self, family: ToolkitFamily) -> Result<String, String>;
    fn write(&self, family: ToolkitFamily, value: &str) -> Result<(), String>;
    fn write_checked(
        &self,
        family: ToolkitFamily,
        value: &str,
        _expected: &str,
    ) -> Result<(), ToolkitWriteError> {
        self.write(family, value)
            .map_err(|_| ToolkitWriteError::Uncertain)
    }
    fn writable(&self, _family: ToolkitFamily) -> Result<bool, String> {
        Ok(true)
    }
}

pub fn canonical_toolkit_value(family: ToolkitFamily, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() > 64 {
        return Err("toolkit value exceeds limit".into());
    }
    match family {
        ToolkitFamily::Gtk if value == "follow-nickel" => Ok("0".into()),
        ToolkitFamily::Gtk => value
            .strip_prefix("uint32 ")
            .unwrap_or(value)
            .parse::<u32>()
            .map(|value| value.to_string())
            .map_err(|_| "invalid GTK scale value".into()),
        ToolkitFamily::Qt => {
            if value.is_empty() || value == "follow-nickel" {
                return Ok("follow-nickel".into());
            }
            let value = value.parse::<f64>().map_err(|_| "invalid Qt scale value")?;
            if !value.is_finite() || !(0.0..=64.0).contains(&value) {
                return Err("invalid Qt scale value".into());
            }
            Ok(format!("{value:.6}"))
        }
    }
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
        }
    }

    pub fn prepare_read(&self, family: ToolkitFamily) -> Result<PreparedToolkitCommand, String> {
        let mut command = std::process::Command::new(match family {
            ToolkitFamily::Gtk => self.gsettings.as_ref().ok_or("GTK settings unavailable")?,
            ToolkitFamily::Qt => return Err("Qt reads use the typed config adapter".into()),
        });
        match family {
            ToolkitFamily::Gtk => {
                command.args(["get", "org.gnome.desktop.interface", "scaling-factor"]);
            }
            ToolkitFamily::Qt => {
                command.args([
                    "--file",
                    "kdeglobals",
                    "--group",
                    "KScreen",
                    "--key",
                    "ScaleFactor",
                ]);
            }
        }
        Ok(PreparedToolkitCommand(command))
    }

    pub fn prepare_writable(&self) -> Result<PreparedToolkitCommand, String> {
        let mut command =
            std::process::Command::new(self.gsettings.as_ref().ok_or("GTK settings unavailable")?);
        command.args(["writable", "org.gnome.desktop.interface", "scaling-factor"]);
        Ok(PreparedToolkitCommand(command))
    }
}
#[cfg(target_os = "linux")]
pub struct PreparedToolkitCommand(std::process::Command);
#[cfg(target_os = "linux")]
impl PreparedToolkitCommand {
    /// Acceptance boundary: callers must authorize immediately before spawning.
    pub fn spawn(mut self) -> Result<RunningToolkitCommand, String> {
        use std::os::fd::OwnedFd;
        // Configure the owned output channel before process acceptance. This
        // avoids any post-spawn setup failure requiring an owner-thread wait.
        let (reader, writer) =
            std::os::unix::net::UnixStream::pair().map_err(|_| "toolkit pipe unavailable")?;
        reader
            .set_nonblocking(true)
            .map_err(|_| "toolkit pipe unavailable")?;
        let child = self
            .0
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(OwnedFd::from(writer)))
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| "toolkit command unavailable")?;
        Ok(RunningToolkitCommand {
            child: Some(child),
            reader,
        })
    }
}
#[cfg(target_os = "linux")]
pub struct RunningToolkitCommand {
    child: Option<std::process::Child>,
    reader: std::os::unix::net::UnixStream,
}
#[cfg(target_os = "linux")]
impl RunningToolkitCommand {
    pub fn abort_into_child(mut self) -> std::process::Child {
        let mut child = self.child.take().unwrap();
        let _ = child.kill();
        child
    }
    /// Wait on a worker. Output is limited to 4 KiB and wall time to 750 ms. On
    /// cancellation, kill and reap before releasing caller's worker admission.
    pub fn wait(mut self, mut check: impl FnMut() -> Result<(), String>) -> Result<String, String> {
        use std::io::Read;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(750);
        let child = self.child.as_mut().unwrap();
        let mut bytes = Vec::new();
        let mut completed: Option<std::process::ExitStatus> = None;
        let result = (|| {
            loop {
                check()?;
                if std::time::Instant::now() >= deadline {
                    return Err("toolkit command timed out".into());
                }
                let mut buffer = [0u8; 512];
                loop {
                    match self.reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            if bytes.len() + count > 4096 {
                                return Err("toolkit response exceeds limit".into());
                            }
                            bytes.extend_from_slice(&buffer[..count]);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => return Err("toolkit response unavailable".into()),
                    }
                }
                if let Some(status) = completed {
                    if !status.success() {
                        return Err("toolkit command failed".into());
                    }
                    check()?;
                    return String::from_utf8(bytes)
                        .map(|value| value.trim().to_owned())
                        .map_err(|_| "invalid toolkit response".into());
                }
                completed = child
                    .try_wait()
                    .map_err(|_| "toolkit process unavailable")?;
                if completed.is_some() {
                    continue;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        })();
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child.take();
        result
    }
}
#[cfg(target_os = "linux")]
impl Drop for RunningToolkitCommand {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.try_wait();
        }
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
                available: true,
                live: false,
                restart_required: true,
            },
        ]
    }
    fn read(&self, family: ToolkitFamily) -> Result<String, String> {
        if family == ToolkitFamily::Qt {
            return crate::read_qt_scale();
        }
        canonical_toolkit_value(
            family,
            &self.prepare_read(family)?.spawn()?.wait(|| Ok(()))?,
        )
    }
    fn writable(&self, family: ToolkitFamily) -> Result<bool, String> {
        Ok(family != ToolkitFamily::Gtk
            || self.prepare_writable()?.spawn()?.wait(|| Ok(()))? == "true")
    }
    fn write(&self, family: ToolkitFamily, value: &str) -> Result<(), String> {
        let expected = self.read(family)?;
        self.write_checked(family, value, &expected)
            .map_err(|error| format!("{error:?}"))
    }
    fn write_checked(
        &self,
        family: ToolkitFamily,
        value: &str,
        expected: &str,
    ) -> Result<(), ToolkitWriteError> {
        let prepared = crate::PreparedToolkitWrite::prepare(family, value, expected)?;
        prepared.check_observed(&self.read(family).map_err(ToolkitWriteError::not_accepted)?)?;
        prepared.accept(|| Ok(()))?.wait(|| Ok(()))
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
    fn canonical_values_allow_typed_gtk_reset_and_reject_non_numeric_data() {
        assert_eq!(
            canonical_toolkit_value(ToolkitFamily::Gtk, "uint32 2").unwrap(),
            "2"
        );
        assert_eq!(
            canonical_toolkit_value(ToolkitFamily::Qt, "").unwrap(),
            "follow-nickel"
        );
        for value in ["NaN", "inf", "65", "arbitrary data"] {
            assert!(canonical_toolkit_value(ToolkitFamily::Qt, value).is_err());
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn native_command_runner_bounds_output_and_honors_cancellation() {
        let mut command = std::process::Command::new("/usr/bin/printf");
        command.arg("uint32 2");
        assert_eq!(
            PreparedToolkitCommand(command)
                .spawn()
                .unwrap()
                .wait(|| Ok(()))
                .unwrap(),
            "uint32 2"
        );
        let mut command = std::process::Command::new("/usr/bin/printf");
        command.arg("x".repeat(4097));
        assert!(
            PreparedToolkitCommand(command)
                .spawn()
                .unwrap()
                .wait(|| Ok(()))
                .unwrap_err()
                .contains("limit")
        );
        let mut command = std::process::Command::new("/usr/bin/sleep");
        command.arg("10");
        let running = PreparedToolkitCommand(command).spawn().unwrap();
        let pid = running.child.as_ref().unwrap().id();
        let start = std::time::Instant::now();
        assert_eq!(
            running.wait(|| Err("cancelled".into())).unwrap_err(),
            "cancelled"
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_command_timeout_reaps_the_owned_helper() {
        let mut command = std::process::Command::new("/usr/bin/sleep");
        command.arg("10");
        let running = PreparedToolkitCommand(command).spawn().unwrap();
        let pid = running.child.as_ref().unwrap().id();
        let start = std::time::Instant::now();
        assert!(running.wait(|| Ok(())).unwrap_err().contains("timed out"));
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
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
        if cfg!(target_os = "linux") {
            assert_eq!(env["QT_SCALE_FACTOR"], "1.250000");
            assert_eq!(env["GDK_SCALE"], "2");
            assert_eq!(env["GDK_DPI_SCALE"], "0.625000");
        } else {
            assert!(env.is_empty());
        }
    }
}
