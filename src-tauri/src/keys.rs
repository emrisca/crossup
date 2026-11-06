//! Hardware scan codes (set 1), which is what the original stores and what the
//! low-level hook reports. Names follow the original's convention where it had
//! one (`ARROW_UP`, `SPACEBAR`) and are friendly elsewhere.

/// A key the UI can offer. `extended` matters for `SendInput`: arrows, Insert,
/// Home and friends need `KEYEVENTF_EXTENDEDKEY` or the game sees numpad keys.
pub struct KeyInfo {
    pub scan: u16,
    pub label: &'static str,
    pub extended: bool,
    pub group: &'static str,
}

macro_rules! keys {
    ($(($scan:expr, $label:expr, $group:expr)),* $(,)?) => {
        &[$(KeyInfo { scan: $scan, label: $label, extended: false, group: $group }),*]
    };
}

const EXTENDED_KEYS: &[KeyInfo] = &[
    KeyInfo { scan: 0x48, label: "↑", extended: true, group: "Arrows" },
    KeyInfo { scan: 0x50, label: "↓", extended: true, group: "Arrows" },
    KeyInfo { scan: 0x4B, label: "←", extended: true, group: "Arrows" },
    KeyInfo { scan: 0x4D, label: "→", extended: true, group: "Arrows" },
    KeyInfo { scan: 0x52, label: "Insert", extended: true, group: "Nav" },
    KeyInfo { scan: 0x53, label: "Delete", extended: true, group: "Nav" },
    KeyInfo { scan: 0x47, label: "Home", extended: true, group: "Nav" },
    KeyInfo { scan: 0x4F, label: "End", extended: true, group: "Nav" },
    KeyInfo { scan: 0x49, label: "Page up", extended: true, group: "Nav" },
    KeyInfo { scan: 0x51, label: "Page down", extended: true, group: "Nav" },
    // The right-hand modifiers need a scan code of their own. The hook reports
    // `0x1D` and `0x38` for both sides and distinguishes them with the extended
    // flag, so keeping them at the plain code made these rows duplicates of
    // Left Ctrl and Left Alt in every picker — the dropdown then snapped the
    // choice back to whichever row came first, and "Left Ctrl" could not be
    // selected at all.
    KeyInfo { scan: EXTENDED_BIT | 0x1D, label: "Right Ctrl", extended: true, group: "Modifiers" },
    KeyInfo { scan: EXTENDED_BIT | 0x38, label: "Right Alt", extended: true, group: "Modifiers" },
    // The rest of the keys the hook can report with the flag set, each with a
    // row of its own so a press can be picked and read back by name instead of
    // arriving as a bare scan number — or, in the chord block, not arriving at
    // all because nothing in the UI named it.
    KeyInfo { scan: EXTENDED_BIT | 0x1C, label: "Numpad Enter", extended: true, group: "Numpad" },
    KeyInfo { scan: EXTENDED_BIT | 0x35, label: "Numpad /", extended: true, group: "Numpad" },
    KeyInfo { scan: EXTENDED_BIT | 0x37, label: "Print Screen", extended: true, group: "Nav" },
    KeyInfo { scan: EXTENDED_BIT | 0x5B, label: "Left Windows", extended: true, group: "Modifiers" },
    KeyInfo { scan: EXTENDED_BIT | 0x5C, label: "Right Windows", extended: true, group: "Modifiers" },
    KeyInfo { scan: EXTENDED_BIT | 0x5D, label: "Menu", extended: true, group: "Nav" },
];

/// Marks an extended key in a scan value the UI sees.
///
/// Only the two modifiers use it, because only they collide with a main-table
/// key. Arrows keep their plain codes so `.socd` files and settings written by
/// the original still resolve.
pub const EXTENDED_BIT: u16 = 0x100;

/// The scan code to send, and whether it needs the extended flag.
pub fn split(scan: u16) -> (u16, bool) {
    if scan & EXTENDED_BIT != 0 {
        (scan & !EXTENDED_BIT, true)
    } else {
        (scan, is_extended(scan))
    }
}

