//! Fixed GTK/Qt setter endpoints. Preparation and completion run on admitted
//! workers; the final rename and each bounded native D-Bus write run under authority.
use crate::{ToolkitFamily, ToolkitRejection, ToolkitWriteError, canonical_toolkit_value};
use futures_lite::Stream;
use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
const LIMIT: usize = 65536;
fn read_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    nickel_storage::read_regular_file(path, LIMIT)
        .map_err(|_| "toolkit config is unavailable or not a bounded regular file".into())
}

fn config_root() -> Result<PathBuf, String> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .ok_or_else(|| "config home unavailable".into())
}
pub fn read_qt_scale() -> Result<String, String> {
    let path = config_root()?.join("kdeglobals");
    let bytes = read_file(&path)?.unwrap_or_default();
    read_key(&bytes, ToolkitFamily::Qt)
}
fn read_key(bytes: &[u8], family: ToolkitFamily) -> Result<String, String> {
    let (group, target, default) = match family {
        ToolkitFamily::Qt => ("KScreen", "ScaleFactor", "follow-nickel"),
        ToolkitFamily::Gtk => ("org/gnome/desktop/interface", "scaling-factor", "0"),
    };
    let text = std::str::from_utf8(bytes).map_err(|_| "invalid toolkit config")?;
    let mut inside = false;
    let mut groups = 0;
    let mut value = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == format!("[{group}]");
            if line.starts_with(&format!("[{group}]")) && !inside {
                return Err("unsupported toolkit group flags".into());
            }
            if inside {
                groups += 1;
                if groups > 1 {
                    return Err("duplicate toolkit group".into());
                }
            }
        }
        if inside && let Some((key, contents)) = line.split_once('=') {
            if key.trim().starts_with(&format!("{target}[")) {
                return Err("unsupported toolkit key flags".into());
            }
            if key.trim() == target {
                if value.is_some() {
                    return Err("duplicate toolkit key".into());
                }
                value = Some(canonical_toolkit_value(family, contents)?);
            }
        }
    }
    Ok(value.unwrap_or_else(|| default.into()))
}

