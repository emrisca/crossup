//! The replayer: record what the person does, then do it again.
//!
//! The recorder is fed by the two hooks that are already installed — the
//! keyboard one and the mouse one — rather than installing its own. A second
//! `WH_KEYBOARD_LL` hook would double up with the first the moment both ran, and
//! the hooks already know how to tell the person's input from the app's own
//! `SendInput` output, which is exactly the distinction that keeps a replay from
//! recording itself.
//!
//! Three rules shape the rest:
//!
//! 1. **The shortcut is not an action.** While recording, the keys and buttons
//!    that make up either replayer shortcut are dropped rather than written
//!    down, so a recording that ends by pressing the chord replays as it was
//!    recorded instead of replaying its own stop.
//! 2. **The window gets out of the way.** Starting a recording hides Crossup:
//!    anything it draws would otherwise be in the way of the program being
//!    recorded. The shortcut brings it back.
//! 3. **A replay is one more thing that drives the desktop.** It claims the same
//!    run slot as the clicker, the typer and the macros, so two of them can
//!    never fight over the cursor. When a macro asks for one, the macro already
//!    holds that slot and the replay plays inside it — the driver below never
//!    claims, so a nested replay is not refused by its own caller.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::config::Settings;
use crate::hook::SharedState;
use crate::input::{self, MouseButton};
use crate::keys;
use crate::runtime::RuntimeState;

/// The name a replay is reported under in `activity`.
pub const REPLAY: &str = "replay";

/// The shortcuts a fresh replayer comes up with. Both used to start empty, so
/// the tab's fields read "not set" and the keys did nothing until somebody went
/// looking for a feature that had never been wired up.
pub const DEFAULT_RECORD_HOTKEY: &str = "Control+Alt+R";
pub const DEFAULT_REPLAY_HOTKEY: &str = "Control+Alt+P";

/// The most events one recording may hold. At roughly sixty mouse moves a
/// second this is about fifteen minutes of continuous movement; past it the run
/// is left as it is rather than filling the settings file.
const MAX_EVENTS: usize = 60_000;
/// Mouse moves this close together, in time and in distance, are the same
/// movement: writing every one would triple the file for nothing.
const MOVE_GAP_MS: u128 = 12;
const MOVE_GAP_PX: i32 = 3;
/// The beat between passes of a looped replay, so the last action and the first
/// one of the next pass are not the same instant.
const BETWEEN_PASSES_MS: u64 = 250;
/// How far from the recorded pace a replay may go: a quarter speed to four
/// times. Past either end the gaps stop reading as a person doing something.
const MIN_SPEED: f64 = 0.25;
const MAX_SPEED: f64 = 4.0;

// ── what a recording holds ─────────────────────────────────────────

/// One captured action, with the time it happened at in milliseconds from the
/// start of the recording.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    Move { t: u32, x: i32, y: i32 },
    Button { t: u32, x: i32, y: i32, button: String, down: bool },
    Wheel { t: u32, x: i32, y: i32, amount: i32 },
    Key { t: u32, scan: u16, down: bool },
}

fn event_time(event: &Event) -> u32 {
    match event {
        Event::Move { t, .. }
        | Event::Button { t, .. }
        | Event::Wheel { t, .. }
        | Event::Key { t, .. } => *t,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub id: String,
    pub name: String,
    pub events: Vec<Event>,
    /// The last event's time: a recording's length is what it does, not how long
    /// it sat still after the last thing.
    pub duration_ms: u32,
}

/// A recording as the list shows it: no events, because the window never needs
/// them and a long recording is not something to send over the bridge.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub events: usize,
    pub duration_ms: u32,
}

fn summary_of(recording: &Recording) -> Summary {
    Summary {
        id: recording.id.clone(),
        name: recording.name.clone(),
        events: recording.events.len(),
        duration_ms: recording.duration_ms,
    }
}

/// Everything the replayer keeps in the settings file.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct ReplaySettings {
    pub recordings: Vec<Recording>,
    /// The recording the Play button and the replay shortcut use.
    pub selected: String,
    /// The one key that starts a recording and ends it again.
    pub record_hotkey: String,
    /// The one key that starts a replay and stops it again.
    pub replay_hotkey: String,
    /// How many passes a replay makes. `None` is forever.
    pub repeat: Option<u32>,
    /// Playback speed as a multiple of the recorded pace: 1 is as recorded, 2 is
    /// twice as fast, 0.5 is half.
    pub speed: f64,
}

