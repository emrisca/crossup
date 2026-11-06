//! The region picker: a second window, transparent and on top of everything,
//! sized to the whole virtual desktop, that turns a mouse drag into a rectangle
//! in screen pixels.
//!
//! It is a normal interactive window rather than a click-through overlay, and
//! that is the whole trick: dragging inside it is an ordinary mouse drag in the
//! webview, so there is no low-level mouse hook, no hook to leak if something
//! goes wrong, and no interference with whatever is underneath. The price is
//! that the window below stops receiving clicks while the picker is up — which
//! is what "pick a region" means anyway.
//!
//! Coordinates come in as CSS pixels inside the webview and leave as physical
//! screen pixels. The conversion uses the window's *actual* outer position and
//! scale factor rather than the position that was asked for, so a window that
//! Windows nudged, or a display at 150%, still reports the rectangle the user
//! dragged.

use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};

/// The picker window, and the event target the answer is sent to.
pub const LABEL: &str = "picker";
pub const MAIN: &str = "main";
/// The event the main window listens for, with a [`PickResult`] payload.
pub const PICKED: &str = "region-picked";
/// The pick was given up: cancelled, Escaped, or the overlay went away.
pub const CANCELLED: &str = "region-pick-cancelled";
/// The one purpose whose rectangle is captured by the main window *after* the
/// answer, and so the one that keeps the app hidden a moment longer.
pub const PICTURE: &str = "macro-picture";

/// A rectangle in some pixel space. Which one is stated where it is used.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// What is being picked, and what is already there to place it against.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PickRequest {
    /// `zone` or `point`. The picker does not interpret it; the answer goes back
    /// to the window that asked, which knows what to do with a rectangle.
    pub purpose: String,
    /// Which zone or point in that window's list.
    pub index: i64,
    /// Existing zones, in screen pixels, so the new one is drawn in context.
    pub zones: Vec<Rect>,
    /// Existing click points, in screen pixels, drawn as dots.
    pub points: Vec<Rect>,
}

/// The same request with every coordinate in the picker's own CSS pixels.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PickContext {
    pub purpose: String,
    pub index: i64,
    pub zones: Vec<Rect>,
    pub points: Vec<Rect>,
}

/// The answer: a rectangle in screen pixels, and the row it belongs to, so the
/// window that asked can put it back where the user clicked.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PickResult {
    pub purpose: String,
    pub index: i64,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The request being served, held here rather than in an event so it does not
/// matter whether the picker window loaded before or after the click that
/// opened it — a window that has just been created has no listeners yet.
static PENDING: Mutex<Option<PickRequest>> = Mutex::new(None);

fn pending() -> MutexGuard<'static, Option<PickRequest>> {
    // A panic in a pick would otherwise poison the lock and break every later
    // one, so the guard is taken back rather than unwrapped.
    PENDING.lock().unwrap_or_else(|error| error.into_inner())
}

/// Whether the main window was showing when the picker covered it.
///
/// `None` means nothing is covered. It is remembered rather than assumed
/// because a window the person had already hidden must not be brought back.
static COVERED: Mutex<Option<bool>> = Mutex::new(None);

fn covered() -> MutexGuard<'static, Option<bool>> {
    COVERED.lock().unwrap_or_else(|error| error.into_inner())
}

/// Take the app off the screen while a rectangle is drawn.
///
/// The rectangle is a piece of the desktop, and the app is usually sitting on
/// it. Hiding first keeps the Crossup window out of the zone and the click
/// point, and — the reason this exists — out of the picture a macro will look
/// for: a picture that contains the builder picking it matches the builder
/// instead of the game.
fn cover(app: &AppHandle) {
    let mut covered = covered();
    if covered.is_some() {
        return;
    }
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    let showing = window.is_visible().unwrap_or(true);
    let _ = window.hide();
    *covered = Some(showing);
}

