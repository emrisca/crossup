//! Starting with Windows.
//!
//! There is one supported way to do this for a per-user app that does not want
//! elevation: a value under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. No service, no
//! scheduled task, and it is the same place Task Manager's Startup tab reads —
//! so the switch here and the one in Task Manager are the same switch, which is
//! what you want when one of them looks wrong.
//!
//! The executable is configurable rather than always `current_exe()`, because a
//! copy that has been moved, or a portable build next to an installed one, gets
//! this wrong in a way that is invisible until the next sign-in.

use std::path::Path;

use serde::Serialize;
use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};

/// The Run key, spelled the way Windows spells it.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// What the entry is called, in the registry and in Task Manager.
pub const VALUE_NAME: &str = "Crossup";

/// What the window shows about the sign-in entry.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutostartView {
    /// Whether the Run entry exists at all.
    pub enabled: bool,
    /// The command it runs, as stored.
    pub entry: Option<String>,
    /// The executable the entry is written with when it is switched on.
    pub exe: String,
    /// The executable that is running right now.
    pub running: String,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Open the Run key for reading, or for writing as well.
fn open(write: bool) -> Result<HKEY, String> {
    let path = wide(RUN_KEY);
    let mut key = HKEY::default();
    let access = if write {
        KEY_QUERY_VALUE | KEY_SET_VALUE
    } else {
        KEY_QUERY_VALUE
    };
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, access, &mut key) };
    if status.0 != 0 {
        return Err(format!(
            "could not open the Run key in the registry (Windows error {})",
            status.0
        ));
    }
    Ok(key)
}

/// The command the sign-in entry runs, if there is one.
///
/// Read as UTF-16 because that is what `REG_SZ` is; the surrounding quotes are
/// stripped so the path can be shown and compared as a path.
pub fn entry() -> Option<String> {
    let key = open(false).ok()?;
    let name = wide(VALUE_NAME);

    let mut length: u32 = 0;
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            None,
            None,
            Some(&mut length),
        )
    };
    if status.0 != 0 || length == 0 {
        unsafe { let _ = RegCloseKey(key); }
        return None;
    }

    let mut buffer = vec![0u8; length as usize];
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            None,
            Some(buffer.as_mut_ptr()),
            Some(&mut length),
        )
    };
    unsafe { let _ = RegCloseKey(key); }
    if status.0 != 0 {
        return None;
    }

    let units: Vec<u16> = buffer
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    let text = String::from_utf16_lossy(&units);
    Some(text.trim().trim_matches('"').to_string())
}

/// Put the executable in the sign-in entry.
pub fn enable(exe: &str) -> Result<(), String> {
    let path = Path::new(exe);
    if !path.is_file() {
        return Err(format!("there is no executable at {exe}"));
    }

    let key = open(true)?;
    let name = wide(VALUE_NAME);
    // Quoted, because the default install lives under `Program Files` and a Run
    // value with a space in it is split at the space otherwise.
    let command = wide(&format!("\"{}\"", path.display()));
    let bytes: Vec<u8> = command.iter().flat_map(|unit| unit.to_le_bytes()).collect();

    let status = unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(&bytes)) };
    unsafe { let _ = RegCloseKey(key); }
    if status.0 != 0 {
        return Err(format!(
            "Windows refused the sign-in entry (error {})",
            status.0
        ));
    }
    Ok(())
}

/// Take the sign-in entry out again. Removing one that is already gone is fine.
pub fn disable() -> Result<(), String> {
    let key = open(true)?;
    let name = wide(VALUE_NAME);
    let status = unsafe { RegDeleteValueW(key, PCWSTR(name.as_ptr())) };
    unsafe { let _ = RegCloseKey(key); }
    // 2 is ERROR_FILE_NOT_FOUND: nothing to delete, which is the state asked for.
    if status.0 != 0 && status.0 != 2 {
        return Err(format!(
            "Windows refused to remove the sign-in entry (error {})",
            status.0
        ));
    }
    Ok(())
}

/// The executable that is running now, which is the sensible default.
pub fn running_exe() -> String {
    std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

/// What the window should show, given the settings' chosen executable.
pub fn view(chosen: &str) -> AutostartView {
    let running = running_exe();
    let exe = if chosen.trim().is_empty() {
        running.clone()
    } else {
        chosen.trim().to_string()
    };
    AutostartView {
        enabled: entry().is_some(),
        entry: entry(),
        exe,
        running,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The entry name and key are what Task Manager and every other guide
    /// expect; a rename would quietly orphan an existing entry.
    #[test]
    fn the_entry_is_named_and_placed_where_windows_expects() {
        assert_eq!(VALUE_NAME, "Crossup");
        assert_eq!(RUN_KEY, r"Software\Microsoft\Windows\CurrentVersion\Run");
    }

    #[test]
    fn a_path_that_does_not_exist_is_refused_before_writing_anything() {
        let error = enable(r"C:\nowhere\Crossup.exe").expect_err("that path cannot exist");
        assert!(error.contains("no executable"), "{error}");
    }

    /// Reading the current state must never panic, whether or not the entry is
    /// there — the window asks for it every time settings are opened.
    #[test]
    fn reading_the_entry_is_safe() {
        let state = entry();
        assert!(state.is_none() || !state.unwrap().is_empty());
    }

    #[test]
    fn the_view_falls_back_to_the_running_copy() {
        let shown = view("");
        assert_eq!(shown.exe, shown.running);
        assert!(!shown.exe.is_empty());

        let chosen = view(r"C:\Somewhere\Crossup.exe");
        assert_eq!(chosen.exe, r"C:\Somewhere\Crossup.exe");
    }
}