impl ReplaySettings {
    /// The speed a run uses, kept inside what still replays usefully.
    fn speed(&self) -> f64 {
        clamp_speed(self.speed)
    }
}

/// Keep a speed inside the range the engine accepts.
pub fn clamp_speed(speed: f64) -> f64 {
    if speed.is_finite() {
        speed.clamp(MIN_SPEED, MAX_SPEED)
    } else {
        1.0
    }
}

impl Default for ReplaySettings {
    fn default() -> Self {
        Self {
            recordings: Vec::new(),
            selected: String::new(),
            record_hotkey: DEFAULT_RECORD_HOTKEY.to_string(),
            replay_hotkey: DEFAULT_REPLAY_HOTKEY.to_string(),
            repeat: None,
            speed: 1.0,
        }
    }
}

// ── live state ─────────────────────────────────────────────────────

/// The recording in progress. Held only while one is running.
struct Draft {
    started: Instant,
    events: Vec<Event>,
    last_move: Instant,
    last_point: (i32, i32),
    capped: bool,
    /// Keys and buttons that are a shortcut rather than an action.
    ignore_keys: Vec<u16>,
    ignore_buttons: Vec<String>,
}

impl Default for Draft {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            started: now,
            events: Vec::new(),
            last_move: now,
            last_point: (i32::MIN, i32::MIN),
            capped: false,
            ignore_keys: Vec::new(),
            ignore_buttons: Vec::new(),
        }
    }
}

impl Draft {
    fn elapsed_ms(&self) -> u32 {
        self.started.elapsed().as_millis().min(u32::MAX as u128) as u32
    }
}

#[derive(Clone, Debug)]
struct Progress {
    /// `recording`, `replaying`, `done`, `stopped`, or empty for idle.
    status: String,
    played: u32,
    total: u32,
    pass: u32,
    passes: Option<u32>,
    /// The pace this run is going at, for the badge to report.
    speed: f64,
    note: Option<String>,
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            status: String::new(),
            played: 0,
            total: 0,
            pass: 0,
            passes: None,
            speed: 1.0,
            note: None,
        }
    }
}

#[derive(Default)]
struct Session {
    draft: Option<Draft>,
    /// Recorded but not yet saved: what the tab offers to name.
    pending: Option<Recording>,
    progress: Progress,
}

/// Module state rather than Tauri state, because the hooks that feed the
/// recorder are installed before any window exists and cannot reach `State`.
static STATE: Mutex<Option<Session>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Session) -> T) -> T {
    let mut guard: MutexGuard<'static, Option<Session>> =
        STATE.lock().unwrap_or_else(|error| error.into_inner());
    f(guard.get_or_insert_with(Session::default))
}

/// What the tab polls while it is open.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReplayState {
    pub recording: bool,
    /// Events captured so far, and how long the recording has been running.
    pub events: usize,
    pub duration_ms: u32,
    /// A finished recording waiting for a name.
    pub pending: Option<Summary>,
    pub playing: bool,
    /// `idle`, `recording`, `replaying`, `done` or `stopped`.
    pub status: String,
    pub played: u32,
    pub total: u32,
    pub pass: u32,
    pub passes: Option<u32>,
    /// The pace the run is going at: 1 is as recorded.
    pub speed: f64,
    /// Why something could not start, if it could not.
    pub note: Option<String>,
}

pub fn view() -> ReplayState {
    with(|state| {
        let draft = state.draft.as_ref();
        ReplayState {
            recording: draft.is_some(),
            events: draft.map(|draft| draft.events.len()).unwrap_or(0),
            duration_ms: draft.map(|draft| draft.elapsed_ms()).unwrap_or(0),
            pending: state.pending.as_ref().map(summary_of),
            playing: state.progress.status == "replaying",
            status: if state.progress.status.is_empty() {
                "idle".to_string()
            } else {
                state.progress.status.clone()
            },
            played: state.progress.played,
            total: state.progress.total,
            pass: state.progress.pass,
            passes: state.progress.passes,
            speed: state.progress.speed,
            note: state.progress.note.clone(),
        }
    })
}

fn note(message: String) {
    with(|state| state.progress.note = Some(message));
}

// ── recording ──────────────────────────────────────────────────────

