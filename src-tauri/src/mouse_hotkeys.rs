//! Mouse-button shortcuts.
//!
//! The global shortcut layer handles keyboards only: a binding is a modifier
//! combination plus one `Code`, and there is no mouse button in that vocabulary.
//! A clicker whose shortcut is "the button under my thumb" is a normal thing to
//! want, though — a side button on the mouse is the one key you can press
//! without leaving the mouse — so those bindings are watched here instead, from
//! a `WH_MOUSE_LL` hook installed on the same thread as the keyboard hook.
//!
//! A wheel notch is watched here too, and it is the odd one out: Windows reports
//! it as a single message carrying a signed delta and nothing follows, so it is
//! only ever a press. A macro reads that press and runs; a binding that needs a
//! release as well cannot be answered by a notch, which is why the clicker
//! refuses one in its hold mode (`runtime::register_shortcuts`).
//!
//! Two details are worth knowing:
//!
//! * The modifiers are read with `GetAsyncKeyState` rather than tracked from
//!   events. A low-level hook runs before the input is delivered to any window,
//!   so the async state *is* the state at that moment, and it saves keeping a
//!   parallel copy in sync with a hook that can be uninstalled behind our back.
//! * A shortcut is dropped while the cursor is over Hitboxer's own window. That
//!   is what makes assigning one possible at all: without it, clicking into the
//!   recorder would fire the shortcut that is being replaced. It also means a
//!   mouse shortcut does not fire while you are using the app itself, which is
//!   the same rule the original uses.
//!
//! The binding must match its modifiers exactly, so an unmodified `Mouse4` does
//! not also answer to `Control+Mouse4`.

use std::sync::{Mutex, MutexGuard, OnceLock};

use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetCursorPos, GetWindowThreadProcessId, WindowFromPoint, GA_ROOT,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEWHEEL, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

/// Which button a mouse shortcut uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    Left,
    Right,
    Middle,
    /// The back button on the side, `Mouse4` to Windows.
    Back,
    /// The forward button on the side, `Mouse5`.
    Forward,
    /// One notch of the wheel, away from you — up a page.
    WheelUp,
    /// One notch of the wheel, towards you — down a page.
    WheelDown,
}

impl Button {
    /// A notch rather than a button: something that happens once, and cannot be
    /// held down, so a binding that waits for a release can never be answered.
    pub fn is_wheel(self) -> bool {
        matches!(self, Button::WheelUp | Button::WheelDown)
    }

    fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "mouse1" | "mouseleft" | "leftmouse" | "lmb" => Button::Left,
            "mouse2" | "mouseright" | "rightmouse" | "rmb" => Button::Right,
            "mouse3" | "mousemiddle" | "middlemouse" | "mmb" => Button::Middle,
            "mouse4" | "xbutton1" | "back" => Button::Back,
            "mouse5" | "xbutton2" | "forward" => Button::Forward,
            "wheelup" | "scrollup" | "mousewheelup" | "mousewheel" => Button::WheelUp,
            "wheeldown" | "scrolldown" | "mousewheeldown" => Button::WheelDown,
            _ => return None,
        })
    }

    /// The button a hook message is about, and which way it went.
    ///
    /// `mouse_data` is the `MSLLHOOKSTRUCT` field: for the extra buttons its
    /// high word is which one, which is the only place that is reported.
    fn from_message(message: u32, mouse_data: u32) -> Option<(Self, bool)> {
        Some(match message {
            WM_LBUTTONDOWN => (Button::Left, true),
            WM_LBUTTONUP => (Button::Left, false),
            WM_RBUTTONDOWN => (Button::Right, true),
            WM_RBUTTONUP => (Button::Right, false),
            WM_MBUTTONDOWN => (Button::Middle, true),
            WM_MBUTTONUP => (Button::Middle, false),
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                let which = (mouse_data >> 16) & 0xFFFF;
                let button = if which == 1 { Button::Back } else { Button::Forward };
                (button, message == WM_XBUTTONDOWN)
            }
            // The one event that is a whole press in itself. The high word is a
            // signed delta: positive is a wheel notch away from you, which
            // scrolls up. Nothing follows it, so it is reported as going down.
            WM_MOUSEWHEEL => {
                let delta = ((mouse_data >> 16) & 0xFFFF) as u16 as i16;
                if delta > 0 {
                    (Button::WheelUp, true)
                } else if delta < 0 {
                    (Button::WheelDown, true)
                } else {
                    return None;
                }
            }
            _ => return None,
        })
    }
}

