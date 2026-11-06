//! The pictures a macro can look for.
//!
//! A picture lives next to the settings file, as a PNG under `pictures/`, and a
//! block refers to it by file name. Keeping the pixels out of `settings.json`
//! matters: that file is rewritten as a macro is edited, so a few screenshots
//! inside it would turn every keystroke into a megabyte write, and a copy of the
//! settings would carry images nobody asked to back up.
//!
//! Capturing and matching are both here rather than in the macro engine, so the
//! engine keeps talking to a trait and stays testable without a screen.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::picker::Rect;
use crate::vision::{self, Gray};

/// Shortest side a picture may have. A handful of pixels matches everywhere.
pub const MIN_SIDE: u32 = 8;
/// Longest side, so one pick cannot fill the disk with a screenshot.
pub const MAX_SIDE: u32 = 2000;

/// A saved picture, as the window needs it back.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Picture {
    pub id: String,
    pub width: u32,
    pub height: u32,
}

fn folder(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|error| format!("could not find the settings folder: {error}"))?
        .join("pictures");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("could not make the pictures folder: {error}"))?;
    Ok(dir)
}

/// A name this app could have written. Anything with a slash, a dot or a
/// parent-directory hop is refused rather than joined onto a path.
fn checked(id: &str) -> Result<&str, String> {
    let fine = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || letter == '-' || letter == '_');
    if fine {
        Ok(id)
    } else {
        Err("that is not a picture this app saved".to_string())
    }
}

fn path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(folder(app)?.join(format!("{}.png", checked(id)?)))
}

static NEXT: AtomicU64 = AtomicU64::new(0);

fn next_id() -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("pic-{stamp}-{n}")
}

/// Save PNG bytes and hand back the name to store in the macro.
pub fn save(app: &AppHandle, png: &[u8]) -> Result<String, String> {
    let id = next_id();
    std::fs::write(path(app, &id)?, png)
        .map_err(|error| format!("could not save the picture: {error}"))?;
    Ok(id)
}

pub fn read(app: &AppHandle, id: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path(app, id)?)
        .map_err(|_| "that picture is missing from disk — pick it again".to_string())
}

/// Most pictures a gallery will offer at once.
pub const MAX_HISTORY: usize = 24;

/// The pictures already saved, newest first.
///
/// A macro that looks for the same thing in several blocks — wait for it, click
/// it, loop while it is there — used to mean dragging the same rectangle out of
/// the screen again for each block. The folder already holds every picture a
/// block has ever pointed at, so it can be read back and chosen from instead.
pub fn history(app: &AppHandle) -> Vec<Picture> {
    let Ok(dir) = folder(app) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, Picture)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ending| ending.to_str()) != Some("png") {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            // A name this app could not have written is not offered back to it.
            checked(&id).ok()?;
            let time = entry.metadata().ok()?.modified().ok()?;
            let (width, height) = size_of(&path)?;
            Some((time, Picture { id, width, height }))
        })
        .collect();
    // Newest first: the one just dragged out is the one most likely wanted.
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found
        .into_iter()
        .map(|(_, picture)| picture)
        .take(MAX_HISTORY)
        .collect()
}

/// Width and height straight out of the PNG header, without decoding the image.
fn size_of(path: &std::path::Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut head = [0u8; 24];
    let mut file = std::fs::File::open(path).ok()?;
    file.read_exact(&mut head).ok()?;
    size_from_header(&head)
}