/// Start recording, and hide the window.
pub fn begin(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let mut guard = STATE.lock().unwrap_or_else(|error| error.into_inner());
    let session = guard.get_or_insert_with(Session::default);
    if session.draft.is_some() {
        return Err("a recording is already running".to_string());
    }

    let mut draft = Draft::default();
    for accelerator in [
        settings.replay.record_hotkey.clone(),
        settings.replay.replay_hotkey.clone(),
    ] {
        draft.ignore_keys.extend(hotkey_scans(&accelerator));
        draft.ignore_buttons.extend(hotkey_buttons(&accelerator));
    }
    session.draft = Some(draft);
    // A fresh recording replaces whatever was waiting to be named: keeping both
    // would mean the Save button saved one of two things with no way to tell.
    session.pending = None;
    session.progress = Progress {
        status: "recording".to_string(),
        ..Progress::default()
    };
    drop(guard);

    // Out of the way: the window would otherwise be recorded, and is standing
    // in front of whatever is being recorded.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    Ok(())
}

/// Stop recording, keep what was captured, and bring the window back.
pub fn stop(app: &AppHandle) {
    with(|state| {
        let mut capped = false;
        if let Some(draft) = state.draft.take() {
            capped = draft.capped;
            if let Some(last) = draft.events.last() {
                let duration_ms = event_time(last);
                state.pending = Some(Recording {
                    id: String::new(),
                    name: String::new(),
                    events: draft.events,
                    duration_ms,
                });
            }
        }
        let mut progress = std::mem::take(&mut state.progress);
        progress.status = "stopped".to_string();
        if capped {
            progress.note = Some(
                "the recording reached its limit, so the end of it is missing".to_string(),
            );
        }
        state.progress = progress;
    });
    crate::reveal(app);
}

/// Take the finished recording out of the holder, for saving.
pub fn take_pending() -> Option<Recording> {
    with(|state| state.pending.take())
}

pub fn discard_pending() {
    with(|state| state.pending = None);
}

/// The record shortcut fired: end a recording, or start one.
pub fn record_shortcut(app: &AppHandle, runtime: &RuntimeState) {
    if with(|state| state.draft.is_some()) {
        stop(app);
        return;
    }
    let settings = {
        let state = app.state::<SharedState>();
        let shared = state.lock().unwrap();
        shared.settings.clone()
    };
    if !settings.replay_enabled {
        note("the replayer is switched off".to_string());
        return;
    }
    if let Some(running) = runtime.running() {
        note(format!("{running} is running — stop it before recording"));
        return;
    }
    if let Err(error) = begin(app, &settings) {
        note(error);
    }
}

// ── capturing ──────────────────────────────────────────────────────
//
// These are called from the hooks, so they must not panic, must not block, and
// must be cheap when nothing is being recorded: one lock and a `None` check.

pub fn capture_key(scan: u16, down: bool) {
    with(|state| {
        let Some(draft) = state.draft.as_mut() else {
            return;
        };
        // A key with no scan code cannot be pressed again: it is a keystroke
        // with nothing in it. Recording one makes a file that plays back
        // silence, which reads as a broken replayer rather than a broken event.
        if scan == 0 {
            return;
        }
        // The shortcut is not an action: this is what lets the same chord end
        // the recording and start the replay without either being replayed.
        if draft.ignore_keys.contains(&scan) {
            return;
        }
        if draft.events.len() >= MAX_EVENTS {
            draft.capped = true;
            return;
        }
        let t = draft.elapsed_ms();
        draft.events.push(Event::Key { t, scan, down });
    });
}

pub fn capture_move(x: i32, y: i32) {
    with(|state| {
        let Some(draft) = state.draft.as_mut() else {
            return;
        };
        let now = Instant::now();
        let moved = (x - draft.last_point.0).abs() + (y - draft.last_point.1).abs();
        // Mouse moves arrive hundreds of times a second; one this close to the
        // last saved one is the same movement.
        if moved < MOVE_GAP_PX && now.duration_since(draft.last_move).as_millis() < MOVE_GAP_MS {
            return;
        }
        draft.last_move = now;
        draft.last_point = (x, y);
        if draft.events.len() >= MAX_EVENTS {
            draft.capped = true;
            return;
        }
        let t = draft.elapsed_ms();
        draft.events.push(Event::Move { t, x, y });
    });
}

