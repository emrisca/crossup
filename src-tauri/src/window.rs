//! Other programs' windows: which ones are open, and how to put one in front.
//!
//! Every macro block that talks about windows — focus a window, wait for a
//! program to open one, loop while a program is in front — needs the same two
//! things, so they live here rather than in the engine: a list of the top-level
//! windows a person can actually see, and a way to raise one of them.
//!
//! `SetForegroundWindow` is allowed to refuse. Windows only honours it when the
//! caller is already in the foreground, or the last input came from it, or one
//! of a short list of exceptions applies — and a macro started from a shortcut
//! then asking for a different program hits exactly the case that is denied.
//! The refusal is answered by borrowing the foreground thread's input state for
//! the length of one call, and then by *checking*: a Focus block that silently
//! failed would make the next block type into the wrong program, which is worse
//! than not running at all.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow,
    SW_RESTORE,
};

/// How long to wait between looks at the window list. The same beat as the
/// picture search: the desktop does not change between two reads of it, and a
/// tighter loop only spends processor time.
const POLL_MS: u64 = 120;

/// The longest a run may wait for a window before it is told nothing is coming,
/// however the block was configured.
pub const MAX_WAIT_MS: u32 = 120_000;

/// A top-level window a person could point at.
///
/// The handle is what makes it useful; the rest is what a macro can describe it
/// by, and what a picker shows.
#[derive(Clone, Debug)]
pub struct Window {
    pub hwnd: HWND,
    pub title: String,
    /// The executable that owns the window, e.g. `notepad.exe`. Empty when the
    /// process cannot be opened — an elevated program, typically.
    pub exe: String,
    pub pid: u32,
}

/// Every visible, titled window that is not Crossup's own.
///
/// Returned in the order the desktop stacks them, front to back, so the first
/// match is the one a person would call "the" window. Windows without a title
/// are left out: a macro names windows by title, and a picker full of blanks
/// helps nobody.
pub fn visible() -> Vec<Window> {
    let mut found: Vec<Window> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(collect),
            LPARAM(&mut found as *mut Vec<Window> as isize),
        );
    }
    found
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let found = &mut *(lparam.0 as *mut Vec<Window>);
    if let Some(window) = describe(hwnd) {
        found.push(window);
    }
    // Keep going: one window that cannot be described is not a reason to stop
    // looking at the rest.
    BOOL(1)
}

/// One window, or `None` when it is not something a macro should offer.
fn describe(hwnd: HWND) -> Option<Window> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        let title = title_of(hwnd);
        if title.trim().is_empty() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        // Crossup's own windows — the settings window, the region overlay, the
        // replay badge — are never a target: focusing yourself mid-macro is a
        // mistake, not a workflow.
        if pid == 0 || pid == std::process::id() {
            return None;
        }
        Some(Window {
            hwnd,
            title,
            exe: exe_of_pid(pid),
            pid,
        })
    }
}

fn title_of(hwnd: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let written = GetWindowTextW(hwnd, &mut buffer);
        if written <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buffer[..written as usize])
    }
}

/// The executable name behind a process id, e.g. `chrome.exe`.
///
/// An empty answer is normal: a program running as another user, or elevated
/// while Crossup is not, refuses even the limited query.
pub fn exe_of_pid(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(process);
        if result.is_err() {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buffer[..size as usize]);
        path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string()
    }
}

/// The executable name of the window in front, e.g. `Overwatch.exe`.
pub fn foreground_program() -> String {
    unsafe {
        let front = GetForegroundWindow();
        if front.is_invalid() {
            return String::new();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(front, Some(&mut pid));
        exe_of_pid(pid)
    }
}

/// The first window that matches, waiting up to `timeout_ms` for one to appear.
///
/// `timeout_ms` of zero looks once. A stop asked for during the wait is
/// honoured within one beat, so a long wait does not hold the stop button
/// hostage.
pub fn find(
    matches: impl Fn(&Window) -> bool,
    timeout_ms: u32,
    stop: &AtomicBool,
) -> Result<Window, String> {
    let started = Instant::now();
    let limit = Duration::from_millis(u64::from(timeout_ms.min(MAX_WAIT_MS)));
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err("Stopped".to_string());
        }
        if let Some(window) = visible().into_iter().find(|window| matches(window)) {
            return Ok(window);
        }
        if started.elapsed() >= limit {
            return Err(match timeout_ms {
                0 => "no window like that is open".to_string(),
                ms => format!(
                    "no window like that appeared within {:.1} s",
                    f64::from(ms) / 1000.0
                ),
            });
        }
        std::thread::sleep(Duration::from_millis(POLL_MS));
    }
}

