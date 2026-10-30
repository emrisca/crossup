//! Input synthesis: the only place in this crate that writes to Windows.
//!
//! The auto clicker and the macro runner both talk to the desktop through here,
//! so the engines above it stay pure logic that can be tested without pressing
//! anybody's mouse. Two details are worth stating because they are the
//! difference between working in a game and silently doing nothing:
//!
//! * Keys go out as **hardware scan codes**, not virtual keys, with the
//!   extended-key flag set from [`crate::keys`]. That is what the remapper
//!   already replays, and what a game reading DirectInput sees.
//! * The mouse moves in **absolute virtual-desktop coordinates**. Without
//!   `MOUSEEVENTF_VIRTUALDESK` a second monitor to the left of the primary one
//!   gets coordinates clipped to the primary display.

use serde::{Deserialize, Serialize};

use windows::Win32::Foundation::{BOOL, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState,
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
    KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT,
    MOUSE_EVENT_FLAGS, VIRTUAL_KEY, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

use crate::keys;

/// One wheel notch, the unit `MOUSEEVENTF_WHEEL` counts in.
const WHEEL_DELTA: i32 = 120;

/// Which mouse button an action means. Serde names match the UI's values.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub fn flags(self) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS) {
        match self {
            MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
            MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MouseButton::Left => "Left",
            MouseButton::Right => "Right",
            MouseButton::Middle => "Middle",
        }
    }

    /// The button a macro key block's scan stands for.
    ///
    /// Keyboard scans stop at 0x1FF, so a button lives above them and one
    /// number can mean either input. That keeps a key block a key block: the
    /// same tap/hold/down/up applies, only to a button instead of a key.
    pub fn from_scan(scan: u16) -> Option<Self> {
        match scan {
            0x201 => Some(MouseButton::Left),
            0x202 => Some(MouseButton::Right),
            0x203 => Some(MouseButton::Middle),
            _ => None,
        }
    }
}

// ── timing ─────────────────────────────────────────────────────────

#[link(name = "ntdll")]
extern "system" {
    /// `DesiredResolution` is in 100 ns units, so 10 000 asks for a 1 ms timer.
    /// Microsoft's documented ceiling is 0.5 ms and this is the knob every
    /// accurate auto clicker turns: with the default 15.6 ms tick, `sleep(1ms)`
    /// really waits 15 ms and the click rate collapses.
    fn NtSetTimerResolution(
        desired_resolution: u32,
        set_resolution: u8,
        current_resolution: *mut u32,
    ) -> u32;
}

/// Asks for a 1 ms system timer for as long as it is alive, and puts the old
/// resolution back on drop so closing the app leaves the machine as it was.
pub struct TimerResolution {
    raised: bool,
}

impl TimerResolution {
    pub fn raise() -> Self {
        let mut current = 0u32;
        let status = unsafe { NtSetTimerResolution(10_000, 1, &mut current) };
        Self {
            raised: status == 0,
        }
    }

    pub fn raised(&self) -> bool {
        self.raised
    }
}

impl Drop for TimerResolution {
    fn drop(&mut self) {
        if self.raised {
            let mut current = 0u32;
            unsafe { NtSetTimerResolution(10_000, 0, &mut current) };
        }
    }
}

// ── keyboard ───────────────────────────────────────────────────────

fn key_input(scan: u16, up: bool, unicode: bool) -> INPUT {
    let mut flags = if unicode {
        KEYEVENTF_UNICODE
    } else {
        KEYEVENTF_SCANCODE
    };
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    if !unicode && keys::is_extended(scan) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }

    INPUT {
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
    }
}

fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    unsafe {
        SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Press or release one key by hardware scan code.
pub fn key(scan: u16, down: bool) {
    send(&[key_input(scan, !down, false)]);
}

/// Type text as Unicode, which is layout-independent: sending `é` as packets
/// means the machine does not have to be in a French layout to receive it.
pub fn text(text: &str) {
    let mut inputs: Vec<INPUT> = Vec::with_capacity(text.chars().count() * 2);
    for character in text.chars() {
        // Encode as UTF-16 so anything outside the BMP still arrives.
        let mut buffer = [0u16; 2];
        for unit in character.encode_utf16(&mut buffer) {
            inputs.push(key_input(*unit, false, true));
            inputs.push(key_input(*unit, true, true));
        }
    }
    send(&inputs);
}

// ── mouse ──────────────────────────────────────────────────────────

fn mouse_input(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// The whole virtual desktop: `(x, y, width, height)`, spanning every monitor.
pub fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Every monitor's rectangle, in virtual-desktop pixels.
///
/// The edge and corner failsafes are measured against these rather than against
/// [`virtual_screen`], because with two screens side by side the seam between
/// them is not an edge of the desktop but is an edge of both monitors — and it
/// is exactly where a mouse runs off to when something is clicking too fast.
pub fn monitors() -> Vec<(i32, i32, i32, i32)> {
    /// The callback Windows calls once per monitor. `data` is the `Vec` to fill.
    unsafe extern "system" fn visit(
        monitor: HMONITOR,
        _hdc: HDC,
        _clip: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let list = &mut *(data.0 as *mut Vec<(i32, i32, i32, i32)>);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let rect = info.rcMonitor;
            list.push((
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
            ));
        }
        true.into()
    }

    let mut list: Vec<(i32, i32, i32, i32)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(visit),
            LPARAM(&mut list as *mut Vec<(i32, i32, i32, i32)> as isize),
        );
    }
    if list.is_empty() {
        // A session that reports no monitors still has a desktop to guard.
        list.push(virtual_screen());
    }
    list
}