pub fn capture_button(button: &str, down: bool, x: i32, y: i32) {
    with(|state| {
        let Some(draft) = state.draft.as_mut() else {
            return;
        };
        if draft.ignore_buttons.iter().any(|ignored| ignored == button) {
            return;
        }
        if draft.events.len() >= MAX_EVENTS {
            draft.capped = true;
            return;
        }
        let t = draft.elapsed_ms();
        draft.events.push(Event::Button {
            t,
            x,
            y,
            button: button.to_string(),
            down,
        });
    });
}

pub fn capture_wheel(amount: i32, x: i32, y: i32) {
    with(|state| {
        let Some(draft) = state.draft.as_mut() else {
            return;
        };
        if draft.ignore_buttons.iter().any(|ignored| ignored.starts_with("wheel")) {
            return;
        }
        if amount == 0 || draft.events.len() >= MAX_EVENTS {
            draft.capped = true;
            return;
        }
        let t = draft.elapsed_ms();
        draft.events.push(Event::Wheel { t, x, y, amount });
    });
}

// ── the shortcuts' own keys ────────────────────────────────────────

/// The scan codes a shortcut is made of.
///
/// `Control+Alt+R` yields Ctrl, Alt and R, so pressing that chord while
/// recording writes nothing into the recording.
fn hotkey_scans(accelerator: &str) -> Vec<u16> {
    let mut scans = Vec::new();
    for part in accelerator.split('+') {
        let token = part.trim();
        if token.is_empty() {
            continue;
        }
        let scan = match token.to_ascii_lowercase().as_str() {
            "control" | "ctrl" => Some(0x1d),
            "shift" => Some(0x2a),
            "alt" | "option" => Some(0x38),
            "meta" | "super" | "cmd" | "win" => Some(0x5b),
            _ => keys::all()
                .iter()
                .find(|key| key.label.eq_ignore_ascii_case(token))
                .map(|key| key.scan),
        };
        if let Some(scan) = scan {
            scans.push(scan);
        }
    }
    scans
}

/// The mouse buttons a shortcut is made of, in the recorder's vocabulary.
fn hotkey_buttons(accelerator: &str) -> Vec<String> {
    accelerator
        .split('+')
        .filter_map(|part| {
            let name = match part.trim().to_ascii_lowercase().as_str() {
                "mouse1" | "mouseleft" | "lmb" => "left",
                "mouse2" | "mouseright" | "rmb" => "right",
                "mouse3" | "mousemiddle" | "mmb" => "middle",
                "mouse4" | "xbutton1" | "back" => "back",
                "mouse5" | "xbutton2" | "forward" => "forward",
                "wheelup" | "scrollup" => "wheelup",
                "wheeldown" | "scrolldown" => "wheeldown",
                _ => return None,
            };
            Some(name.to_string())
        })
        .collect()
}

// ── replaying ──────────────────────────────────────────────────────

/// Start a replay of the selected recording — or of `id` — on its own thread.
pub fn start(
    runtime: &RuntimeState,
    settings: &Settings,
    id: Option<&str>,
) -> Result<(), String> {
    if with(|state| state.draft.is_some()) {
        return Err("a recording is running".to_string());
    }
    let wanted = id
        .map(|value| value.to_string())
        .unwrap_or_else(|| settings.replay.selected.clone());
    let recording = settings
        .replay
        .recordings
        .iter()
        .find(|entry| entry.id == wanted)
        .cloned()
        .ok_or_else(|| "there is no recording to replay".to_string())?;
    if recording.events.is_empty() {
        return Err("that recording has nothing in it".to_string());
    }

    let stop = runtime.claim(REPLAY)?;
    let passes = settings.replay.repeat.map(|count| count.max(1));
    let speed = settings.replay.speed();
    with(|state| {
        state.progress = Progress {
            status: "replaying".to_string(),
            played: 0,
            total: recording.events.len() as u32,
            pass: 0,
            passes,
            speed,
            note: None,
        };
    });

    let shared = Arc::clone(runtime);
    let events = recording.events.clone();
    std::thread::Builder::new()
        .name("crossup-replay".into())
        .spawn(move || {
            let outcome = play_inside(&events, passes, speed, &stop);
            let mut progress = with(|state| std::mem::take(&mut state.progress));
            progress.status = match outcome {
                Ok(()) => "done".to_string(),
                Err(why) => why,
            };
            with(|state| state.progress = progress);
            shared.release(0, 0.0);
        })
        .map_err(|error| {
            with(|state| state.progress.status = "stopped".to_string());
            runtime.release(0, 0.0);
            format!("could not start the replay thread: {error}")
        })?;

    Ok(())
}