/// Preserve every unrelated line byte-for-byte; ambiguous duplicate/immutable
/// target groups or keys are rejected rather than guessed or broadly rewritten.
fn replace_key(
    bytes: &[u8],
    group: &str,
    key: &str,
    value: Option<&str>,
) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "invalid toolkit config")?;
    let mut output = String::new();
    let mut inside = false;
    let mut groups = 0;
    let mut keys = 0;
    let mut inserted = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == "[$i]" {
            return Err("toolkit file is immutable".into());
        }
        if trimmed.starts_with('[') {
            if inside && !inserted {
                if let Some(value) = value {
                    output.push_str(&format!("{key}={value}\n"));
                }
                inserted = true;
            }
            inside = trimmed == format!("[{group}]");
            if trimmed.starts_with(&format!("[{group}]")) && !inside {
                return Err("toolkit group has unsupported flags".into());
            }
            if inside {
                groups += 1;
                if groups > 1 {
                    return Err("duplicate toolkit group".into());
                }
            }
        }
        if inside
            && trimmed.split_once('=').is_some_and(|(left, _)| {
                left.trim() == key || left.trim().starts_with(&format!("{key}["))
            })
        {
            if trimmed.split_once('=').unwrap().0.trim() != key {
                return Err("toolkit key has unsupported flags".into());
            }
            keys += 1;
            if keys > 1 {
                return Err("duplicate toolkit key".into());
            }
            if let Some(value) = value {
                output.push_str(&format!("{key}={value}\n"));
            }
            inserted = true;
        } else {
            output.push_str(line);
        }
    }
    if !inserted && let Some(value) = value {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        if groups == 0 {
            output.push_str(&format!("[{group}]\n"));
        }
        output.push_str(&format!("{key}={value}\n"));
    }
    if output.len() > LIMIT {
        return Err("toolkit config exceeds limit".into());
    }
    Ok(output.into_bytes())
}
pub struct PreparedToolkitWrite {
    family: ToolkitFamily,
    expected: String,
    write: NativeWrite,
}
pub enum NativeWrite {
    File {
        path: PathBuf,
        previous: Option<Vec<u8>>,
        staged: nickel_storage::StagedDurableWrite,
    },
    Dconf {
        sender: crate::bounded_dbus::GuardedSender,
        connection: zbus::blocking::Connection,
        message: zbus::Message,
        stream: Box<zbus::MessageStream>,
    },
}
pub enum ToolkitWriteCompletion {
    File(nickel_storage::DirectorySyncReceipt),
    Dconf {
        connection: zbus::blocking::Connection,
        serial: std::num::NonZeroU32,
        stream: Box<zbus::MessageStream>,
    },
}
fn dconf_database() -> Result<String, String> {
    let profile = std::env::var_os("DCONF_PROFILE");
    let path = profile
        .map(PathBuf::from)
        .map(|p| {
            if p.is_absolute() {
                p
            } else {
                Path::new("/etc/dconf/profile").join(p)
            }
        })
        .unwrap_or_else(|| PathBuf::from("/etc/dconf/profile/user"));
    let Some(bytes) = read_file(&path)? else {
        return Ok("user".into());
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| "invalid dconf profile")?;
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or("empty dconf profile")?;
    let name = first
        .strip_prefix("user-db:")
        .ok_or("dconf profile has no writable user database")?;
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || v == b'_' || v == b'-')
    {
        return Err("unsupported dconf database".into());
    }
    Ok(name.into())
}
#[allow(deprecated)]
fn dconf_blob(value: u32) -> Result<Vec<u8>, String> {
    // GNOME dconf Changeset wire representation is GVariant a{smv}.
    let values = BTreeMap::from([(
        "/org/gnome/desktop/interface/scaling-factor",
        Some(zvariant::Value::U32(value)),
    )]);
    zvariant::to_bytes(
        zvariant::serialized::Context::new_gvariant(zvariant::LE, 0),
        &values,
    )
    .map(|data| data.to_vec())
    .map_err(|_| "dconf encoding unavailable".into())
}
impl PreparedToolkitWrite {
    pub fn prepare(
        family: ToolkitFamily,
        value: &str,
        expected: &str,
    ) -> Result<Self, ToolkitWriteError> {
        let expected =
            canonical_toolkit_value(family, expected).map_err(ToolkitWriteError::not_accepted)?;
        let value =
            canonical_toolkit_value(family, value).map_err(ToolkitWriteError::not_accepted)?;
        let backend = std::env::var("GSETTINGS_BACKEND").unwrap_or_else(|_| "dconf".into());
        let file = match family {
            ToolkitFamily::Qt => Some((
                config_root()
                    .map_err(ToolkitWriteError::not_accepted)?
                    .join("kdeglobals"),
                "KScreen",
                "ScaleFactor",
                if value == "follow-nickel" {
                    None
                } else {
                    Some(value.as_str())
                },
            )),
            ToolkitFamily::Gtk if backend == "keyfile" => Some((
                config_root()
                    .map_err(ToolkitWriteError::not_accepted)?
                    .join("glib-2.0/settings/keyfile"),
                "org/gnome/desktop/interface",
                "scaling-factor",
                Some(value.as_str()),
            )),
            ToolkitFamily::Gtk if backend == "dconf" => None,
            _ => {
                return Err(ToolkitWriteError::NotAccepted(
                    ToolkitRejection::Unavailable,
                ));
            }
        };
        if let Some((path, group, key, value)) = file {
            return Self::prepare_file(path, family, expected, group, key, value);
        }
        let write = Self::prepare_dconf(&value).map_err(ToolkitWriteError::not_accepted)?;
        Ok(Self {
            family,
            expected,
            write,
        })
    }
    fn prepare_dconf(value: &str) -> Result<NativeWrite, String> {
        let database = dconf_database()?;
        let (connection, sender) = crate::bounded_dbus::connect_guarded_blocking(
            zbus::Address::session().map_err(|_| "session bus unavailable")?,
            crate::bounded_dbus::Limits::CONTROL,
            Duration::from_millis(300),
        )?;
        let stream = Box::new(zbus::MessageStream::from(connection.inner()));
        let blob = dconf_blob(value.parse().map_err(|_| "invalid GTK value")?)?;
        let message =
            zbus::Message::method_call(format!("/ca/desrt/dconf/Writer/{database}"), "Change")
                .map_err(|_| "dconf message unavailable")?
                .destination("ca.desrt.dconf")
                .map_err(|_| "dconf message unavailable")?
                .interface("ca.desrt.dconf.Writer")
                .map_err(|_| "dconf message unavailable")?
                .build(&(blob,))
                .map_err(|_| "dconf message unavailable")?;
        if message.data().len() > 8192 {
            return Err("dconf message exceeds limit".into());
        }
        Ok(NativeWrite::Dconf {
            sender,
            connection,
            message,
            stream,
        })
    }
    pub(crate) fn prepare_file(
        path: PathBuf,
        family: ToolkitFamily,
        expected: String,
        group: &str,
        key: &str,
        value: Option<&str>,
    ) -> Result<Self, ToolkitWriteError> {
        let previous = read_file(&path).map_err(ToolkitWriteError::not_accepted)?;
        if read_key(previous.as_deref().unwrap_or_default(), family)
            .map_err(ToolkitWriteError::not_accepted)?
            != expected
        {
            return Err(ToolkitWriteError::NotAccepted(
                ToolkitRejection::ExternalConflict,
            ));
        }
        let bytes = replace_key(previous.as_deref().unwrap_or_default(), group, key, value)
            .map_err(ToolkitWriteError::not_accepted)?;
        let staged = nickel_storage::stage_durable_write(&path, bytes)
            .map_err(ToolkitWriteError::not_accepted)?;
        Ok(Self {
            family,
            expected,
            write: NativeWrite::File {
                path,
                previous,
                staged,
            },
        })
    }
    pub fn check_observed(&self, value: &str) -> Result<(), ToolkitWriteError> {
        if canonical_toolkit_value(self.family, value).map_err(ToolkitWriteError::not_accepted)?
            != self.expected
        {
            return Err(ToolkitWriteError::NotAccepted(
                ToolkitRejection::ExternalConflict,
            ));
        }
        Ok(())
    }
    /// Caller holds continuous authority. Pending send is never resumed later.
    pub fn accept(
        self,
        check: impl FnMut() -> Result<(), String>,
    ) -> Result<ToolkitWriteCompletion, ToolkitWriteError> {
        self.write.accept(check)
    }
}
impl NativeWrite {
    pub fn accept(
        self,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<ToolkitWriteCompletion, ToolkitWriteError> {
        match self {
            Self::File {
                path,
                previous,
                staged,
            } => {
                let mut reason = ToolkitRejection::Failed;
                staged
                    .commit(|| {
                        if read_file(&path).map_err(std::io::Error::other)? != previous {
                            reason = ToolkitRejection::ExternalConflict;
                            return Err(std::io::Error::other("toolkit config changed"));
                        }
                        check().map_err(std::io::Error::other)
                    })
                    .map(ToolkitWriteCompletion::File)
                    .map_err(|_| ToolkitWriteError::NotAccepted(reason))
            }
            Self::Dconf {
                sender,
                connection,
                message,
                stream,
            } => {
                sender
                    .send_guarded(&message, check)
                    .map_err(|error| match error {
                        crate::bounded_dbus::GuardedSendError::NotAccepted => {
                            ToolkitWriteError::NotAccepted(ToolkitRejection::Failed)
                        }
                        crate::bounded_dbus::GuardedSendError::Uncertain => {
                            ToolkitWriteError::Uncertain
                        }
                    })?;
                Ok(ToolkitWriteCompletion::Dconf {
                    connection,
                    serial: message.primary_header().serial_num(),
                    stream,
                })
            }
        }
    }
}
impl ToolkitWriteCompletion {
    /// Successful DConf method reply follows service commit; value equality alone
    /// is never used as proof that a previously uncertain setter is terminal.
    pub fn wait(self, check: impl FnMut() -> Result<(), String>) -> Result<(), ToolkitWriteError> {
        self.wait_inner(check)
            .map_err(|_| ToolkitWriteError::Uncertain)
    }
    fn wait_inner(self, mut check: impl FnMut() -> Result<(), String>) -> Result<(), String> {
        match self {
            Self::File(receipt) => {
                receipt
                    .sync()
                    .map_err(|_| "toolkit persistence uncertain")?;
                check()
            }
            Self::Dconf {
                connection,
                serial,
                mut stream,
            } => {
                let _connection = connection;
                let deadline = Instant::now() + Duration::from_millis(750);
                let mut count = 0;
                loop {
                    check()?;
                    if Instant::now() >= deadline {
                        return Err("dconf completion uncertain".into());
                    }
                    match Pin::new(stream.as_mut())
                        .poll_next(&mut Context::from_waker(Waker::noop()))
                    {
                        Poll::Ready(Some(Ok(message))) => {
                            count += 1;
                            if count > 16 {
                                return Err("dconf response limit".into());
                            }
                            if message.header().reply_serial() == Some(serial) {
                                if message.message_type() != zbus::message::Type::MethodReturn {
                                    return Err("dconf service rejected change".into());
                                }
                                check()?;
                                let owner_reply = {
                                    let body = ("ca.desrt.dconf",);
                                    let mut query =
                                        std::pin::pin!(_connection.inner().call_method(
                                            Some("org.freedesktop.DBus"),
                                            "/org/freedesktop/DBus",
                                            Some("org.freedesktop.DBus"),
                                            "GetNameOwner",
                                            &body
                                        ));
                                    loop {
                                        check()?;
                                        if Instant::now() >= deadline {
                                            return Err("dconf completion uncertain".into());
                                        }
                                        match query
                                            .as_mut()
                                            .poll(&mut Context::from_waker(Waker::noop()))
                                        {
                                            Poll::Ready(result) => {
                                                break result
                                                    .map_err(|_| "dconf owner unavailable")?;
                                            }
                                            Poll::Pending => {
                                                std::thread::sleep(Duration::from_millis(5))
                                            }
                                        }
                                    }
                                };
                                if owner_reply.header().sender().map(|sender| sender.as_str())
                                    != Some("org.freedesktop.DBus")
                                {
                                    return Err("dconf owner reply is not from the bus".into());
                                }
                                if owner_reply.body().len() > 512 || message.body().len() > 512 {
                                    return Err("dconf reply exceeds bounds".into());
                                }
                                let owner: String = owner_reply
                                    .body()
                                    .deserialize()
                                    .map_err(|_| "invalid dconf owner")?;
                                if message.header().sender().map(|sender| sender.as_str())
                                    != Some(owner.as_str())
                                {
                                    return Err("dconf completion sender changed".into());
                                }
                                let tag: String = message
                                    .body()
                                    .deserialize()
                                    .map_err(|_| "invalid dconf completion")?;
                                if tag.len() > 256 {
                                    return Err("dconf tag exceeds limit".into());
                                }
                                return check();
                            }
                        }
                        Poll::Ready(_) => return Err("dconf completion unavailable".into()),
                        Poll::Pending => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn key_edit_preserves_unrelated_configuration_and_rejects_ambiguous_keys() {
        let before = b"# note\n[Other]\nKeep=hello\n[KScreen]\nScaleFactor=1.25\nOther=stay\n";
        assert_eq!(
            replace_key(before, "KScreen", "ScaleFactor", Some("1.5")).unwrap(),
            b"# note\n[Other]\nKeep=hello\n[KScreen]\nScaleFactor=1.5\nOther=stay\n"
        );
        assert!(
            replace_key(
                b"[KScreen]\nScaleFactor=1\nScaleFactor=2\n",
                "KScreen",
                "ScaleFactor",
                Some("3")
            )
            .is_err()
        );
        assert!(
            replace_key(
                b"[KScreen][$i]\nScaleFactor=1\n",
                "KScreen",
                "ScaleFactor",
                None
            )
            .is_err()
        );
    }
    fn file_write(path: &Path) -> NativeWrite {
        NativeWrite::File {
            path: path.into(),
            previous: read_file(path).unwrap(),
            staged: nickel_storage::stage_durable_write(path, b"replacement").unwrap(),
        }
    }
    #[test]
    fn direct_setter_checks_authority_and_fresh_contents_at_actual_rename() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("kdeglobals");
        fs::write(&path, "original").unwrap();
        assert!(file_write(&path).accept(|| Err("revoked".into())).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
        let pending = file_write(&path);
        fs::write(&path, "external").unwrap();
        assert!(pending.accept(|| Ok(())).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
        file_write(&path)
            .accept(|| Ok(()))
            .unwrap()
            .wait(|| Ok(()))
            .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "replacement");
    }
    #[test]
    fn owner_rename_recheck_rejects_fifo_without_waiting_for_a_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config");
        fs::write(&path, "original").unwrap();
        let prepared = file_write(&path);
        fs::remove_file(&path).unwrap();
        assert!(
            std::process::Command::new("/usr/bin/mkfifo")
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(prepared.accept(|| Ok(())).is_err());
        });
        assert!(
            rx.recv_timeout(Duration::from_secs(2))
                .expect("FIFO recheck blocked")
        );
    }

    #[allow(deprecated)]
    #[test]
    fn dconf_payload_contains_only_fixed_typed_scaling_key() {
        let bytes = dconf_blob(2).unwrap();
        let data = zvariant::serialized::Data::new(
            bytes,
            zvariant::serialized::Context::new_gvariant(zvariant::LE, 0),
        );
        let (values, _): (BTreeMap<String, Option<zvariant::OwnedValue>>, usize) =
            data.deserialize().unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(
            u32::try_from(values.into_values().next().unwrap().unwrap()).unwrap(),
            2
        );
    }
}