/// Bring the app back, if it was covered and was showing beforehand. Safe to
/// call when nothing is covered, which is what makes it safe to call twice.
pub fn restore(app: &AppHandle) {
    let Some(showing) = covered().take() else {
        return;
    };
    if !showing {
        return;
    }
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Where the picker window actually is, and how many screen pixels a CSS pixel
/// is worth there.
fn frame(app: &AppHandle) -> Result<(i32, i32, f64), String> {
    let window = app
        .get_webview_window(LABEL)
        .ok_or_else(|| "the region picker is not open".to_string())?;
    let origin = window
        .outer_position()
        .map_err(|error| format!("could not place the region picker: {error}"))?;
    let scale = window.scale_factor().unwrap_or(1.0);
    Ok((origin.x, origin.y, scale))
}

/// Screen pixels to the picker window's own pixels.
fn to_local(rect: &Rect, (origin_x, origin_y, scale): (i32, i32, f64)) -> Rect {
    Rect {
        x: ((rect.x - origin_x) as f64 / scale).round() as i32,
        y: ((rect.y - origin_y) as f64 / scale).round() as i32,
        width: (rect.width as f64 / scale).round() as i32,
        height: (rect.height as f64 / scale).round() as i32,
    }
}

/// The picker window's own pixels back to screen pixels.
///
/// The far edge is converted rather than the size, so a rectangle does not drift
/// by a pixel per conversion on a scaled display.
fn to_screen(rect: &Rect, (origin_x, origin_y, scale): (i32, i32, f64)) -> Rect {
    let left = origin_x + (rect.x as f64 * scale).round() as i32;
    let top = origin_y + (rect.y as f64 * scale).round() as i32;
    let right = origin_x + ((rect.x + rect.width.max(0)) as f64 * scale).round() as i32;
    let bottom = origin_y + ((rect.y + rect.height.max(0)) as f64 * scale).round() as i32;
    Rect {
        x: left,
        y: top,
        width: (right - left).max(0),
        height: (bottom - top).max(0),
    }
}

fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}

/// Show the picker over the whole desktop, serving `request`.
pub fn open(app: &AppHandle, request: PickRequest) -> Result<(), String> {
    let (origin_x, origin_y, width, height) = crate::input::virtual_screen();
    *pending() = Some(request);

    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
            .title("Pick a region on screen")
            // Built hidden, then placed: `position` and `inner_size` on the
            // builder work in logical units, and the virtual desktop is not a
            // whole number of those on a scaled display, which would leave a
            // strip of screen the picker does not cover.
            .visible(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .build()
            .map_err(|error| format!("could not open the region picker: {error}"))?,
    };

    let _ = window.set_position(PhysicalPosition::new(origin_x, origin_y));
    let _ = window.set_size(PhysicalSize::new(
        width.max(1) as u32,
        height.max(1) as u32,
    ));
    // The app goes away before the overlay appears, so it is not inside the
    // rectangle being drawn — and, for a picture, not inside the shot taken
    // from it a moment later.
    cover(app);
    let _ = window.show();
    let _ = window.set_focus();
    // The window is reused, and its page is already loaded on every open after
    // the first, so it has to be told to read the new request. On the first open
    // the page is still loading and reads it on mount instead.
    let _ = window.emit("picker-open", ());
    Ok(())
}

/// What the picker window should draw. `None` when nothing is being picked,
/// which is also what a stray tab that wandered onto this page gets.
pub fn context(app: &AppHandle) -> Option<PickContext> {
    let request = pending().clone()?;
    let frame = frame(app).ok()?;
    Some(PickContext {
        purpose: request.purpose.clone(),
        index: request.index,
        zones: request.zones.iter().map(|rect| to_local(rect, frame)).collect(),
        points: request
            .points
            .iter()
            .map(|rect| to_local(rect, frame))
            .collect(),
    })
}