/// The replay shortcut fired: stop a replay, or start one.
pub fn replay_shortcut(app: &AppHandle, runtime: &RuntimeState) {
    if runtime.running().as_deref() == Some(REPLAY) {
        runtime.stop();
        return;
    }
    if with(|state| state.draft.is_some()) {
        // Recording now: the replay key is only a key, and pressing it must not
        // start anything that would be recorded.
        return;
    }
    let settings = {
        let state = app.state::<SharedState>();
        let shared = state.lock().unwrap();
        shared.settings.clone()
    };
    request(runtime, &settings);
}

/// The one answer to "play something", shared by the chord and the button.
///
/// A person pressing the replay key and a person clicking Replay are asking the
/// same question, so a refusal lands in the same place for both: the strip above
/// the tabs. The button used to be disabled when nothing was selected, which made
/// the tab silent about the one thing it could have explained.
pub fn request(runtime: &RuntimeState, settings: &Settings) {
    if !settings.replay_enabled {
        note("the replayer is switched off".to_string());
        runtime.alert("the replayer is switched off, so nothing can replay");
        return;
    }
    if !settings
        .replay
        .recordings
        .iter()
        .any(|entry| entry.id == settings.replay.selected)
    {
        runtime.alert("a replay needs a recording: record one, then pick it on the Replayer tab");
        return;
    }
    if let Err(error) = start(runtime, settings, None) {
        runtime.alert(format!("the replay could not start: {error}"));
        note(error);
    }
}

/// Play a recording from inside a run that already holds the desktop — a macro.
///
/// The badge is shown for the same reason it is shown when the Replayer tab
/// starts one: the screen is being driven and nothing else on the desktop says
/// so. Nothing is claimed and nothing is released, because the caller owns the
/// slot and will release it when it finishes.
pub fn play_inside(
    events: &[Event],
    passes: Option<u32>,
    speed: f64,
    stop: &AtomicBool,
) -> Result<(), String> {
    if events.is_empty() {
        return Err("that recording has nothing in it".to_string());
    }
    badge(true, speed);
    let outcome = drive(events, passes, speed, stop);
    badge(false, speed);
    outcome
}

/// Play the events, one pass after another.
///
/// Deliberately does not claim the run slot: the caller either owns it already
/// or is running inside something that does. Claiming here would make every
/// nested replay fail against its own caller.
pub fn drive(
    events: &[Event],
    passes: Option<u32>,
    speed: f64,
    stop: &AtomicBool,
) -> Result<(), String> {
    // A 1 ms timer, so a mouse path drawn from sixty samples a second lands
    // near where it was drawn rather than in fifteen-millisecond steps.
    let _timer = input::TimerResolution::raise();
    let per_pass = events.len() as u32;
    let mut pass = 0u32;

    loop {
        if stop.load(Ordering::Relaxed) {
            return Err("Stopped".to_string());
        }
        if let Some(limit) = passes {
            if pass >= limit {
                return Ok(());
            }
        }
        pass += 1;
        with(|state| state.progress.pass = pass);

        let began = Instant::now();
        for (index, event) in events.iter().enumerate() {
            let due = began + Duration::from_millis(scaled_ms(event_time(event), speed));
            if !wait_until(stop, due) {
                return Err("Stopped".to_string());
            }
            apply(event);
            let played = (pass - 1) * per_pass + index as u32 + 1;
            with(|state| state.progress.played = played);
        }

        if passes.map(|limit| pass >= limit).unwrap_or(false) {
            return Ok(());
        }
        if !wait_until(
            stop,
            Instant::now() + Duration::from_millis(scaled_ms(BETWEEN_PASSES_MS as u32, speed)),
        ) {
            return Err("Stopped".to_string());
        }
    }
}

fn apply(event: &Event) {
    match event {
        Event::Move { x, y, .. } => input::move_to(*x, *y),
        Event::Button { button, down, .. } => {
            if let Some(button) = mouse_button(button) {
                input::button(button, *down);
            }
        }
        Event::Wheel { amount, .. } => input::scroll(*amount),
        Event::Key { scan, down, .. } => input::key(*scan, *down),
    }
}

/// A recorded moment, moved to the pace this replay is going at.
///
/// At twice the speed every gap halves, so the run finishes in half the time; at
/// half the speed they double. The rounding is a millisecond, which is under the
/// timer the run asks Windows for.
fn scaled_ms(t: u32, speed: f64) -> u64 {
    (t as f64 / clamp_speed(speed)).round() as u64
}