/// Put a window in front and confirm it got there.
///
/// The confirmation is the point of this function: `SetForegroundWindow` says
/// whether the *request* was accepted, and Windows will happily accept it and
/// leave the previous window in front when it decides the caller does not
/// deserve the foreground.
pub fn activate(window: &Window) -> Result<(), String> {
    unsafe {
        if IsIconic(window.hwnd).as_bool() {
            let _ = ShowWindow(window.hwnd, SW_RESTORE);
        }
        if in_front(window.hwnd) {
            return Ok(());
        }
        let _ = SetForegroundWindow(window.hwnd);
        if in_front(window.hwnd) {
            return Ok(());
        }

        // Refused. Attaching this thread's input to the foreground thread's —
        // and to the target's — makes the call count as coming from a process
        // that already has the foreground. Both attachments are undone before
        // returning, because leaving them would tie two programs' input queues
        // together for as long as Crossup runs.
        let front = GetForegroundWindow();
        let front_thread = if front.is_invalid() {
            0
        } else {
            GetWindowThreadProcessId(front, None)
        };
        let target_thread = GetWindowThreadProcessId(window.hwnd, None);
        let ours = GetCurrentThreadId();

        let borrowed_front =
            front_thread != 0 && front_thread != ours && attach(front_thread, ours, true);
        let borrowed_target =
            target_thread != 0 && target_thread != ours && attach(target_thread, ours, true);

        let _ = BringWindowToTop(window.hwnd);
        let _ = SetForegroundWindow(window.hwnd);

        if borrowed_target {
            attach(target_thread, ours, false);
        }
        if borrowed_front {
            attach(front_thread, ours, false);
        }

        if in_front(window.hwnd) {
            Ok(())
        } else {
            Err(format!(
                "Windows would not let Crossup bring “{}” forward",
                window.title.trim()
            ))
        }
    }
}

fn attach(from: u32, to: u32, on: bool) -> bool {
    unsafe { AttachThreadInput(from, to, BOOL::from(on)).as_bool() }
}

fn in_front(hwnd: HWND) -> bool {
    unsafe { GetForegroundWindow() == hwnd }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossups_own_windows_are_never_offered() {
        let ours = std::process::id();
        assert!(visible().iter().all(|window| window.pid != ours));
    }

    #[test]
    fn every_offered_window_has_a_title() {
        assert!(visible()
            .iter()
            .all(|window| !window.title.trim().is_empty()));
    }

    #[test]
    fn an_executable_name_can_be_read_for_this_process() {
        // The test harness is a real process with a real image on disk, so this
        // exercises the query without depending on which programs are open.
        let name = exe_of_pid(std::process::id());
        assert!(name.to_lowercase().ends_with(".exe"), "got {name:?}");
        assert!(!name.contains(['\\', '/']), "got {name:?}");
    }

    #[test]
    fn a_process_that_cannot_be_asked_gives_no_name() {
        assert_eq!(exe_of_pid(0), "");
        // Id 4 is the System process on Windows: it exists and will not answer.
        assert_eq!(exe_of_pid(4), "");
    }

    #[test]
    fn no_timeout_means_one_look_and_a_plain_answer() {
        let stop = AtomicBool::new(false);
        let started = Instant::now();
        let error = find(|_| false, 0, &stop).unwrap_err();
        assert_eq!(error, "no window like that is open");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_wait_stops_when_it_is_told_to() {
        let stop = AtomicBool::new(true);
        let started = Instant::now();
        let error = find(|_| false, 5_000, &stop).unwrap_err();
        assert_eq!(error, "Stopped");
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn a_window_that_is_open_is_found_at_once() {
        // Skipped on a desktop with nothing open: there is nothing to find, and
        // that is a fact about the machine rather than a failure.
        let Some(wanted) = visible().first().cloned() else {
            return;
        };
        let stop = AtomicBool::new(false);
        let found = find(|window| window.pid == wanted.pid, 0, &stop).unwrap();
        assert_eq!(found.pid, wanted.pid);
    }
}
