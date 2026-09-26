//! Narrow installer helper for Nickel's per-user Windows shell selection.

use std::path::Path;

#[cfg(target_os = "windows")]
const SHELL_VALUE: &str = "Shell";
#[cfg(target_os = "windows")]
const WINLOGON_KEY: &str = "Software\\Microsoft\\Windows NT\\CurrentVersion\\Winlogon";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShellMutation {
    Set(String),
    Delete,
    None,
}

pub fn enable_mutation(executable: &Path) -> Result<ShellMutation, String> {
    validate_executable(executable)?;
    Ok(ShellMutation::Set(shell_command(executable)))
}

pub fn disable_mutation(
    current_shell: Option<&str>,
    installed_executable: &Path,
) -> Result<ShellMutation, String> {
    validate_executable(installed_executable)?;
    Ok(
        if current_shell.is_some_and(|current| {
            shell_commands_match(current, &shell_command(installed_executable))
        }) {
            ShellMutation::Delete
        } else {
            ShellMutation::None
        },
    )
}

fn validate_executable(executable: &Path) -> Result<(), String> {
    let text = executable
        .to_str()
        .ok_or("the Nickel shell path must be valid Unicode")?;
    let windows_absolute = text.as_bytes().get(..3).is_some_and(|prefix| {
        prefix[0].is_ascii_alphabetic() && prefix[1] == b':' && prefix[2] == b'\\'
    }) || text.starts_with("\\\\");
    if !executable.is_absolute() && !windows_absolute {
        return Err("the Nickel shell path must be absolute".into());
    }
    if text
        .rsplit(['/', '\\'])
        .next()
        .is_none_or(|name| !name.eq_ignore_ascii_case("nickel.exe"))
    {
        return Err("the Nickel shell path must end in nickel.exe".into());
    }
    if text.contains(['\0', '"']) {
        return Err("the Nickel shell path contains an unsupported character".into());
    }
    Ok(())
}

fn shell_command(executable: &Path) -> String {
    format!("\"{}\"", executable.display())
}

fn shell_commands_match(left: &str, right: &str) -> bool {
    normalize_shell_command(left).eq_ignore_ascii_case(normalize_shell_command(right))
}

fn normalize_shell_command(value: &str) -> &str {
    value.trim().trim_matches('"')
}

#[cfg(target_os = "windows")]
pub fn enable(executable: &Path) -> Result<(), String> {
    let ShellMutation::Set(value) = enable_mutation(executable)? else {
        unreachable!("enable always plans a write")
    };
    windows_registry::set_string(WINLOGON_KEY, SHELL_VALUE, &value)
}

#[cfg(target_os = "windows")]
pub fn disable(installed_executable: &Path) -> Result<(), String> {
    let current = windows_registry::read_string(WINLOGON_KEY, SHELL_VALUE)?;
    if disable_mutation(current.as_deref(), installed_executable)? == ShellMutation::Delete {
        windows_registry::delete_value(WINLOGON_KEY, SHELL_VALUE)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
mod windows_registry {
    use std::{ffi::c_void, mem::size_of};

    use windows::{
        Win32::{
            Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR},
            System::Registry::{
                HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
                RegSetKeyValueW,
            },
        },
        core::PCWSTR,
    };

    const MAX_VALUE_BYTES: u32 = 64 * 1024;

    pub(super) fn read_string(subkey: &str, name: &str) -> Result<Option<String>, String> {
        let subkey = wide(subkey);
        let name = wide(name);
        let mut bytes = 0_u32;
        // SAFETY: both input strings are NUL terminated and the sizing call provides no buffer.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&raw mut bytes),
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(status, "read the current per-user shell")?;
        if bytes == 0 || bytes > MAX_VALUE_BYTES {
            return Err("the current per-user shell value has an invalid size".into());
        }

        let mut data = vec![0_u16; bytes.div_ceil(2) as usize];
        // SAFETY: the allocated buffer is at least as large as the byte count returned above.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(data.as_mut_ptr().cast::<c_void>()),
                Some(&raw mut bytes),
            )
        };
        check(status, "read the current per-user shell")?;
        let units = (bytes as usize / 2).min(data.len());
        let end = data[..units]
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units);
        String::from_utf16(&data[..end])
            .map(Some)
            .map_err(|_| "the current per-user shell is not valid UTF-16".into())
    }

    pub(super) fn set_string(subkey: &str, name: &str, value: &str) -> Result<(), String> {
        let subkey = wide(subkey);
        let name = wide(name);
        let value = wide(value);
        // SAFETY: all strings are NUL terminated and value remains live for the declared size.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                REG_SZ.0,
                Some(value.as_ptr().cast()),
                (value.len() * size_of::<u16>()) as u32,
            )
        };
        check(status, "set Nickel as the per-user shell")
    }

    pub(super) fn delete_value(subkey: &str, name: &str) -> Result<(), String> {
        let subkey = wide(subkey);
        let name = wide(name);
        // SAFETY: both strings are NUL terminated for the duration of the call.
        let status = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            check(status, "restore the default Windows shell")
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain([0]).collect()
    }

    fn check(status: WIN32_ERROR, action: &str) -> Result<(), String> {
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("could not {action}: Windows error {}", status.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{ShellMutation, disable_mutation, enable_mutation};

    const NICKEL: &str = "C:\\Users\\Ada Lovelace\\AppData\\Local\\Nickel\\nickel.exe";

    #[test]
    fn enable_quotes_the_absolute_executable_path() {
        assert_eq!(
            enable_mutation(Path::new(NICKEL)),
            Ok(ShellMutation::Set(format!("\"{NICKEL}\"")))
        );
    }

    #[test]
    fn disable_deletes_only_the_matching_nickel_override() {
        assert_eq!(
            disable_mutation(Some(&format!("\"{NICKEL}\"")), Path::new(NICKEL)),
            Ok(ShellMutation::Delete)
        );
        assert_eq!(
            disable_mutation(Some(NICKEL), Path::new(NICKEL)),
            Ok(ShellMutation::Delete)
        );
        assert_eq!(
            disable_mutation(Some("other-shell.exe"), Path::new(NICKEL)),
            Ok(ShellMutation::None)
        );
        assert_eq!(
            disable_mutation(None, Path::new(NICKEL)),
            Ok(ShellMutation::None)
        );
    }

    #[test]
    fn comparisons_follow_windows_path_case_rules() {
        assert_eq!(
            disable_mutation(
                Some("c:\\users\\ADA LOVELACE\\appdata\\local\\nickel\\NICKEL.EXE"),
                Path::new(NICKEL),
            ),
            Ok(ShellMutation::Delete)
        );
    }

    #[test]
    fn rejects_relative_or_unexpected_executables() {
        assert!(enable_mutation(Path::new("nickel.exe")).is_err());
        assert!(enable_mutation(Path::new("C:\\Nickel\\other.exe")).is_err());
    }
}