// ── the badge ─────────────────────────────────────────────────────

/// The badge window's label, so the main window can tell it apart.
pub const BADGE: &str = "replay-badge";

static BADGE_APP: OnceLock<AppHandle> = OnceLock::new();

/// Build the small "replaying" window, hidden, at startup.
///
/// It is built here rather than when a replay starts because creating a webview
/// from the replay thread hangs the app — the same trap the region picker
/// documents. After this, a run only shows and hides it.
pub fn install_badge(app: &AppHandle) -> Result<(), String> {
    let _ = BADGE_APP.set(app.clone());
    if app.get_webview_window(BADGE).is_none() {
        let window = WebviewWindowBuilder::new(app, BADGE, WebviewUrl::App("badge.html".into()))
            .title("Replaying")
            .inner_size(150.0, 32.0)
            .position(18.0, 18.0)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .resizable(false)
            .shadow(false)
            .visible(false)
            .build()
            .map_err(|error| format!("could not build the replay badge: {error}"))?;
        // Clicks belong to the window underneath: the badge reports on the run,
        // it never stands between the run and its target.
        let _ = window.set_ignore_cursor_events(true);
    }
    Ok(())
}

/// Show or hide the badge. Called from the replay thread, which is why it takes
/// the window by label instead of holding one.
fn badge(playing: bool, _speed: f64) {
    let Some(app) = BADGE_APP.get() else { return };
    let Some(window) = app.get_webview_window(BADGE) else { return };
    if playing {
        let _ = window.show();
    } else {
        let _ = window.hide();
    }
}

fn mouse_button(name: &str) -> Option<MouseButton> {
    Some(match name {
        "left" => MouseButton::Left,
        "right" => MouseButton::Right,
        "middle" => MouseButton::Middle,
        _ => return None,
    })
}

/// Wait until a moment, watching the stop flag while it waits.
fn wait_until(stop: &AtomicBool, at: Instant) -> bool {
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let now = Instant::now();
        if now >= at {
            return true;
        }
        let slice = (at - now).as_millis().clamp(1, 15) as u64;
        std::thread::sleep(Duration::from_millis(slice));
    }
}

// ── commands ──────────────────────────────────────────────────────

/// What the session is doing, polled by the tab.
#[tauri::command]
pub fn replay_state() -> ReplayState {
    view()
}

#[tauri::command]
pub fn start_recording(
    app: AppHandle,
    runtime: State<'_, RuntimeState>,
    state: State<'_, SharedState>,
) -> Result<ReplayState, String> {
    let settings = state.lock().unwrap().settings.clone();
    if !settings.replay_enabled {
        return Err("the replayer is switched off".to_string());
    }
    // The window hides itself while recording, so a shortcut is the only way
    // back: refusing here is the difference between a toggle and a trap.
    if settings.replay.record_hotkey.trim().is_empty() {
        return Err(
            "set a record shortcut first: the window hides while a recording runs".to_string(),
        );
    }
    if let Some(running) = runtime.running() {
        return Err(format!("{running} is running; stop it before recording"));
    }
    begin(&app, &settings)?;
    Ok(view())
}

#[tauri::command]
pub fn stop_recording(app: AppHandle) -> ReplayState {
    stop(&app);
    view()
}

#[tauri::command]
pub fn start_replay(
    runtime: State<'_, RuntimeState>,
    state: State<'_, SharedState>,
    id: Option<String>,
) -> Result<ReplayState, String> {
    let settings = state.lock().unwrap().settings.clone();
    if !settings.replay_enabled {
        return Err("the replayer is switched off".to_string());
    }
    match id.as_deref() {
        Some(id) => start(&runtime, &settings, Some(id))?,
        // Nothing chosen: the same answer the chord gives, in the same strip.
        None => request(&runtime, &settings),
    }
    Ok(view())
}

#[tauri::command]
pub fn stop_replay(runtime: State<'_, RuntimeState>) -> ReplayState {
    runtime.stop();
    view()
}