pub fn cursor_position() -> (i32, i32) {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    (point.x, point.y)
}

/// Whether Escape is physically down right now.
///
/// Read from the key state rather than from the keyboard hook, so the clicker's
/// escape failsafe keeps working while the hook is paused, swallowed by a game,
/// or busy remapping. The high bit is "down"; the low bit is "pressed since the
/// last call" and is deliberately ignored, because polling it would depend on
/// how often the loop happens to ask.
pub fn escape_down() -> bool {
    unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) as u16 & 0x8000 != 0 }
}

/// The event that moves the cursor to a point, normalised across `screen`.
///
/// `MOUSEEVENTF_MOVE` is the part that is easy to leave out and impossible to
/// notice: with only `ABSOLUTE` and `VIRTUALDESK` set, Windows drops the event
/// and the cursor does not move, and `SendInput` reports no error for it. The
/// cursor simply stays where it was while the clicker counts its clicks — which
/// is how "where it clicks" looked broken when the points were fine all along.
///
/// `None` for a screen with no area to scale against.
fn absolute_move_input(x: i32, y: i32, screen: (i32, i32, i32, i32)) -> Option<INPUT> {
    let (origin_x, origin_y, width, height) = screen;
    if width <= 1 || height <= 1 {
        return None;
    }
    // Absolute mouse coordinates are normalised into 0..65535 across the
    // virtual desktop, so the origin has to be subtracted before scaling.
    let normalised_x = ((x - origin_x) as f64 * 65_535.0 / (width - 1) as f64).round() as i32;
    let normalised_y = ((y - origin_y) as f64 * 65_535.0 / (height - 1) as f64).round() as i32;
    Some(mouse_input(
        normalised_x,
        normalised_y,
        0,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    ))
}

/// Move the cursor to a point in virtual-desktop pixels.
pub fn move_to(x: i32, y: i32) {
    if let Some(input) = absolute_move_input(x, y, virtual_screen()) {
        send(&[input]);
    }
}

/// Move the cursor by a relative amount. `MOUSEEVENTF_MOVE` is again what asks
/// for movement rather than a button event.
pub fn move_by(dx: i32, dy: i32) {
    send(&[mouse_input(dx, dy, 0, MOUSEEVENTF_MOVE)]);
}

pub fn button(button: MouseButton, down: bool) {
    let (down_flag, up_flag) = button.flags();
    send(&[mouse_input(0, 0, 0, if down { down_flag } else { up_flag })]);
}

/// Many press/release pairs in one `SendInput` call.
///
/// This is how the high rates are reached: at 500 CPS one click is due every
/// 2 ms, which is inside the noise of a single wake-up, so the engine hands the
/// kernel a batch and lets it pace them.
pub fn click_batch(button: MouseButton, count: usize) {
    if count == 0 {
        return;
    }
    let (down_flag, up_flag) = button.flags();
    let mut inputs: Vec<INPUT> = Vec::with_capacity(count * 2);
    for _ in 0..count {
        inputs.push(mouse_input(0, 0, 0, down_flag));
        inputs.push(mouse_input(0, 0, 0, up_flag));
    }
    send(&inputs);
}

/// One press/release pair for a keyboard key, batched the same way.
pub fn key_batch(scan: u16, count: usize) {
    if count == 0 {
        return;
    }
    let mut inputs: Vec<INPUT> = Vec::with_capacity(count * 2);
    for _ in 0..count {
        inputs.push(key_input(scan, false, false));
        inputs.push(key_input(scan, true, false));
    }
    send(&inputs);
}