/// Turn what the hook reports into the value the tables are keyed by.
pub fn normalize(scan: u16, extended: bool) -> u16 {
    // The codes that have an extended row of their own, so what the picker stores
    // is what the hook later matches. An arrow is left alone so a saved mapping
    // still matches.
    if extended && matches!(scan, 0x1D | 0x38 | 0x1C | 0x35 | 0x37 | 0x5B | 0x5C | 0x5D) {
        scan | EXTENDED_BIT
    } else {
        scan
    }
}

const MAIN_KEYS: &[KeyInfo] = keys![
    (0x01, "Esc", "Nav"),
    (0x02, "1", "Digits"),
    (0x03, "2", "Digits"),
    (0x04, "3", "Digits"),
    (0x05, "4", "Digits"),
    (0x06, "5", "Digits"),
    (0x07, "6", "Digits"),
    (0x08, "7", "Digits"),
    (0x09, "8", "Digits"),
    (0x0A, "9", "Digits"),
    (0x0B, "0", "Digits"),
    (0x0C, "-", "Digits"),
    (0x0D, "=", "Digits"),
    (0x0E, "Backspace", "Nav"),
    (0x0F, "Tab", "Nav"),
    (0x10, "Q", "Letters"),
    (0x11, "W", "Letters"),
    (0x12, "E", "Letters"),
    (0x13, "R", "Letters"),
    (0x14, "T", "Letters"),
    (0x15, "Y", "Letters"),
    (0x16, "U", "Letters"),
    (0x17, "I", "Letters"),
    (0x18, "O", "Letters"),
    (0x19, "P", "Letters"),
    (0x1A, "[", "Letters"),
    (0x1B, "]", "Letters"),
    (0x1C, "Enter", "Nav"),
    (0x1D, "Left Ctrl", "Modifiers"),
    (0x1E, "A", "Letters"),
    (0x1F, "S", "Letters"),
    (0x20, "D", "Letters"),
    (0x21, "F", "Letters"),
    (0x22, "G", "Letters"),
    (0x23, "H", "Letters"),
    (0x24, "J", "Letters"),
    (0x25, "K", "Letters"),
    (0x26, "L", "Letters"),
    (0x27, ";", "Letters"),
    (0x28, "'", "Letters"),
    (0x29, "`", "Letters"),
    (0x2A, "Left Shift", "Modifiers"),
    (0x2B, "\\", "Letters"),
    (0x2C, "Z", "Letters"),
    (0x2D, "X", "Letters"),
    (0x2E, "C", "Letters"),
    (0x2F, "V", "Letters"),
    (0x30, "B", "Letters"),
    (0x31, "N", "Letters"),
    (0x32, "M", "Letters"),
    (0x33, ",", "Letters"),
    (0x34, ".", "Letters"),
    (0x35, "/", "Letters"),
    (0x36, "Right Shift", "Modifiers"),
    (0x37, "Numpad *", "Numpad"),
    (0x38, "Left Alt", "Modifiers"),
    (0x39, "Spacebar", "Nav"),
    (0x3A, "Caps Lock", "Modifiers"),
    (0x3B, "F1", "Function"),
    (0x3C, "F2", "Function"),
    (0x3D, "F3", "Function"),
    (0x3E, "F4", "Function"),
    (0x3F, "F5", "Function"),
    (0x40, "F6", "Function"),
    (0x41, "F7", "Function"),
    (0x42, "F8", "Function"),
    (0x43, "F9", "Function"),
    (0x44, "F10", "Function"),
    (0x45, "Num Lock", "Modifiers"),
    (0x46, "Scroll Lock", "Modifiers"),
    (0x4A, "Numpad -", "Numpad"),
    (0x4E, "Numpad +", "Numpad"),
    (0x57, "F11", "Function"),
    (0x58, "F12", "Function"),
    // The ISO keyboard's extra key beside the left Shift, and the numpad keys
    // that had no name: without them a press was reported as `Scan 86` and a
    // chord could not be built from it at all.
    (0x56, "ISO \\", "Letters"),
    (0x59, "Numpad =", "Numpad"),
    (0x73, "Intl Ro", "Letters"),
    (0x7D, "Intl Yen", "Letters"),
    (0x7E, "Numpad ,", "Numpad"),
];