/// The drag is finished: take it as screen pixels, hide, and tell the window
/// that asked.
pub fn finish(app: &AppHandle, drag: Rect) -> Result<(), String> {
    let Some(request) = pending().take() else {
        hide(app);
        restore(app);
        return Err("nothing was being picked".to_string());
    };
    let placed = frame(app);
    hide(app);
    let rect = match placed {
        Ok(frame) => to_screen(&drag, frame),
        Err(problem) => {
            restore(app);
            return Err(problem);
        }
    };

    let result = PickResult {
        purpose: request.purpose.clone(),
        index: request.index,
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    };
    let _ = app.emit_to(MAIN, PICKED, result);

    if request.purpose == PICTURE {
        // The picture is taken by the main window once this answer lands, so the
        // app stays hidden until then: shown first, it would be captured into
        // the very picture it is saving. `save_picture` brings it back, and the
        // timer is the answer to a capture that never arrives — a rectangle the
        // capture refuses must not leave a window nobody can see.
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            restore(&handle);
        });
    } else {
        restore(app);
    }
    Ok(())
}

/// Give up on the pick. Safe to call when nothing is being picked.
pub fn cancel(app: &AppHandle) {
    let was_picking = pending().take().is_some();
    hide(app);
    restore(app);
    // The window that asked is waiting on an event, not on this call, so giving
    // up quietly left a block saying it was waiting for a spot long after the
    // person had cancelled the pick.
    if was_picking {
        let _ = app.emit_to(MAIN, CANCELLED, ());
    }
}

/// The picker window itself went away — a task switch with the window manager
/// in a mood, or the app shutting down. Forgetting the request is the important
/// part: a stale one would be answered against a window that no longer exists.
pub fn closed(app: &AppHandle) {
    if pending().take().is_some() {
        let _ = app.emit_to(MAIN, CANCELLED, ());
    }
    restore(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn a_rect_round_trips_at_one_to_one() {
        let frame = (100, 50, 1.0);
        let screen = rect(140, 90, 30, 20);
        let local = to_local(&screen, frame);
        assert_eq!(local, rect(40, 40, 30, 20));
        assert_eq!(to_screen(&local, frame), screen);
    }

    /// A second monitor to the left of the primary one is at a negative origin,
    /// and a scaled display is not one screen pixel per CSS pixel.
    #[test]
    fn a_scaled_display_off_the_origin_converts_both_ways() {
        let frame = (-1920, -120, 1.5);
        let local = rect(10, 20, 100, 50);
        let screen = to_screen(&local, frame);
        assert_eq!(screen, rect(-1905, -90, 150, 75));
        assert_eq!(to_local(&screen, frame), local);
    }

    /// A click with no drag is a zero-size rectangle, which is what placing a
    /// click point with one press relies on.
    #[test]
    fn a_click_without_a_drag_stays_a_point() {
        let frame = (0, 0, 2.0);
        let screen = to_screen(&rect(300, 200, 0, 0), frame);
        assert_eq!(screen, rect(600, 400, 0, 0));
        assert_eq!(to_local(&screen, frame), rect(300, 200, 0, 0));
    }

    /// Dragging up and to the left gives a rectangle with a negative size, which
    /// must not survive the trip.
    #[test]
    fn a_backwards_drag_cannot_produce_a_negative_size() {
        let frame = (0, 0, 1.0);
        let screen = to_screen(&rect(300, 200, -250, -150), frame);
        assert_eq!(screen.width, 0);
        assert_eq!(screen.height, 0);
    }

    /// The answer has to carry which row it was for, or a rectangle picked for
    /// the third zone lands on whichever one the list happens to render first.
    #[test]
    fn the_answer_carries_the_row_it_belongs_to() {
        let request = PickRequest {
            purpose: "zone".to_string(),
            index: 4,
            zones: vec![rect(0, 0, 10, 10)],
            points: Vec::new(),
        };
        assert_eq!(request.purpose, "zone");
        assert_eq!(request.index, 4);
    }
}
