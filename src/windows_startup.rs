//! Windows per-user Registry `Run` adapter implementing [`StartupRegistry`].
//!
//! Manages `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` without
//! requiring administrator privileges.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW,
};
use windows::core::{PCWSTR, w};

use crate::startup::{StartupError, StartupRegistry};

const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const APP_VALUE_NAME: PCWSTR = w!("MicrophoneLocker");

/// Production Windows [`StartupRegistry`] backed by per-user `HKCU\...\Run`.
#[derive(Debug, Clone)]
pub struct WindowsStartupRegistry {
    executable_path: PathBuf,
}

impl WindowsStartupRegistry {
    /// Creates a registry adapter pointing to the currently running executable.
    pub fn current_exe() -> Result<Self, StartupError> {
        let executable_path = std::env::current_exe().map_err(|err| {
            StartupError::Registry(format!("failed to resolve current executable path: {err}"))
        })?;
        Ok(Self::new(executable_path))
    }

    /// Creates a registry adapter for a specific executable path.
    pub fn new(executable_path: PathBuf) -> Self {
        Self { executable_path }
    }
}

impl StartupRegistry for WindowsStartupRegistry {
    fn is_enabled(&self) -> Result<bool, StartupError> {
        let mut hkey = HKEY::default();
        // SAFETY: `RUN_SUBKEY` is a valid static NUL-terminated UTF-16 string and
        // `hkey` receives the opened per-user handle.
        let open_status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                RUN_SUBKEY,
                Some(0),
                KEY_QUERY_VALUE,
                &mut hkey,
            )
        };

        if open_status == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        check_win32(open_status, "open HKCU Run key for query")?;
        let _guard = RegKeyGuard(hkey);

        let mut value_type = windows::Win32::System::Registry::REG_VALUE_TYPE::default();
        let mut byte_len: u32 = 0;
        // SAFETY: `hkey` is open for `KEY_QUERY_VALUE` and `APP_VALUE_NAME` is a valid PCWSTR.
        let query_status = unsafe {
            RegQueryValueExW(
                hkey,
                APP_VALUE_NAME,
                None,
                Some(&mut value_type),
                None,
                Some(&mut byte_len),
            )
        };

        if query_status == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        check_win32(query_status, "query MicrophoneLocker startup entry")?;

        Ok(byte_len > 2)
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), StartupError> {
        if enabled {
            let mut hkey = HKEY::default();
            // SAFETY: Creates or opens per-user `HKCU\...\Run` with `KEY_SET_VALUE`.
            let status = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    RUN_SUBKEY,
                    Some(0),
                    None,
                    REG_OPTION_NON_VOLATILE,
                    KEY_SET_VALUE,
                    None,
                    &mut hkey,
                    None,
                )
            };
            check_win32(status, "open or create HKCU Run key")?;
            let _guard = RegKeyGuard(hkey);

            let command = format_run_command(&self.executable_path);
            let wide: Vec<u16> = OsStr::new(&command)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: `wide` is a contiguous Vec<u16>; viewing its initialized bytes as `&[u8]`
            // is valid for `RegSetValueExW` with `REG_SZ`.
            let bytes = unsafe {
                std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2)
            };

            // SAFETY: `hkey` is open with `KEY_SET_VALUE` and `bytes` is a NUL-terminated UTF-16 string.
            let set_status = unsafe {
                RegSetValueExW(hkey, APP_VALUE_NAME, Some(0), REG_SZ, Some(bytes))
            };
            check_win32(set_status, "write MicrophoneLocker startup entry")?;
            Ok(())
        } else {
            let mut hkey = HKEY::default();
            // SAFETY: Opens per-user `HKCU\...\Run` with `KEY_SET_VALUE`.
            let open_status = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    RUN_SUBKEY,
                    Some(0),
                    KEY_SET_VALUE,
                    &mut hkey,
                )
            };
            if open_status == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            check_win32(open_status, "open HKCU Run key for removal")?;
            let _guard = RegKeyGuard(hkey);

            // SAFETY: `hkey` is open with `KEY_SET_VALUE` and `APP_VALUE_NAME` is valid.
            let delete_status = unsafe { RegDeleteValueW(hkey, APP_VALUE_NAME) };
            if delete_status == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            check_win32(delete_status, "delete MicrophoneLocker startup entry")?;
            Ok(())
        }
    }
}

struct RegKeyGuard(HKEY);

impl Drop for RegKeyGuard {
    fn drop(&mut self) {
        // SAFETY: `self.0` is an owned open HKEY returned by RegOpenKeyExW / RegCreateKeyExW.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn check_win32(err: WIN32_ERROR, context: &str) -> Result<(), StartupError> {
    if err == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(StartupError::Registry(format!(
            "failed to {context}: Win32 error {}",
            err.0
        )))
    }
}

fn format_run_command(executable_path: &Path) -> String {
    format!("\"{}\"", executable_path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_run_command_quotes_executable_path_with_spaces() {
        let path = PathBuf::from(r"C:\Program Files\Microphone Locker\microphone-locker.exe");
        assert_eq!(
            format_run_command(&path),
            r#""C:\Program Files\Microphone Locker\microphone-locker.exe""#
        );
    }
}