/// Every key the UI knows, arrows first so they are easy to find.
pub fn all() -> Vec<&'static KeyInfo> {
    EXTENDED_KEYS.iter().chain(MAIN_KEYS.iter()).collect()
}

pub fn label(scan: u16) -> String {
    // Mouse buttons share this space so a key block can hold one: they come
    // from the same place and are read back the same way.
    if let Some(button) = crate::input::MouseButton::from_scan(scan) {
        return format!("{} click", button.label());
    }
    all()
        .iter()
        .find(|key| key.scan == scan)
        .map(|key| key.label.to_string())
        .unwrap_or_else(|| format!("Scan {scan}"))
}

pub fn is_extended(scan: u16) -> bool {
    if scan & EXTENDED_BIT != 0 {
        return true;
    }
    all()
        .iter()
        .find(|key| key.scan == scan)
        .map(|key| key.extended)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wasd_and_arrow_defaults_resolve() {
        assert_eq!(label(17), "W");
        assert_eq!(label(30), "A");
        assert_eq!(label(31), "S");
        assert_eq!(label(32), "D");
        assert_eq!(label(0x48), "↑");
        assert!(is_extended(0x48));
        assert!(!is_extended(0x11));
    }

    #[test]
    fn labels_are_unique() {
        let mut labels: Vec<String> = all().iter().map(|key| key.label.to_string()).collect();
        let total = labels.len();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), total);
    }

    /// The pickers key their options by scan value, so two rows sharing one is
    /// one row that cannot be chosen: Left Ctrl used to be exactly that.
    #[test]
    fn scan_codes_are_unique() {
        let mut scans: Vec<u16> = all().iter().map(|key| key.scan).collect();
        let total = scans.len();
        scans.sort();
        scans.dedup();
        assert_eq!(scans.len(), total);
        assert_eq!(label(0x1D), "Left Ctrl");
        assert_eq!(label(0x11D), "Right Ctrl");
        assert_eq!(label(0x38), "Left Alt");
        assert_eq!(label(0x138), "Right Alt");
    }

    /// Every key the picker can offer has to come back the same way from the
    /// hook: the extended bit it stores is the one `split` sends.
    #[test]
    fn the_extra_keys_split_and_normalize_like_the_picker_stores_them() {
        assert_eq!(label(EXTENDED_BIT | 0x1C), "Numpad Enter");
        assert_eq!(label(EXTENDED_BIT | 0x5B), "Left Windows");
        assert_eq!(label(EXTENDED_BIT | 0x37), "Print Screen");
        assert_eq!(label(0x56), "ISO \\");
        assert_eq!(split(EXTENDED_BIT | 0x1C), (0x1C, true));
        assert_eq!(split(EXTENDED_BIT | 0x35), (0x35, true));
        assert_eq!(split(EXTENDED_BIT | 0x37), (0x37, true));
        assert_eq!(split(0x56), (0x56, false));
        assert_eq!(normalize(0x1C, true), EXTENDED_BIT | 0x1C);
        assert_eq!(normalize(0x5B, true), EXTENDED_BIT | 0x5B);
        assert_eq!(normalize(0x1C, false), 0x1C);
        assert_eq!(normalize(0x56, true), 0x56);
    }

    #[test]
    fn the_two_sides_of_modifiers_split_back_to_their_own_codes() {
        assert_eq!(split(0x1D), (0x1D, false), "left ctrl is not extended");
        assert_eq!(split(0x11D), (0x1D, true), "right ctrl is");
        assert_eq!(split(0x38), (0x38, false));
        assert_eq!(split(0x138), (0x38, true));
        // Arrows keep the shape they always had.
        assert_eq!(split(0x48), (0x48, true));
        assert_eq!(normalize(0x48, true), 0x48);
        assert_eq!(normalize(0x1D, true), 0x11D);
        assert_eq!(normalize(0x1D, false), 0x1D);
    }
}