/// Wheel scroll. Positive `amount` scrolls up, negative down; each unit is one
/// notch (120 of the 120-per-notch `WHEEL_DELTA`).
pub fn scroll(amount: i32) {
    if amount == 0 {
        return;
    }
    send(&[mouse_input(
        0,
        0,
        (amount * WHEEL_DELTA) as u32,
        MOUSEEVENTF_WHEEL,
    )]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_buttons_map_to_the_right_flags() {
        assert_eq!(MouseButton::Left.flags().0, MOUSEEVENTF_LEFTDOWN);
        assert_eq!(MouseButton::Right.flags().1, MOUSEEVENTF_RIGHTUP);
        assert_eq!(MouseButton::Middle.flags().0, MOUSEEVENTF_MIDDLEDOWN);
    }

    #[test]
    fn a_key_packet_carries_the_scan_code_not_a_virtual_key() {
        let down = key_input(0x11, false, false);
        let ki = unsafe { down.Anonymous.ki };
        assert_eq!(ki.wScan, 0x11);
        assert_eq!(ki.wVk.0, 0, "a virtual key would break DirectInput games");
        assert!(ki.dwFlags.0 & KEYEVENTF_SCANCODE.0 != 0);

        let up = key_input(0x11, true, false);
        assert!(unsafe { up.Anonymous.ki }.dwFlags.0 & KEYEVENTF_KEYUP.0 != 0);
    }

    #[test]
    fn an_absolute_move_asks_for_the_move() {
        let input = absolute_move_input(960, 540, (0, 0, 1920, 1080)).expect("a 1920x1080 desktop");
        let mi = unsafe { input.Anonymous.mi };
        for flag in [MOUSEEVENTF_MOVE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_VIRTUALDESK] {
            assert_eq!(mi.dwFlags.0 & flag.0, flag.0, "{flag:?} must be set");
        }
        assert!(mi.dx > 0 && mi.dy > 0, "normalised into the screen, not left as pixels");
        // A screen with no area has nothing to scale against.
        assert!(absolute_move_input(0, 0, (0, 0, 0, 0)).is_none());
    }

    #[test]
    fn an_absolute_move_subtracts_a_second_screens_origin() {
        // 1920x1080 to the left of the primary, so the desktop runs -1920..1919
        // and the seam between the screens is x = 0, halfway across.
        let input = absolute_move_input(0, 0, (-1920, 0, 3840, 1080)).expect("two screens");
        let mi = unsafe { input.Anonymous.mi };
        assert!(
            (mi.dx - 32_768).abs() <= 16,
            "the seam is halfway across, not at the origin: {}",
            mi.dx
        );
        assert_eq!(mi.dy, 0);
    }

    #[test]
    fn extended_keys_are_flagged_so_arrows_are_not_numpad() {
        let arrow = key_input(0x48, false, false);
        assert!(unsafe { arrow.Anonymous.ki }.dwFlags.0 & KEYEVENTF_EXTENDEDKEY.0 != 0);

        let letter = key_input(0x11, false, false);
        assert!(unsafe { letter.Anonymous.ki }.dwFlags.0 & KEYEVENTF_EXTENDEDKEY.0 == 0);
    }

    #[test]
    fn text_typing_uses_unicode_packets() {
        let packet = key_input('é' as u16, false, true);
        let ki = unsafe { packet.Anonymous.ki };
        assert_eq!(ki.wScan, 'é' as u16);
        assert!(ki.dwFlags.0 & KEYEVENTF_UNICODE.0 != 0);
        assert!(ki.dwFlags.0 & KEYEVENTF_SCANCODE.0 == 0);
    }

    /// The engines treat this as the desktop's size; a zero or negative answer
    /// would mean the scaling maths divides by nothing.
    #[test]
    fn the_virtual_desktop_has_a_size() {
        let (_, _, width, height) = virtual_screen();
        assert!(width > 0 && height > 0, "virtual desktop was {width}x{height}");
    }

    #[test]
    fn every_monitor_has_a_rectangle_inside_the_virtual_desktop() {
        let (origin_x, origin_y, width, height) = virtual_screen();
        let monitors = monitors();
        assert!(!monitors.is_empty(), "a machine with no monitors has no cursor");
        for (x, y, w, h) in monitors {
            assert!(w > 0 && h > 0, "a monitor was {w}x{h}");
            assert!(
                x >= origin_x && y >= origin_y,
                "monitor at {x},{y} starts before the desktop at {origin_x},{origin_y}"
            );
            assert!(
                x + w <= origin_x + width && y + h <= origin_y + height,
                "monitor at {x},{y} {w}x{h} runs past the desktop {origin_x},{origin_y} {width}x{height}"
            );
        }
    }

    /// The cursor is on exactly one monitor, which is what lets the failsafes
    /// answer per screen.
    #[test]
    fn the_monitors_contain_the_cursor() {
        let (x, y) = cursor_position();
        let hit = monitors()
            .into_iter()
            .filter(|(mx, my, mw, mh)| {
                x >= *mx && x < mx + mw && y >= *my && y < my + mh
            })
            .count();
        assert_eq!(hit, 1, "the cursor at {x},{y} was inside {hit} monitors");
    }
}
