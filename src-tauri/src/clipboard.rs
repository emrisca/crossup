//! Putting text on the clipboard.
//!
//! One direction only: a macro pastes what it has just copied, and with no
//! variables in the engine there is nowhere for a *read* of the clipboard to go.
//! Reading it would also mean holding the clipboard open while a later block
//! runs, which is the shortest route to a machine where Ctrl+V stops working.
//!
//! The clipboard is a single machine-wide resource and any program may be
//! sitting on it for the few milliseconds it takes to paste, so opening it is
//! retried a handful of times rather than reported as a failure on the first
//! refusal.

use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

/// How many times to try for the clipboard before giving up on it.
const ATTEMPTS: u32 = 5;
/// The pause between attempts. Long enough for a program in the middle of a
/// paste to finish, short enough that a macro does not visibly stall.
const RETRY_MS: u64 = 50;

/// `CF_UNICODETEXT`. A fixed number since the clipboard was introduced, and
/// naming it here avoids pulling in the whole of OLE for one constant.
const CF_UNICODETEXT: u32 = 13;

/// Put `text` on the clipboard, replacing whatever was there.
///
/// An empty string is allowed and means "clear it", which is occasionally
/// exactly what a macro wants before pasting into a field that already has
/// something in it.
pub fn set_text(text: &str) -> Result<(), String> {
    let encoded = wide(text);
    let bytes = encoded.len() * std::mem::size_of::<u16>();

    let mut last = String::new();
    for attempt in 0..ATTEMPTS {
        match attempt_once(&encoded, bytes) {
            Ok(()) => return Ok(()),
            Err(why) => last = why,
        }
        if attempt + 1 < ATTEMPTS {
            std::thread::sleep(Duration::from_millis(RETRY_MS));
        }
    }
    Err(last)
}

fn attempt_once(encoded: &[u16], bytes: usize) -> Result<(), String> {
    unsafe {
        if OpenClipboard(None).is_err() {
            return Err("another program is holding the clipboard".to_string());
        }
        // The clipboard must be closed on every path out of here, including the
        // failures: a clipboard left open by this process makes every other
        // program's copy and paste fail until Crossup exits.
        let outcome = fill(encoded, bytes);
        let _ = CloseClipboard();
        outcome
    }
}

unsafe fn fill(encoded: &[u16], bytes: usize) -> Result<(), String> {
    if EmptyClipboard().is_err() {
        return Err("the clipboard would not let go of what was on it".to_string());
    }
    let block = GlobalAlloc(GMEM_MOVEABLE, bytes)
        .map_err(|_| "there was no room to copy that much text".to_string())?;
    let target = GlobalLock(block);
    if target.is_null() {
        release(block);
        return Err("the text could not be reached once room was made for it".to_string());
    }
    std::ptr::copy_nonoverlapping(encoded.as_ptr(), target as *mut u16, encoded.len());
    let _ = GlobalUnlock(block);
    // Once SetClipboardData succeeds the memory belongs to the clipboard and is
    // not ours to free; before that it is still ours to clean up.
    if SetClipboardData(CF_UNICODETEXT, HANDLE(block.0)).is_err() {
        release(block);
        return Err("the clipboard refused the text".to_string());
    }
    Ok(())
}

/// Give the block back when the clipboard did not take it.
///
/// Once `SetClipboardData` succeeds the memory belongs to the clipboard, so this
/// is only for the paths where it never got that far.
fn release(block: HGLOBAL) {
    unsafe {
        let _ = GlobalFree(block);
    }
}

/// The text as the clipboard expects it: UTF-16 with a terminating zero, even
/// when there is nothing to say.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_still_a_terminated_string() {
        assert_eq!(wide(""), vec![0]);
    }

    #[test]
    fn text_is_encoded_as_utf16_with_exactly_one_terminator() {
        let encoded = wide("héllo");
        assert_eq!(encoded.last(), Some(&0));
        assert_eq!(encoded.iter().filter(|unit| **unit == 0).count(), 1);
        assert_eq!(
            String::from_utf16_lossy(&encoded[..encoded.len() - 1]),
            "héllo"
        );
    }

    #[test]
    fn a_character_outside_the_basic_plane_becomes_a_pair() {
        // A game controller, U+1F3AE: not in the basic plane, so it needs two
        // UTF-16 units — the case a plain byte copy would corrupt.
        let encoded = wide("🎮");
        assert_eq!(encoded.len(), 3);
        assert_eq!(String::from_utf16_lossy(&encoded[..2]), "🎮");
    }
}