fn size_from_header(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 24 || &head[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let width = u32::from_be_bytes(head[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(head[20..24].try_into().ok()?);
    Some((width, height))
}

pub fn exists(app: &AppHandle, id: &str) -> bool {
    path(app, id).map(|file| file.exists()).unwrap_or(false)
}

pub fn remove(app: &AppHandle, id: &str) -> Result<(), String> {
    let file = path(app, id)?;
    if file.exists() {
        std::fs::remove_file(&file)
            .map_err(|error| format!("could not delete the picture: {error}"))?;
    }
    Ok(())
}

/// The picture as a data URL, for the thumbnail beside its block.
pub fn preview(app: &AppHandle, id: &str) -> Result<String, String> {
    let bytes = read(app, id)?;
    Ok(format!("data:image/png;base64,{}", base64(&bytes)))
}

/// Take the rectangle the person dragged as a new picture.
pub fn capture(app: &AppHandle, rect: Rect) -> Result<Picture, String> {
    check_size(rect.width.max(0) as u32, rect.height.max(0) as u32)?;
    let (png, width, height) = capture_png(rect)?;
    let id = save(app, &png)?;
    Ok(Picture { id, width, height })
}

/// Capture a rectangle as PNG bytes, for anything that writes its own file
/// rather than keeping a picture for a block to look for.
///
/// No size rules here: a screenshot may be a whole screen, which is exactly
/// what a picture may not be.
pub fn capture_png(rect: Rect) -> Result<(Vec<u8>, u32, u32), String> {
    let width = rect.width.max(0) as u32;
    let height = rect.height.max(0) as u32;
    let shot = grab(rect.x, rect.y, width, height)?;
    let png = encode(&shot)?;
    Ok((png, shot.width, shot.height))
}

/// Whether a dragged area is a picture worth keeping.
///
/// A few pixels of screen match almost anywhere, and a whole screen is a
/// screenshot rather than something to look for, so both ends are refused with
/// the size that would have been right.
fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width < MIN_SIDE || height < MIN_SIDE {
        return Err(format!(
            "drag a bigger area — a picture needs at least {MIN_SIDE} × {MIN_SIDE} pixels"
        ));
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!(
            "drag a smaller area — a picture may be at most {MAX_SIDE} × {MAX_SIDE} pixels"
        ));
    }
    Ok(())
}

/// Where the picture is now, as the centre of the match, in screen pixels.
///
/// Every monitor is searched in turn, so a picture may be found on the second
/// screen as readily as the first.
pub fn locate(app: &AppHandle, id: &str, similarity: u32) -> Option<(i32, i32)> {
    let needle = read_gray(app, id).ok()?;
    let wanted = similarity.clamp(1, 100) as f32 / 100.0;
    let screens = screenshots::Screen::all().ok()?;
    for screen in screens {
        let origin_x = screen.display_info.x;
        let origin_y = screen.display_info.y;
        let width = screen.display_info.width;
        let height = screen.display_info.height;
        let Ok(shot) = screen.capture_area(0, 0, width, height) else {
            continue;
        };
        let (shot_width, shot_height) = shot.dimensions();
        let raw = shot.into_raw();
        let Some(gray) = Gray::from_bytes(shot_width, shot_height, &raw, 4) else {
            continue;
        };
        if let Some(hit) = vision::find(&gray, &needle, wanted) {
            return Some((
                origin_x + hit.x + needle.width as i32 / 2,
                origin_y + hit.y + needle.height as i32 / 2,
            ));
        }
    }
    None
}

/// One captured rectangle, as RGB.
struct Shot {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

/// Capture a rectangle of a screen.
fn grab(x: i32, y: i32, width: u32, height: u32) -> Result<Shot, String> {
    let screens = screenshots::Screen::all()
        .map_err(|error| format!("could not read the screens: {error}"))?;
    let screen = screens
        .into_iter()
        .find(|screen| {
            let info = &screen.display_info;
            x >= info.x
                && y >= info.y
                && x < info.x + info.width as i32
                && y < info.y + info.height as i32
        })
        .ok_or_else(|| "that area is not on a screen".to_string())?;
    let (origin_x, origin_y) = (screen.display_info.x, screen.display_info.y);
    let shot = screen
        .capture_area(x - origin_x, y - origin_y, width, height)
        .map_err(|error| format!("could not capture the screen: {error}"))?;
    let (width, height) = shot.dimensions();
    let raw = shot.into_raw();
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for pixel in raw.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
    }
    Ok(Shot {
        width,
        height,
        rgb,
    })
}

fn encode(shot: &Shot) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, shot.width, shot.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("could not write the picture: {error}"))?;
        writer
            .write_image_data(&shot.rgb)
            .map_err(|error| format!("could not write the picture: {error}"))?;
    }
    Ok(out)
}

fn read_gray(app: &AppHandle, id: &str) -> Result<Gray, String> {
    let bytes = read(app, id)?;
    let mut reader = png::Decoder::new(Cursor::new(bytes))
        .read_info()
        .map_err(|error| format!("that picture cannot be read: {error}"))?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("that picture cannot be read: {error}"))?;
    buffer.truncate(info.buffer_size());
    let stride = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        _ => return Err("that picture is in a form this app cannot compare".to_string()),
    };
    Gray::from_bytes(info.width, info.height, &buffer, stride)
        .ok_or_else(|| "that picture is empty".to_string())
}

/// Base64, so the window can show a thumbnail without a second file request.
fn base64(data: &[u8]) -> String {
    const LETTERS: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let bytes = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let packed = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        out.push(LETTERS[(packed >> 18) as usize & 63] as char);
        out.push(LETTERS[(packed >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            LETTERS[(packed >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            LETTERS[packed as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_pads_the_way_images_expect() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// The name in a macro comes from the settings file, which is a file: a
    /// name that walks out of the pictures folder must not be joined to a path.
    #[test]
    fn a_name_with_a_path_in_it_is_refused() {
        assert!(checked("pic-1234-0").is_ok());
        assert!(checked("../settings").is_err());
        assert!(checked("pictures/pic").is_err());
        assert!(checked("").is_err());
        assert!(checked("pic.png").is_err());
    }

    /// A screenshot round-trips through the encoder and the reader unchanged,
    /// which is what makes a picture match itself exactly.
    #[test]
    fn a_captured_picture_reads_back_as_the_pixels_that_went_in() {
        let shot = Shot {
            width: 4,
            height: 2,
            rgb: vec![
                10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170, 180,
                190, 200, 210, 220, 230, 240,
            ],
        };
        let png = encode(&shot).expect("the picture encodes");
        let mut reader = png::Decoder::new(Cursor::new(png))
            .read_info()
            .expect("the picture reads");
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).expect("the frame reads");
        buffer.truncate(info.buffer_size());
        assert_eq!((info.width, info.height), (4, 2));
        assert_eq!(&buffer[..], &shot.rgb[..]);
    }

    /// The gallery reads a picture's size out of its header rather than decoding
    /// it: a folder of them has to be listed without decoding every image.
    #[test]
    fn a_pictures_size_comes_from_its_header() {
        let shot = Shot {
            width: 7,
            height: 3,
            rgb: vec![0; 7 * 3 * 3],
        };
        let png = encode(&shot).expect("the picture encodes");
        assert_eq!(size_from_header(&png), Some((7, 3)));
        assert_eq!(size_from_header(b"not a png at all, not even close"), None);
        assert_eq!(size_from_header(&png[..12]), None);
    }

    /// A few pixels match everywhere and a whole screen is a screenshot, so both
    /// ends are refused before anything is captured.
    #[test]
    fn a_picture_that_is_too_small_or_too_large_is_refused() {
        assert!(check_size(3, 40).is_err());
        assert!(check_size(40, 7).is_err());
        assert!(check_size(MAX_SIDE + 1, 40).is_err());
        assert!(check_size(MIN_SIDE, MIN_SIDE).is_ok());
        assert!(check_size(64, 40).is_ok());
    }
}