#[tauri::command]
pub fn discard_recording() -> ReplayState {
    discard_pending();
    view()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_contributes_every_key_it_is_made_of() {
        let scans = hotkey_scans("Control+Alt+R");
        assert!(scans.contains(&0x1d), "{scans:?}");
        assert!(scans.contains(&0x38), "{scans:?}");
        // The main key resolves through the app's own key table.
        assert_eq!(scans.len(), 3, "R should have resolved: {scans:?}");
        assert!(hotkey_scans("").is_empty());
    }

    #[test]
    fn a_mouse_shortcut_is_read_as_a_button() {
        assert_eq!(hotkey_buttons("Mouse4"), vec!["back".to_string()]);
        assert_eq!(hotkey_buttons("Control+WheelDown"), vec!["wheeldown".to_string()]);
        assert!(hotkey_buttons("Control+Alt+R").is_empty());
    }

    /// The times a recording carries are the times it replays at, so the last
    /// one is the length of the recording rather than how long the person took
    /// to reach for the shortcut afterwards.
    #[test]
    fn the_length_is_the_last_event() {
        let events = vec![
            Event::Move { t: 0, x: 1, y: 1 },
            Event::Key { t: 240, scan: 0x1e, down: true },
            Event::Key { t: 290, scan: 0x1e, down: false },
        ];
        assert_eq!(event_time(events.last().expect("events")), 290);
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    #[test]
    fn a_key_with_no_scan_code_is_not_recorded() {
        with(|state| state.draft = Some(Draft::default()));
        capture_key(0, true);
        let empty = with(|state| state.draft.as_ref().map(|draft| draft.events.len()));
        assert_eq!(empty, Some(0), "a key with no scan code was recorded");

        capture_key(0x2d, true);
        capture_key(0x2d, false);
        let typed = with(|state| state.draft.take().map(|draft| draft.events.len()));
        assert_eq!(typed, Some(2));
    }
}

#[cfg(test)]
mod slot_tests {
    use super::*;
    use crate::runtime::{Runtime, RuntimeState};

    fn runtime() -> RuntimeState {
        std::sync::Arc::new(Runtime::new())
    }

    fn one_event() -> Event {
        Event::Move {
            t: 0,
            x: 10,
            y: 20,
        }
    }

    /// The whole point of 1.5: a replay asked for by a macro plays inside the
    /// macro's claim instead of asking for a second one, which the runtime would
    /// refuse. No electron passes here on purpose — the question is the claim.
    #[test]
    fn playing_from_inside_a_run_leaves_the_claim_alone() {
        let runtime = runtime();
        let stop = runtime.claim("macro A").expect("the slot should be free");

        let outcome = play_inside(&[one_event()], Some(0), 1.0, &stop);
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(
            runtime.running(),
            Some("macro A".to_string()),
            "the replay took or released the slot"
        );
    }

    #[test]
    fn a_recording_with_nothing_in_it_says_so() {
        let stop = AtomicBool::new(false);
        let outcome = play_inside(&[], Some(1), 1.0, &stop);
        assert_eq!(outcome.unwrap_err(), "that recording has nothing in it");
    }

    /// A stop reaches a playback that is nested inside a macro just as it
    /// reaches one started from the tab.
    #[test]
    fn a_stop_reaches_a_playback_inside_a_macro() {
        let stop = AtomicBool::new(true);
        let outcome = play_inside(&[one_event()], None, 1.0, &stop);
        assert_eq!(outcome.unwrap_err(), "Stopped");
    }

    #[test]
    fn a_playback_with_no_passes_to_make_finishes_at_once() {
        let stop = AtomicBool::new(false);
        assert!(play_inside(&[one_event()], Some(0), 1.0, &stop).is_ok());
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;
    use crate::runtime::Runtime;

    #[test]
    fn a_replay_with_nothing_chosen_says_so_and_starts_nothing() {
        let runtime: RuntimeState = std::sync::Arc::new(Runtime::new());
        let settings = Settings::default();
        request(&runtime, &settings);
        assert_eq!(runtime.running(), None, "something started");
        let alerts = runtime.view().alerts;
        assert!(
            alerts.iter().any(|message| message.contains("needs a recording")),
            "{alerts:?}"
        );
    }

    #[test]
    fn a_dismissed_notice_is_gone_until_the_next_refusal() {
        let runtime: RuntimeState = std::sync::Arc::new(Runtime::new());
        request(&runtime, &Settings::default());
        assert!(!runtime.view().alerts.is_empty());
        runtime.dismiss();
        assert!(runtime.view().alerts.is_empty(), "the notice survived a dismiss");
        request(&runtime, &Settings::default());
        assert!(!runtime.view().alerts.is_empty(), "a new refusal stayed hidden");
    }
}
