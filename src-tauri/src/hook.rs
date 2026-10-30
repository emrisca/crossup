//! The Windows side: a low-level keyboard hook, `SendInput`, and the foreground
//! program that decides which profile is live.
//!
//! The hook runs on its own thread with a message pump, because a `WH_KEYBOARD_LL`
//! hook is delivered to the thread that installed it. It swallows the physical
//! key and replays whatever [`Engine`] decided, with the extended-key flag set
//! from the key table so arrows are never mistaken for the numpad.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    SetWindowsHookExW, TranslateMessage, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED,
    LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

use crate::config::Settings;
use crate::keys;
use crate::socd::{Action, Engine};

/// Everything the hook thread and the UI both touch.
pub struct Shared {
    pub settings: Settings,
    pub engine: Engine,
    pub program: String,
    pub installed: bool,
    pub log: Vec<String>,
    last_probe: Instant,
}

impl Shared {
    pub fn new(settings: Settings) -> Self {
        let mut engine = Engine::new();
        if let Some(profile) = settings.profile_for("") {
            engine.set_rules(profile.mappings.clone());
        }
        Self {
            settings,
            engine,
            program: String::new(),
            installed: false,
            log: Vec::new(),
            last_probe: Instant::now() - Duration::from_secs(5),
        }
    }

    pub fn note(&mut self, line: String) {
        if !self.settings.debug {
            return;
        }
        self.log.push(line);
        if self.log.len() > 200 {
            self.log.remove(0);
        }
    }

    /// Switch the engine to the profile that matches the foreground program.
    pub fn retarget(&mut self, program: &str) -> Vec<Action> {
        self.program = program.to_string();
        let mappings = self
            .settings
            .profile_for(program)
            .map(|profile| profile.mappings.clone())
            .unwrap_or_default();
        if mappings == self.engine.rules() {
            return Vec::new();
        }
        self.engine.set_rules(mappings)
    }
}

/// The hook's state, as everything that needs it holds it.
pub type SharedState = Arc<Mutex<Shared>>;

static SHARED: OnceLock<SharedState> = OnceLock::new();

pub fn install(shared: SharedState) {
    let _ = SHARED.set(shared);
    std::thread::spawn(|| unsafe {
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0);
        // The mouse hook goes on the same thread, so both are serviced by the
        // one message pump below. It is installed whether or not any mouse
        // shortcut is set: it does nothing until one is.
        let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0);
        match hook {
            Ok(hook) => {
                if let Some(shared) = SHARED.get() {
                    if let Ok(mut state) = shared.lock() {
                        state.installed = true;
                        state.note("hook installed".to_string());
                    }
                }
                let mut message = MSG::default();
                // GetMessageW never returns while the hook is installed.
                while GetMessageW(&mut message, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
                if let Ok(mouse_hook) = mouse_hook {
                    let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(mouse_hook);
                }
            }
            Err(error) => {
                if let Some(shared) = SHARED.get() {
                    if let Ok(mut state) = shared.lock() {
                        state.installed = false;
                        state.note(format!("hook failed: {error}"));
                    }
                }
            }
        }
    });
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        // Our own replay must never be re-processed.
        if event.flags.0 & LLKHF_INJECTED.0 == 0 {
            let message = wparam.0 as u32;
            let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;

            if let Some(shared) = SHARED.get() {
                if let Ok(mut state) = shared.lock() {
                    if !state.settings.hook_enabled {
                        return CallNextHookEx(None, code, wparam, lparam);
                    }

                    let now = Instant::now();
                    if now.duration_since(state.last_probe) > Duration::from_millis(200) {
                        state.last_probe = now;
                        let program = foreground_program();
                        let released = state.retarget(&program);
                        send(&released);
                    }

                    // Bit 0x80 is `LLKHF_UP`, not the auto-repeat: reading it
                    // as "repeat" marked every release as one and every repeat
                    // as a fresh press. The engine already knows what it is
                    // holding, so an auto-repeat is caught by it instead.
                    let extended = event.flags.0 & LLKHF_EXTENDED.0 != 0;
                    let scan = keys::normalize(event.scanCode as u16, extended);
                    let decision = state.engine.on_key(scan, down, false);

                    for note in &decision.notes {
                        let line = format!("{}: {note}", keys::label(scan));
                        state.note(line);
                    }

                    send(&decision.actions);

                    if decision.consume {
                        return LRESULT(1);
                    }
                }
            }
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}

/// Mouse buttons, which are only ever read for shortcuts. Nothing here is
/// swallowed: a shortcut fires alongside the click, exactly as pressing a key
/// that happens to be a shortcut still types it.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        if !is_ours(event.flags) {
            crate::mouse_hotkeys::dispatch(wparam.0 as u32, event.mouseData);
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}

/// Whether a mouse event is the app's own `SendInput` output rather than the
/// person's hand.
///
/// The keyboard side has always drawn this line; the mouse side has to draw it
/// too, and for a sharper reason: the clicker and the macros press mouse
/// buttons, so without it a shortcut bound to a button fires on the app's own
/// clicks. A macro whose shortcut is the right button, pressing the right
/// button, re-fires its own shortcut the moment it starts — and the window then
/// reports that the macro could not run because the macro is running, over the
/// report of the run that is going perfectly well.
pub fn is_ours(flags: u32) -> bool {
    flags & LLMHF_INJECTED != 0
}

/// Release or press keys from outside the hook thread (a pause, a profile edit).
pub fn send_public(actions: &[Action]) {
    send(actions);
}

/// Replay the engine's decision as hardware-scan-code input.
fn send(actions: &[Action]) {
    for action in actions {
        let (scan, up) = match action {
            Action::Down(scan) => (*scan, false),
            Action::Up(scan) => (*scan, true),
        };

        // The value may carry the extended bit the pickers use; `SendInput`
        // wants it as a flag, not as part of the code.
        let (scan, extended) = keys::split(scan);

        let mut flags = KEYEVENTF_SCANCODE;
        if up {
            flags |= KEYEVENTF_KEYUP;
        }
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }

        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: scan,
                    dwFlags: KEYBD_EVENT_FLAGS(flags.0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe {
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        }
    }
}

/// The executable name of the window in front, e.g. `Overwatch.exe`.
pub fn foreground_program() -> String {
    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return String::new();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == 0 {
            return String::new();
        }
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

/// Names of running processes, for the profile picker.
pub fn running_processes() -> Vec<String> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut names: Vec<String> = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if let Ok(handle) = snapshot {
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(handle, &mut entry).is_ok() {
                loop {
                    let name = String::from_utf16_lossy(
                        &entry.szExeFile[..entry
                            .szExeFile
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(entry.szExeFile.len())],
                    );
                    if !name.is_empty() && !names.contains(&name) {
                        names.push(name);
                    }
                    if Process32NextW(handle, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(HANDLE(handle.0));
        }
    }
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_own_clicks_are_not_shortcuts() {
        assert!(is_ours(LLMHF_INJECTED), "SendInput sets the injected bit");
        assert!(
            !is_ours(0),
            "a real press carries no flags and is the person's"
        );
    }
}