/// The modifiers a mouse shortcut requires, and which ones are held.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Mods {
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Mods {
    /// The modifiers held at this instant.
    fn held() -> Self {
        let down = |key: i32| unsafe { GetAsyncKeyState(key) as u16 & 0x8000 != 0 };
        Self {
            control: down(VK_CONTROL.0 as i32),
            shift: down(VK_SHIFT.0 as i32),
            alt: down(VK_MENU.0 as i32),
            meta: down(VK_LWIN.0 as i32) || down(VK_RWIN.0 as i32),
        }
    }
}

/// What a shortcut does when it fires.
#[derive(Clone, Debug)]
pub enum Action {
    /// Start, stop, or toggle the auto clicker, depending on its activation mode.
    Clicker { hold: bool },
    /// Run a macro. Held only on the way down, like the keyboard version.
    Macro { id: String, name: String },
}

/// One watched button.
#[derive(Clone, Debug)]
pub struct Binding {
    pub button: Button,
    pub mods: Mods,
    pub action: Action,
}

/// Read a shortcut written the way the UI writes it: `Mouse4`, `Control+Mouse2`.
///
/// Returns `None` for anything that is not a mouse button, which is how the
/// caller tells a mouse shortcut from a keyboard one. A shortcut this cannot
/// read is left to the keyboard layer to reject, so the message the user sees
/// still comes from one place.
pub fn parse(text: &str) -> Option<(Button, Mods)> {
    let mut mods = Mods::default();
    let mut button: Option<Button> = None;
    for part in text.split('+') {
        let token = part.trim().to_ascii_lowercase();
        if token.is_empty() {
            return None;
        }
        match token.as_str() {
            "control" | "ctrl" => mods.control = true,
            "shift" => mods.shift = true,
            "alt" | "option" => mods.alt = true,
            "meta" | "cmd" | "super" => mods.meta = true,
            other => {
                // Two main keys is not a shortcut.
                if button.is_some() {
                    return None;
                }
                button = Some(Button::parse(other)?);
            }
        }
    }
    Some((button?, mods))
}

/// The bindings in force, replaced whole whenever the settings change.
static BINDINGS: Mutex<Vec<Binding>> = Mutex::new(Vec::new());
/// The handle the hook thread dispatches through. The hook thread is not given
/// one, because it outlives every window and is installed before they exist.
static APP: OnceLock<AppHandle> = OnceLock::new();

fn bindings() -> MutexGuard<'static, Vec<Binding>> {
    BINDINGS.lock().unwrap_or_else(|error| error.into_inner())
}

/// Replace the watched bindings. An empty list simply stops watching anything.
pub fn set(app: &AppHandle, list: Vec<Binding>) {
    let _ = APP.set(app.clone());
    *bindings() = list;
}

/// Is the cursor over one of our own windows?
///
/// The picker counts as one: it is stretched over every monitor while a zone is
/// being drawn, and a clicker firing underneath it would be a surprise.
fn cursor_over_own_window() -> bool {
    unsafe {
        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_err() {
            return false;
        }
        let window = WindowFromPoint(point);
        if window.0.is_null() {
            return false;
        }
        // A child of the webview is ours too, so the check is against the root.
        let root = GetAncestor(window, GA_ROOT);
        let root = if root.0.is_null() { window } else { root };
        let mut pid = 0u32;
        GetWindowThreadProcessId(root, Some(&mut pid));
        pid == GetCurrentProcessId()
    }
}

