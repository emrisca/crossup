//! Screenshots a macro saves as it runs.
//!
//! A different job from the pictures a macro looks for: those live in
//! `pictures/` under a name nobody sees, and exist to be matched against the
//! screen. These are files a person opens afterwards, so they are named,
//! timestamped, and kept together in their own folder beside the settings.

use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use crate::picker::Rect;

/// Longest name a screenshot may carry, before the timestamp is added.
const MAX_NAME: usize = 60;

fn folder(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("could not find the screenshots folder: {error}"))?
        .join("screenshots");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not make the screenshots folder: {error}"))?;
    Ok(dir)
}

/// Capture `rect` and write it as `screenshots/<name>-<stamp>.png`.
pub fn save(
    app: &AppHandle,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    name: &str,
) -> Result<PathBuf, String> {
    if width <= 0 || height <= 0 {
        return Err("a screenshot needs an area with some size to it".to_string());
    }
    if width > 8_000 || height > 8_000 {
        return Err("that area is bigger than any screen".to_string());
    }
    let (png, _, _) = crate::pictures::capture_png(Rect {
        x,
        y,
        width,
        height,
    })?;
    let file = folder(app)?.join(file_name(name, &stamp()));
    std::fs::write(&file, png)
        .map_err(|error| format!("could not save the screenshot: {error}"))?;
    Ok(file)
}

/// The name a screenshot is written under: what the person called it, the
/// moment it was taken, and nothing Windows would refuse as a file name.
fn file_name(name: &str, stamp: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .take(MAX_NAME)
        .map(|letter| {
            if letter.is_alphanumeric() || letter == '-' || letter == '_' {
                letter
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('-');
    let base = if cleaned.is_empty() { "shot" } else { cleaned };
    format!("{base}-{stamp}.png")
}

/// `YYYYMMDD-HHMMSS`, so a folder of runs sorts in the order they happened.
fn stamp() -> String {
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_becomes_something_windows_will_accept() {
        assert_eq!(
            file_name("after the boss fight", "20261105-093000"),
            "after-the-boss-fight-20261105-093000.png"
        );
        // A path is not a name: separators become dashes rather than folders.
        assert_eq!(
            file_name("..\\..\\system32", "20261105-093000"),
            "system32-20261105-093000.png"
        );
    }

    #[test]
    fn an_unnamed_screenshot_still_gets_a_name() {
        assert_eq!(file_name("   ", "20261105-093000"), "shot-20261105-093000.png");
        assert_eq!(file_name("", "20261105-093000"), "shot-20261105-093000.png");
    }

    #[test]
    fn a_very_long_name_is_cut_to_something_a_person_can_read() {
        let name = file_name(&"x".repeat(200), "20261105-093000");
        assert!(name.starts_with(&"x".repeat(MAX_NAME)));
        assert!(name.ends_with(".png"));
    }
}
