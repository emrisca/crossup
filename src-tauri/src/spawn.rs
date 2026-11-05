//! Starting another program from a macro.
//!
//! `CreateProcessW` rather than `ShellExecuteW`, because only the former hands
//! back a process id — and a Launch block needs that id to answer "wait until
//! its window appears" without mistaking some other program's window for it.
//! The price is that a document or a URL cannot be opened this way, and that
//! arguments are passed through verbatim: there is no shell here to expand a
//! wildcard, split a quoted string or run a pipeline.
//!
//! Nothing is inherited that a macro did not ask for: the new program gets no
//! handles from Crossup, and its working directory is the folder it lives in
//! rather than wherever Crossup happens to be running from.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, BOOL};
use windows::Win32::System::Threading::{
    CreateProcessW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW,
};

/// A program that is now running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Started {
    pub pid: u32,
}

/// Start the program at `path` with `args`, and hand back its process id.
///
/// `args` is appended exactly as written, after the quoted program path; a
/// person who needs quotes around an argument writes them themselves.
pub fn launch(path: &str, args: &str) -> Result<Started, String> {
    let asked = path.trim();
    if asked.is_empty() {
        return Err("no program was chosen".to_string());
    }
    let program = Path::new(asked);
    if !program.is_absolute() {
        return Err(format!("“{asked}” is not a full path"));
    }
    if !program.exists() {
        return Err(format!("there is no program at “{asked}”"));
    }

    // CreateProcessW is allowed to write to the command line it is given, so it
    // must be a mutable buffer rather than a borrowed string.
    let mut line: Vec<u16> = command_line(asked, args)
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let program_wide: Vec<u16> = asked.encode_utf16().chain(Some(0)).collect();
    let folder: Vec<u16> = program
        .parent()
        .map(|parent| parent.as_os_str().encode_wide().chain(Some(0)).collect())
        .unwrap_or_default();
    let folder_ptr = if folder.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(folder.as_ptr())
    };

    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut info = PROCESS_INFORMATION::default();

    unsafe {
        CreateProcessW(
            PCWSTR(program_wide.as_ptr()),
            windows::core::PWSTR(line.as_mut_ptr()),
            None,
            None,
            BOOL(0),
            PROCESS_CREATION_FLAGS(0),
            None,
            folder_ptr,
            &startup,
            &mut info,
        )
        .map_err(|error| describe(&error, asked))?;
        // The handles are not needed: the pid is enough to look the program up
        // again, and holding them would keep the process object alive for as
        // long as Crossup runs.
        let _ = CloseHandle(info.hThread);
        let _ = CloseHandle(info.hProcess);
        Ok(Started {
            pid: info.dwProcessId,
        })
    }
}

/// The command line Windows is given: the program path in quotes, then whatever
/// arguments were typed.
pub fn command_line(path: &str, args: &str) -> String {
    let mut line = format!("\"{}\"", path.trim());
    let args = args.trim();
    if !args.is_empty() {
        line.push(' ');
        line.push_str(args);
    }
    line
}

/// Turn the system error into something worth showing above a macro.
fn describe(error: &windows::core::Error, path: &str) -> String {
    // The low half of the HRESULT is the Win32 code the call failed with.
    let code = (error.code().0 as u32) & 0xFFFF;
    match code {
        // ERROR_ELEVATION_REQUIRED. Nothing Crossup can do about it, and saying
        // so is better than a bare "access denied".
        740 => format!("“{path}” has to run as administrator; start it yourself"),
        // ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND, which the existence
        // check above normally catches: a program can vanish in between.
        2 | 3 => format!("there is no program at “{path}”"),
        _ => format!("could not start “{path}”: {}", error.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell() -> Option<String> {
        let path = std::env::var("ComSpec").ok()?;
        Path::new(&path).exists().then_some(path)
    }

    #[test]
    fn an_empty_path_is_refused() {
        assert_eq!(launch("   ", "").unwrap_err(), "no program was chosen");
    }

    #[test]
    fn a_relative_path_is_refused() {
        let error = launch("notepad.exe", "").unwrap_err();
        assert!(error.contains("is not a full path"), "got {error}");
    }

    #[test]
    fn a_missing_program_is_refused_before_windows_is_asked() {
        let error = launch(r"C:\does\not\exist\nothing.exe", "").unwrap_err();
        assert!(error.contains("there is no program at"), "got {error}");
    }

    #[test]
    fn the_program_path_is_quoted_and_the_arguments_are_not_touched() {
        assert_eq!(
            command_line(r"C:\Program Files\Game\game.exe", ""),
            "\"C:\\Program Files\\Game\\game.exe\""
        );
        assert_eq!(
            command_line(r"C:\tools\t.exe", "  --fast  "),
            "\"C:\\tools\\t.exe\" --fast"
        );
    }

    #[test]
    fn a_real_program_starts_and_reports_its_pid() {
        // Skipped only on a machine with no command processor, which is not a
        // Windows a macro would be running on.
        let Some(shell_path) = shell() else {
            return;
        };
        let started = launch(&shell_path, "/c exit 0").unwrap();
        assert!(started.pid > 0);
    }
}