/// Called from the mouse hook for every button event.
///
/// It must not panic and it must not block: a low-level hook that takes too long
/// is removed by Windows. Everything it does is a lookup plus, at most, handing
/// the work to the runtime, which starts its own thread.
pub fn dispatch(message: u32, mouse_data: u32) {
    let Some((button, down)) = Button::from_message(message, mouse_data) else {
        return;
    };

    // Matched before anything is measured, so the common case — a button nobody
    // bound — costs one lock and no work at all.
    let mods = Mods::held();
    let firing: Vec<Action> = {
        let watched = bindings();
        watched
            .iter()
            .filter(|binding| binding.button == button && binding.mods == mods)
            .map(|binding| binding.action.clone())
            .collect()
    };
    if firing.is_empty() || cursor_over_own_window() {
        return;
    }

    let Some(app) = APP.get().cloned() else {
        return;
    };

    let Some(runtime) = app.try_state::<crate::runtime::RuntimeState>() else {
        return;
    };
    let runtime = runtime.inner().clone();

    for action in firing {
        match action {
            Action::Clicker { hold } => {
                crate::runtime::clicker_shortcut(&app, &runtime, hold, down);
            }
            Action::Macro { id, name } => {
                // Only on the way down, so holding the button does not rerun it.
                if down {
                    crate::runtime::macro_shortcut(&app, &runtime, &id, &name);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> (Button, Mods) {
        parse(text).unwrap_or_else(|| panic!("“{text}” should be a mouse shortcut"))
    }

    #[test]
    fn every_button_round_trips() {
        assert_eq!(parsed("Mouse1").0, Button::Left);
        assert_eq!(parsed("Mouse2").0, Button::Right);
        assert_eq!(parsed("Mouse3").0, Button::Middle);
        assert_eq!(parsed("Mouse4").0, Button::Back);
        assert_eq!(parsed("Mouse5").0, Button::Forward);
        assert_eq!(parsed("WheelUp").0, Button::WheelUp);
        assert_eq!(parsed("WheelDown").0, Button::WheelDown);
    }

    /// A notch is a press, so it is reported as going down and can be told
    /// apart from a button that can also be released.
    #[test]
    fn a_notch_is_a_press_and_nothing_else() {
        assert_eq!(Button::from_message(WM_MOUSEWHEEL, 120 << 16), Some((Button::WheelUp, true)));
        assert_eq!(
            Button::from_message(WM_MOUSEWHEEL, (120u32) << 16 | 0xFFFF0000),
            Some((Button::WheelDown, true))
        );
        // A delta of zero is not a notch at all.
        assert_eq!(Button::from_message(WM_MOUSEWHEEL, 0), None);
        assert!(Button::WheelUp.is_wheel() && Button::WheelDown.is_wheel());
        assert!(!Button::Middle.is_wheel() && !Button::Back.is_wheel());
    }

    #[test]
    fn a_wheel_shortcut_takes_modifiers_like_any_other() {
        let (button, mods) = parsed("Control+WheelDown");
        assert_eq!(button, Button::WheelDown);
        assert!(mods.control);
    }

    /// The names are the ones the original stores, so a config moved between the
    /// two reads the same either way.
    #[test]
    fn the_original_names_are_understood_too() {
        assert_eq!(parsed("mouseleft").0, Button::Left);
        assert_eq!(parsed("mouse4").0, Button::Back);
        assert_eq!(parsed("xbutton2").0, Button::Forward);
        assert_eq!(parsed("MOUSE4").0, Button::Back);
    }

    #[test]
    fn modifiers_are_read_in_any_order() {
        let (button, mods) = parsed("Shift+Control+Mouse4");
        assert_eq!(button, Button::Back);
        assert!(mods.control && mods.shift && !mods.alt && !mods.meta);

        let (button, mods) = parsed("Ctrl+Mouse2");
        assert_eq!(button, Button::Right);
        assert!(mods.control);
    }

    /// A keyboard shortcut is not this module's business, and neither is a
    /// shortcut with two main keys.
    #[test]
    fn keyboard_shortcuts_and_nonsense_are_left_alone() {
        assert_eq!(parse("F6"), None);
        assert_eq!(parse("Control+Alt+1"), None);
        assert_eq!(parse("Mouse9"), None);
        assert_eq!(parse("Mouse4+Mouse5"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("Control+"), None);
    }

    #[test]
    fn hook_messages_map_to_the_right_button_and_direction() {
        assert_eq!(Button::from_message(WM_LBUTTONDOWN, 0), Some((Button::Left, true)));
        assert_eq!(Button::from_message(WM_LBUTTONUP, 0), Some((Button::Left, false)));
        assert_eq!(Button::from_message(WM_RBUTTONDOWN, 0), Some((Button::Right, true)));
        assert_eq!(Button::from_message(WM_MBUTTONUP, 0), Some((Button::Middle, false)));
        // The extra buttons are named in the high word of mouseData.
        assert_eq!(
            Button::from_message(WM_XBUTTONDOWN, 1 << 16),
            Some((Button::Back, true))
        );
        assert_eq!(
            Button::from_message(WM_XBUTTONUP, 2 << 16),
            Some((Button::Forward, false))
        );
        // Anything else is not a shortcut: the horizontal wheel is a gesture,
        // not a button, and neither is a message we never see from the hook.
        assert_eq!(Button::from_message(0x020E, 120 << 16), None);
        assert_eq!(Button::from_message(0x0200, 0), None);
    }
}
