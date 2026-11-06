//! Crossup: profiles per program, key mappings with the four SOCD
//! modes, an auto clicker and a macro runner, a tray icon and a settings window.
//!
//! The shell owns the state; the React layer only asks for it and edits it.
//! Every edit goes through a command, which persists the settings and, if the
//! edited profile is the one in front, re-installs the engine's rules.
//!
//! The three tools share one window and one process, and `runtime` makes sure
//! only one of them is driving the desktop at a time.

mod autostart;
mod clicker;
pub mod clipboard;
mod config;
mod hook;
mod input;
mod keys;
mod macros;
mod mouse_hotkeys;
mod picker;
mod pictures;
mod vision;
mod replay;
mod runtime;
mod screenshots;
mod socd;
pub mod spawn;
mod typer;
mod update;
pub mod window;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;

use clicker::ClickerSettings;
use config::{Mapping, Mode, Profile, Settings};
use hook::{Shared, SharedState};
use macros::Macro;
use typer::TyperSettings;
use runtime::{ActivityView, Runtime, RuntimeState};

/// The last delete, held in memory so the window can take it back.
///
/// One slot, because the window offers exactly one undo and says so. A stack
/// would need a way to show the entries, and a delete that is older than the
/// toast is already out of reach.
enum Undo {
    Recording {
        index: usize,
        recording: replay::Recording,
        selected: String,
    },
    Macro {
        index: usize,
        definition: Macro,
    },
    Mapping {
        profile: String,
        index: usize,
        mapping: Mapping,
    },
}

static UNDO: Mutex<Option<Undo>> = Mutex::new(None);

// ── what the UI reads ──────────────────────────────────────────────

#[derive(Serialize)]
struct MappingView {
    mode: Mode,
    source: u16,
    dest: u16,
    source_label: String,
    dest_label: String,
}

#[derive(Serialize)]
struct ProfileView {
    id: String,
    program: String,
    active: bool,
    global: bool,
    mappings: Vec<MappingView>,
}

#[derive(Serialize)]
struct KeyView {
    scan: u16,
    label: String,
    group: String,
}

#[derive(Serialize)]
struct MacroView {
    #[serde(flatten)]
    definition: Macro,
    /// Why this macro cannot run, if it cannot. The UI shows it on the row.
    problem: Option<String>,
}

/// One saved recording, as the list shows it: no events, because the window
/// never needs them and a long recording is not something to send every frame.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingView {
    id: String,
    name: String,
    events: usize,
    duration_ms: u32,
}

/// The replayer's saved state. What it is *doing* is polled through
/// `replay::replay_state` instead: a recording changes while the window is
/// hidden, and has no business being part of a settings snapshot.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplayView {
    recordings: Vec<RecordingView>,
    selected: String,
    record_hotkey: String,
    replay_hotkey: String,
    /// `None` is forever.
    repeat: Option<u32>,
    /// Playback speed as a multiple of the recorded pace.
    speed: f64,
}

#[derive(Serialize)]
struct View {
    profiles: Vec<ProfileView>,
    /// The Remapper's switch. The field keeps its old name so saved settings and
    /// `.socd` files still load.
    hook_enabled: bool,
    clicker_enabled: bool,
    macros_enabled: bool,
    installed: bool,
    close_to_tray: bool,
    debug: bool,
    program: String,
    log: Vec<String>,
    keys: Vec<KeyView>,
    clicker: ClickerSettings,
    macros: Vec<MacroView>,
    /// The typer's draft and pace, and whether the tool is on.
    typer: TyperSettings,
    typer_enabled: bool,
    replay: ReplayView,
    replay_enabled: bool,
    /// Read from the registry, not from the settings: the settings only hold
    /// which executable to use, whether the entry exists is Windows' business.
    autostart: autostart::AutostartView,
}

/// The saved state. What is *running* is deliberately not part of this: the
/// window polls [`activity`] for that, so an edit never has to answer a question
/// about the run it interrupted.
fn view_of(shared: &Shared) -> View {
    View {
        autostart: autostart::view(&shared.settings.autostart_exe),
        clicker: shared.settings.clicker.clone(),
        macros: shared
            .settings
            .macros
            .iter()
            .map(|definition| MacroView {
                problem: macros::validate(definition).err(),
                definition: definition.clone(),
            })
            .collect(),
        typer: shared.settings.typer.clone(),
        typer_enabled: shared.settings.typer_enabled,
        replay: ReplayView {
            recordings: shared
                .settings
                .replay
                .recordings
                .iter()
                .map(|recording| RecordingView {
                    id: recording.id.clone(),
                    name: recording.name.clone(),
                    events: recording.events.len(),
                    duration_ms: recording.duration_ms,
                })
                .collect(),
            selected: shared.settings.replay.selected.clone(),
            record_hotkey: shared.settings.replay.record_hotkey.clone(),
            replay_hotkey: shared.settings.replay.replay_hotkey.clone(),
            repeat: shared.settings.replay.repeat,
            speed: shared.settings.replay.speed,
        },
        replay_enabled: shared.settings.replay_enabled,
        profiles: shared
            .settings
            .profiles
            .iter()
            .map(|profile| ProfileView {
                id: profile.id.clone(),
                program: profile.program.clone(),
                active: profile.active,
                global: profile.is_global(),
                mappings: profile
                    .mappings
                    .iter()
                    .map(|mapping| MappingView {
                        mode: mapping.mode,
                        source: mapping.source,
                        dest: mapping.dest,
                        source_label: keys::label(mapping.source),
                        dest_label: keys::label(mapping.dest),
                    })
                    .collect(),
            })
            .collect(),
        hook_enabled: shared.settings.hook_enabled,
        clicker_enabled: shared.settings.clicker_enabled,
        macros_enabled: shared.settings.macros_enabled,
        installed: shared.installed,
        close_to_tray: shared.settings.close_to_tray,
        debug: shared.settings.debug,
        program: shared.program.clone(),
        log: shared.log.clone(),
        keys: keys::all()
            .into_iter()
            .map(|key| KeyView {
                scan: key.scan,
                label: key.label.to_string(),
                group: key.group.to_string(),
            })
            .collect(),
    }
}

// ── persistence ────────────────────────────────────────────────────

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("settings.json"))
}

fn save(app: &AppHandle, shared: &Shared) {
    if let Some(path) = settings_path(app) {
        if let Ok(text) = serde_json::to_string_pretty(&shared.settings) {
            let _ = std::fs::write(path, text);
        }
    }
}

/// Read the saved settings, keeping whatever still fits.
///
/// Section by section rather than the whole file at once: one field changing
/// shape should cost that field, not every profile, macro and clicker setting
/// the user has. What could not be read comes back as notes for the log.
fn load(app: &AppHandle) -> (Settings, Vec<String>, bool) {
    let Some(path) = settings_path(app) else {
        return (Settings::default(), Vec::new(), false);
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return (Settings::default(), Vec::new(), false);
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(document) => {
            let (mut settings, mut notes) = Settings::from_document(&document);
            // A slot that could never fire is replaced before the window opens,
            // so nothing on screen shows a shortcut that does nothing.
            let healed = settings.heal_shortcuts();
            let changed = !healed.is_empty();
            notes.extend(healed);
            (settings, notes, changed)
        }
        Err(error) => (
            Settings::default(),
            vec![format!(
                "the saved settings could not be read ({error}); starting from the defaults"
            )],
            false,
        ),
    }
}

/// Re-point the engine at the profile that matches the program in front.
fn retarget(state: &SharedState) -> Vec<socd::Action> {
    let program = hook::foreground_program();
    let mut shared = state.lock().unwrap();
    let actions = shared.retarget(&program);
    actions
}

// ── commands ──────────────────────────────────────────────────────

#[tauri::command]
fn snapshot(state: State<'_, SharedState>) -> View {
    let shared = state.lock().unwrap();
    view_of(&shared)
}

// ── the auto clicker and the macros ───────────────────────────────

/// Live counters, polled by the window while something is running.
#[tauri::command]
fn activity(runtime: State<'_, RuntimeState>) -> ActivityView {
    runtime.view()
}

/// Store the clicker settings. The window edits these continuously, so this is
/// also the "save" for every control on the tab.
#[tauri::command]
fn save_clicker(
    app: AppHandle,
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
    clicker: ClickerSettings,
) -> View {
    let (settings, shortcuts_changed) = {
        let mut shared = state.lock().unwrap();
        let previous = shared.settings.clicker.clone();
        // The window saves on every keystroke and slider move, and re-registering
        // every shortcut each time would make the whole machine flicker.
        let changed = previous.hotkey != clicker.hotkey || previous.hold_mode != clicker.hold_mode;
        shared.settings.clicker = clicker;
        save(&app, &shared);
        (shared.settings.clone(), changed)
    };
    if shortcuts_changed {
        runtime::register_shortcuts(&runtime, &app, &settings);
    }
    snapshot(state)
}

#[tauri::command]
fn start_clicker(
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
) -> Result<View, String> {
    let settings = state.lock().unwrap().settings.clone();
    if !settings.clicker_enabled {
        return Err("the auto clicker is switched off".to_string());
    }
    runtime::start_clicker(&runtime, settings.clicker)?;
    Ok(snapshot(state))
}

/// Stop whatever is running — the clicker or a macro.
#[tauri::command]
fn stop_activity(runtime: State<'_, RuntimeState>) -> bool {
    runtime.stop()
}

/// Clear the strip above the tabs. A refusal is a message, not a setting:
/// dismissing it changes nothing except what is on screen.
#[tauri::command]
fn dismiss_notices(state: State<'_, SharedState>, runtime: State<'_, RuntimeState>) -> View {
    runtime.dismiss();
    snapshot(state)
}

/// Add or replace a macro, then re-register the shortcuts so its new one is
/// live straight away.
#[tauri::command]
fn save_macro(
    app: AppHandle,
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
    definition: Macro,
) -> Result<View, String> {
    macros::validate(&definition)?;
    let (settings, shortcuts_changed) = {
        let mut shared = state.lock().unwrap();
        let changed = match shared
            .settings
            .macros
            .iter_mut()
            .find(|entry| entry.id == definition.id)
        {
            Some(existing) => {
                let changed = existing.hotkey != definition.hotkey
                    || existing.enabled != definition.enabled
                    || existing.name != definition.name;
                *existing = definition.clone();
                changed
            }
            None => {
                shared.settings.macros.push(definition.clone());
                true
            }
        };
        save(&app, &shared);
        (shared.settings.clone(), changed)
    };
    if shortcuts_changed {
        runtime::register_shortcuts(&runtime, &app, &settings);
    }
    Ok(snapshot(state))
}

#[tauri::command]
fn delete_macro(
    app: AppHandle,
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
    id: String,
) -> View {
    let settings = {
        let mut shared = state.lock().unwrap();
        if let Some(index) = shared.settings.macros.iter().position(|entry| entry.id == id) {
            let definition = shared.settings.macros.remove(index);
            *UNDO.lock().unwrap() = Some(Undo::Macro { index, definition });
            save(&app, &shared);
        }
        shared.settings.clone()
    };
    runtime::register_shortcuts(&runtime, &app, &settings);
    snapshot(state)
}

/// Put the macros in the order the rail shows them.
///
/// The list is the order, rather than a position stored on each macro: moving a
/// row is one write instead of one per row, and a macro added afterwards still
/// lands at the end of the list without anything having to number it.
#[tauri::command]
fn reorder_macros(app: AppHandle, state: State<'_, SharedState>, ids: Vec<String>) -> View {
    {
        let mut shared = state.lock().unwrap();
        let mut wanted: Vec<Macro> = Vec::with_capacity(shared.settings.macros.len());
        for id in &ids {
            if let Some(index) = shared.settings.macros.iter().position(|entry| &entry.id == id) {
                wanted.push(shared.settings.macros.remove(index));
            }
        }
        // Anything the window did not name — a macro another window added while
        // this drag was in flight — keeps its place at the end instead of being
        // dropped from the settings file.
        wanted.append(&mut shared.settings.macros);
        shared.settings.macros = wanted;
        save(&app, &shared);
    }
    snapshot(state)
}

#[tauri::command]
fn run_macro(
    app: AppHandle,
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
    id: String,
) -> Result<View, String> {
    let settings = state.lock().unwrap().settings.clone();
    if !settings.macros_enabled {
        return Err("the macros are switched off".to_string());
    }
    let definition = settings
        .macros
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
        .ok_or_else(|| "that macro is no longer in the list".to_string())?;
    runtime::start_macro(&runtime, &app, &definition)?;
    Ok(snapshot(state))
}

/// Store the typer's draft and pace.
///
/// This is also the "save" for every control on the tab, so the text lands in
/// the settings file as it is typed rather than on a button press. The shortcut
/// is only re-registered when the shortcut itself changed: saving every
/// keystroke would otherwise take the key back from the system each time.
#[tauri::command]
fn save_typer(
    app: AppHandle,
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
    typer: TyperSettings,
) -> View {
    let (settings, changed) = {
        let mut shared = state.lock().unwrap();
        let changed = shared.settings.typer.hotkey != typer.hotkey;
        shared.settings.typer = typer;
        save(&app, &shared);
        (shared.settings.clone(), changed)
    };
    if changed {
        runtime::register_shortcuts(&runtime, &app, &settings);
    }
    snapshot(state)
}

/// Start typing the draft, on a thread of its own.
#[tauri::command]
fn start_typer(
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
    holder: State<'_, typer::Holder>,
) -> Result<View, String> {
    let settings = state.lock().unwrap().settings.clone();
    if !settings.typer_enabled {
        return Err("the typer is switched off".to_string());
    }
    typer::start(&runtime, &holder, &settings.typer)?;
    Ok(snapshot(state))
}

/// A fresh id for a recording. The count is only a hint, so the loop is what
/// makes it unique after a delete.
fn recording_id(settings: &Settings) -> String {
    let mut next = settings.replay.recordings.len() + 1;
    loop {
        let id = format!("replay-{next}");
        if !settings
            .replay
            .recordings
            .iter()
            .any(|entry| entry.id == id)
        {
            return id;
        }
        next += 1;
    }
}

/// Save the recording that just finished, under a name.
#[tauri::command]
fn save_recording(
    app: AppHandle,
    state: State<'_, SharedState>,
    name: String,
) -> Result<View, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("a recording needs a name before it can be saved".to_string());
    }
    let draft = replay::take_pending()
        .ok_or_else(|| "there is no recording waiting to be saved".to_string())?;
    {
        let mut shared = state.lock().unwrap();
        let id = recording_id(&shared.settings);
        shared.settings.replay.recordings.push(replay::Recording {
            id: id.clone(),
            name,
            events: draft.events,
            duration_ms: draft.duration_ms,
        });
        shared.settings.replay.selected = id;
        save(&app, &shared);
    }
    Ok(snapshot(state))
}

#[tauri::command]
fn delete_recording(app: AppHandle, state: State<'_, SharedState>, id: String) -> View {
    {
        let mut shared = state.lock().unwrap();
        if let Some(index) = shared
            .settings
            .replay
            .recordings
            .iter()
            .position(|entry| entry.id == id)
        {
            let recording = shared.settings.replay.recordings.remove(index);
            *UNDO.lock().unwrap() = Some(Undo::Recording {
                index,
                recording,
                selected: shared.settings.replay.selected.clone(),
            });
        }
        // The selection has to keep pointing at something: a Play button aimed
        // at a deleted recording would fail with no way to tell why.
        if shared.settings.replay.selected == id {
            shared.settings.replay.selected = shared
                .settings
                .replay
                .recordings
                .first()
                .map(|entry| entry.id.clone())
                .unwrap_or_default();
        }
        save(&app, &shared);
    }
    snapshot(state)
}

#[tauri::command]
fn select_recording(app: AppHandle, state: State<'_, SharedState>, id: String) -> View {
    {
        let mut shared = state.lock().unwrap();
        if shared
            .settings
            .replay
            .recordings
            .iter()
            .any(|entry| entry.id == id)
        {
            shared.settings.replay.selected = id;
            save(&app, &shared);
        }
    }
    snapshot(state)
}

/// The replayer's two shortcuts, and how many passes a replay makes.
///
/// Saving them re-registers the shortcuts, which is why they are not part of
/// the settings write every other tab does.
#[tauri::command]
fn save_replay_options(
    app: AppHandle,
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
    record_hotkey: String,
    replay_hotkey: String,
    repeat: Option<u32>,
    speed: f64,
) -> View {
    {
        let mut shared = state.lock().unwrap();
        shared.settings.replay.record_hotkey = record_hotkey.trim().to_string();
        shared.settings.replay.replay_hotkey = replay_hotkey.trim().to_string();
        shared.settings.replay.repeat = repeat;
        shared.settings.replay.speed = replay::clamp_speed(speed);
        save(&app, &shared);
    }
    let settings = state.lock().unwrap().settings.clone();
    runtime::register_shortcuts(&runtime, &app, &settings);
    snapshot(state)
}

/// Turn one tool on or off.
///
/// `tool` is `remapper`, `clicker`, `typer`, `macros` or `replay`. Switching a
/// tool off also stops what it was doing: a disabled clicker that kept clicking,
/// or a remapper left holding keys down, would be worse than the global pause
/// this replaced.
#[tauri::command]
fn set_tool_enabled(
    app: AppHandle,
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
    tool: String,
    enabled: bool,
) -> View {
    // One implementation, shared with the Tool block: two copies of "what a
    // switch means" would drift.
    let _ = runtime::apply_tool(&app, &state, &runtime, &tool, enabled, None);
    snapshot(state)
}

#[tauri::command]
fn set_preferences(
    app: AppHandle,
    state: State<'_, SharedState>,
    close_to_tray: bool,
    debug: bool,
) -> View {
    {
        let mut shared = state.lock().unwrap();
        shared.settings.close_to_tray = close_to_tray;
        shared.settings.debug = debug;
        save(&app, &shared);
    }
    snapshot(state)
}

/// Turn the sign-in entry on or off, and remember which executable it points at.
///
/// `exe` is empty for "the copy that is running", which is what almost everyone
/// wants; the field exists because a moved or side-by-side copy makes the
/// automatic answer wrong in a way nobody notices until the next sign-in.
#[tauri::command]
fn set_autostart(
    app: AppHandle,
    state: State<'_, SharedState>,
    enabled: bool,
    exe: String,
) -> Result<View, String> {
    let chosen = exe.trim().to_string();
    if enabled {
        let target = if chosen.is_empty() {
            autostart::running_exe()
        } else {
            chosen.clone()
        };
        autostart::enable(&target)?;
    } else {
        autostart::disable()?;
    }
    {
        let mut shared = state.lock().unwrap();
        shared.settings.autostart_exe = chosen;
        save(&app, &shared);
    }
    Ok(snapshot(state))
}

/// Ask Windows for an executable instead of making the user type a path.
///
/// The answer arrives as an event rather than as the command's return value:
/// the picker is modal to the desktop, and answering through a callback leaves
/// the window free to keep drawing while it is open.
#[tauri::command]
fn choose_exe(app: AppHandle) {
    app.dialog()
        .file()
        .set_title("Which copy of Crossup should start with Windows?")
        .add_filter("Programs", &["exe"])
        .pick_file(move |picked| {
            let Some(path) = picked else { return };
            let _ = app.emit_to("main", "autostart-exe", path.to_string());
        });
}

// ── the region picker ─────────────────────────────────────────────

/// Open the transparent overlay and let the user drag the rectangle themselves.
///
/// `async` here is load-bearing, not decoration. Creating a webview window from
/// a *synchronous* command deadlocks on Windows, as Tauri says on
/// `WebviewWindowBuilder::new` (it cites the WebView2 issue): a sync command
/// runs on the main thread, and creating the webview waits for that same thread
/// to finish creating it. An async command runs on Tauri's runtime instead,
/// which is exactly where "use async commands when creating windows" comes from.
///
/// The symptom of getting this wrong was not a failed call but a hung app: the
/// overlay window was created and then never sized, shown or filled, so the
/// pick button appeared to freeze the app, nothing could be drawn, a second
/// launch could not bring the window forward, and the app could not be quit.
#[tauri::command]
async fn pick_region(app: AppHandle, request: picker::PickRequest) -> Result<(), String> {
    picker::open(&app, request)
}

/// What the picker should draw, asked by the picker window itself.
/// Take a dragged rectangle on screen and keep it as a picture for a macro.
#[tauri::command]
fn save_picture(
    app: AppHandle,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<pictures::Picture, String> {
    let shot = pictures::capture(
        &app,
        picker::Rect {
            x,
            y,
            width,
            height,
        },
    );
    // The app was hidden for the pick and comes back whether or not the
    // rectangle was a picture worth keeping: a size the capture turns down is
    // something to say, not a reason for the window to stay away.
    picker::restore(&app);
    shot
}

/// The pictures already saved, newest first, so a block can reuse one instead
/// of making the same drag across the screen again.
#[tauri::command]
fn picture_history(app: AppHandle) -> Vec<pictures::Picture> {
    pictures::history(&app)
}

/// The picture itself, as a data URL, for the thumbnail beside its block.
#[tauri::command]
fn picture_preview(app: AppHandle, id: String) -> Result<String, String> {
    pictures::preview(&app, &id)
}

/// Forget a picture whose block has been removed.
#[tauri::command]
fn delete_picture(app: AppHandle, id: String) -> Result<(), String> {
    pictures::remove(&app, &id)
}

/// Take a picture off the saved shelf, unless a block still looks for it.
///
/// The shelf is worth having because blocks share one picture, so removing a
/// shared one would leave every other block saying the picture is missing. A
/// picture that is still wanted is refused with the count, which is also the
/// answer to "why can't I delete this one?" — one of the blocks in front of you
/// is using it.
#[tauri::command]
fn forget_picture(
    app: AppHandle,
    state: State<'_, SharedState>,
    id: String,
) -> Result<(), String> {
    let users = {
        let shared = state.lock().unwrap();
        shared
            .settings
            .macros
            .iter()
            .flat_map(|entry| macros::pictures(&entry.steps))
            .filter(|image| image == &id)
            .count()
    };
    if users > 0 {
        return Err(format!(
            "that picture is still used by {users} block{} — change those first",
            if users == 1 { "" } else { "s" }
        ));
    }
    pictures::remove(&app, &id)
}

/// One block that looks for a picture, by what it is and where it sits.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PictureUse {
    label: String,
    /// Indices into the macro's tree, with `steps` or `otherwise` at each branch.
    path: Vec<String>,
}

/// One macro that still looks for a picture, and the blocks in it that do.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PictureUser {
    id: String,
    macro_name: String,
    blocks: Vec<PictureUse>,
}

/// Which macros, and which blocks in them, still look for this picture.
///
/// The refusal is only useful if it says where: a picture shared by four blocks
/// across two macros is four things to change, and the person reading it is
/// looking at one of them.
#[tauri::command]
fn picture_uses(state: State<'_, SharedState>, id: String) -> Vec<PictureUser> {
    let shared = state.lock().unwrap();
    shared
        .settings
        .macros
        .iter()
        .filter_map(|entry| {
            let blocks: Vec<PictureUse> = macros::picture_blocks(&entry.steps, &id)
                .into_iter()
                .map(|(path, label)| PictureUse { label, path })
                .collect();
            if blocks.is_empty() {
                None
            } else {
                Some(PictureUser {
                    id: entry.id.clone(),
                    macro_name: entry.name.clone(),
                    blocks,
                })
            }
        })
        .collect()
}

/// The titles of the windows a macro could focus, for the builder's list.
#[tauri::command]
fn running_windows() -> Vec<String> {
    let mut titles: Vec<String> = window::visible()
        .into_iter()
        .map(|found| found.title)
        .filter(|title| !title.trim().is_empty())
        .collect();
    titles.sort_by_key(|title| title.to_lowercase());
    titles.dedup();
    titles
}

/// One macro or recording, as a block's picker lists it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Choice {
    id: String,
    name: String,
}

/// Everything a composition block can point at.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BlockChoices {
    macros: Vec<Choice>,
    recordings: Vec<Choice>,
}

/// The macros and recordings a block may name.
#[tauri::command]
fn block_choices(state: State<'_, SharedState>) -> BlockChoices {
    let shared = state.lock().unwrap();
    BlockChoices {
        macros: shared
            .settings
            .macros
            .iter()
            .map(|entry| Choice {
                id: entry.id.clone(),
                name: entry.name.clone(),
            })
            .collect(),
        recordings: shared
            .settings
            .replay
            .recordings
            .iter()
            .map(|entry| Choice {
                id: entry.id.clone(),
                name: entry.name.clone(),
            })
            .collect(),
    }
}

/// Ask for a program rather than making the person type its path.
///
/// The dialog is modal and answers long after the click, so the answer comes
/// back as an event to the window that asked, not as this command's return.
#[tauri::command]
fn choose_program(app: AppHandle) {
    let window = app.clone();
    app.dialog()
        .file()
        .set_title("Which program should the macro start?")
        .add_filter("Programs", &["exe"])
        .pick_file(move |picked| {
            let Some(path) = picked else {
                return;
            };
            let _ = window.emit_to("main", "macro-program", path.to_string());
        });
}

#[tauri::command]
fn picker_context(app: AppHandle) -> Option<picker::PickContext> {
    picker::context(&app)
}

/// The drag is over: take it as screen pixels and tell the main window.
#[tauri::command]
fn picker_finish(app: AppHandle, drag: picker::Rect) -> Result<(), String> {
    picker::finish(&app, drag)
}

#[tauri::command]
fn picker_cancel(app: AppHandle) {
    picker::cancel(&app);
}

#[tauri::command]
fn add_profile(app: AppHandle, state: State<'_, SharedState>, program: String) -> View {
    {
        let mut shared = state.lock().unwrap();
        let program = program.trim().to_string();
        if !program.is_empty()
            && !shared
                .settings
                .profiles
                .iter()
                .any(|profile| profile.program.eq_ignore_ascii_case(&program))
        {
            let id = format!("profile-{}", shared.settings.profiles.len() + 1);
            shared.settings.profiles.push(Profile {
                id,
                program,
                active: true,
                mappings: Vec::new(),
            });
            save(&app, &shared);
        }
    }
    retarget(&state);
    snapshot(state)
}

#[tauri::command]
fn remove_profile(app: AppHandle, state: State<'_, SharedState>, id: String) -> View {
    {
        let mut shared = state.lock().unwrap();
        if let Some(profile) = shared.settings.profile(&id) {
            if !profile.is_global() {
                shared.settings.profiles.retain(|profile| profile.id != id);
                save(&app, &shared);
            }
        }
    }
    retarget(&state);
    snapshot(state)
}

#[tauri::command]
fn set_profile_active(
    app: AppHandle,
    state: State<'_, SharedState>,
    id: String,
    active: bool,
) -> View {
    {
        let mut shared = state.lock().unwrap();
        if let Some(profile) = shared.settings.profile_mut(&id) {
            profile.active = active;
            save(&app, &shared);
        }
    }
    retarget(&state);
    snapshot(state)
}

#[tauri::command]
fn add_mapping(
    app: AppHandle,
    state: State<'_, SharedState>,
    profile: String,
    mode: Mode,
    source: u16,
    dest: u16,
) -> View {
    {
        let mut shared = state.lock().unwrap();
        if let Some(target) = shared.settings.profile_mut(&profile).map(|p| &mut p.mappings) {
            if !target
                .iter()
                .any(|mapping| mapping.source == source && mapping.mode == mode)
            {
                target.push(Mapping { mode, source, dest });
                save(&app, &shared);
            }
        }
    }
    retarget(&state);
    snapshot(state)
}

#[tauri::command]
fn update_mapping(
    app: AppHandle,
    state: State<'_, SharedState>,
    profile: String,
    index: usize,
    mode: Mode,
    source: u16,
    dest: u16,
) -> View {
    {
        let mut shared = state.lock().unwrap();
        if let Some(target) = shared.settings.profile_mut(&profile).map(|p| &mut p.mappings) {
            if let Some(slot) = target.get_mut(index) {
                *slot = Mapping { mode, source, dest };
                save(&app, &shared);
            }
        }
    }
    retarget(&state);
    snapshot(state)
}

#[tauri::command]
fn remove_mapping(
    app: AppHandle,
    state: State<'_, SharedState>,
    profile: String,
    index: usize,
) -> View {
    {
        let mut shared = state.lock().unwrap();
        if let Some(target) = shared.settings.profile_mut(&profile).map(|p| &mut p.mappings) {
            if index < target.len() {
                let mapping = target.remove(index);
                *UNDO.lock().unwrap() = Some(Undo::Mapping {
                    profile: profile.clone(),
                    index,
                    mapping,
                });
                save(&app, &shared);
            }
        }
    }
    retarget(&state);
    snapshot(state)
}

/// Put back whatever the last delete removed, and say nothing if there is
/// nothing left: the window only shows the offer while it exists.
#[tauri::command]
fn undo_delete(
    app: AppHandle,
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
) -> Result<View, String> {
    let undone = UNDO
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| "there is nothing left to take back".to_string())?;
    match undone {
        Undo::Recording {
            index,
            recording,
            selected,
        } => {
            let mut shared = state.lock().unwrap();
            let at = index.min(shared.settings.replay.recordings.len());
            let id = recording.id.clone();
            shared.settings.replay.recordings.insert(at, recording);
            shared.settings.replay.selected = if selected.is_empty() { id } else { selected };
            save(&app, &shared);
        }
        Undo::Macro { index, definition } => {
            let settings = {
                let mut shared = state.lock().unwrap();
                let at = index.min(shared.settings.macros.len());
                shared.settings.macros.insert(at, definition);
                save(&app, &shared);
                shared.settings.clone()
            };
            // Its shortcut becomes live again with the rest.
            runtime::register_shortcuts(&runtime, &app, &settings);
        }
        Undo::Mapping {
            profile,
            index,
            mapping,
        } => {
            {
                let mut shared = state.lock().unwrap();
                if let Some(target) = shared.settings.profile_mut(&profile).map(|p| &mut p.mappings) {
                    let at = index.min(target.len());
                    target.insert(at, mapping);
                    save(&app, &shared);
                }
            }
            retarget(&state);
        }
    }
    Ok(snapshot(state))
}

#[tauri::command]
fn apply_preset(
    app: AppHandle,
    state: State<'_, SharedState>,
    profile: String,
    preset: String,
) -> View {
    let wasd = [(17u16, 31u16), (31, 17), (30, 32), (32, 30)];
    let arrows = [(0x48u16, 0x50u16), (0x50, 0x48), (0x4B, 0x4D), (0x4D, 0x4B)];

    let pairs: Vec<(u16, u16)> = match preset.as_str() {
        "wasd" => wasd.to_vec(),
        "arrows" => arrows.to_vec(),
        _ => wasd.iter().chain(arrows.iter()).copied().collect(),
    };

    {
        let mut shared = state.lock().unwrap();
        if let Some(target) = shared.settings.profile_mut(&profile).map(|p| &mut p.mappings) {
            for (source, dest) in pairs {
                if !target
                    .iter()
                    .any(|mapping| mapping.source == source && mapping.mode == Mode::Opposite)
                {
                    target.push(Mapping {
                        mode: Mode::Opposite,
                        source,
                        dest,
                    });
                }
            }
            save(&app, &shared);
        }
    }
    retarget(&state);
    snapshot(state)
}

#[tauri::command]
fn running_processes() -> Vec<String> {
    hook::running_processes()
}

#[tauri::command]
fn export_socd(state: State<'_, SharedState>) -> String {
    let shared = state.lock().unwrap();
    shared.settings.to_socd()
}

/// The whole settings file as text: every profile, macro, clicker setting and
/// switch, so a copy of it can rebuild this machine's Hitboxer somewhere else.
#[tauri::command]
fn export_settings(state: State<'_, SharedState>) -> String {
    let shared = state.lock().unwrap();
    serde_json::to_string_pretty(&shared.settings).unwrap_or_default()
}

/// Replace every setting from a JSON export.
///
/// It goes through the same reader the app uses at start-up, so a backup from
/// an older build keeps whatever sections still fit and says which ones did
/// not, rather than refusing the file outright.
#[tauri::command]
fn import_settings(
    app: AppHandle,
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
    text: String,
) -> Result<View, String> {
    let document: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("that is not JSON: {error}"))?;
    if !document.is_object() {
        return Err("a settings file is a JSON object".to_string());
    }
    let (imported, notes) = Settings::from_document(&document);
    let settings = {
        let mut shared = state.lock().unwrap();
        shared.settings = imported;
        for note in notes {
            shared.note(format!("import: {note}"));
        }
        save(&app, &shared);
        shared.settings.clone()
    };
    // The profiles in front may have changed, and so may every macro shortcut.
    retarget(&state);
    runtime::register_shortcuts(&runtime, &app, &settings);
    Ok(snapshot(state))
}

#[tauri::command]
fn import_socd(app: AppHandle, state: State<'_, SharedState>, text: String) -> View {
    let (imported, notes) = Settings::parse_socd(&text);
    {
        let mut shared = state.lock().unwrap();
        shared.settings = Settings {
            hook_enabled: shared.settings.hook_enabled,
            ..imported
        };
        for note in notes {
            shared.note(format!("import: {note}"));
        }
        save(&app, &shared);
    }
    retarget(&state);
    snapshot(state)
}

// ── app ───────────────────────────────────────────────────────────

/// Bring the window back from the tray, from minimised, or from behind other
/// windows. Shared by the tray menu, the tray click, a second launch, and the
/// end of a recording — which hides it in the first place.
pub(crate) fn reveal(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Which parts of the window's shape are worth keeping.
///
/// Everything but visibility: closing to the tray hides the window, and a saved
/// "hidden" would make the next launch look like a launch that did nothing.
fn saved_shape() -> tauri_plugin_window_state::StateFlags {
    tauri_plugin_window_state::StateFlags::all()
        & !tauri_plugin_window_state::StateFlags::VISIBLE
}

/// Write the window's shape out now.
///
/// The plugin writes it as the app quits, which is too late for the way Crossup
/// usually leaves: the window has been hidden in the tray for hours by then, and
/// that hidden window is exactly the state a restore should not have to reason
/// about. Saving at the moment it is put away keeps the file meaning "the last
/// shape you saw".
fn save_shape(app: &AppHandle) {
    use tauri_plugin_window_state::AppHandleExt;
    let _ = app.save_window_state(saved_shape());
}

/// Put the window back where it was, and keep it reachable.
fn restore_shape(app: &AppHandle) {
    use tauri_plugin_window_state::WindowExt;
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.restore_state(saved_shape()).is_err() {
        return;
    }
    // A saved spot on a monitor that is no longer plugged in would put the
    // window somewhere nothing can click it. The size is still worth keeping;
    // the position is not.
    if lost_off_screen(&window) {
        let _ = window.center();
    }
}

/// Whether the window's title bar would land on no screen at all.
fn lost_off_screen(window: &tauri::WebviewWindow) -> bool {
    let (Ok(position), Ok(size), Ok(monitors)) = (
        window.outer_position(),
        window.outer_size(),
        window.available_monitors(),
    ) else {
        return false;
    };
    // An 80-point stretch of title bar is enough to grab and drag back.
    let point = (
        position.x + (size.width as i32 / 2).min(80),
        position.y + 8,
    );
    !monitors.iter().any(|monitor| {
        let origin = monitor.position();
        let area = monitor.size();
        point.0 >= origin.x
            && point.0 < origin.x + area.width as i32
            && point.1 >= origin.y
            && point.1 < origin.y + area.height as i32
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Registered first, as the plugin requires. A second launch hands its
        // arguments to this callback and exits, so only one process ever owns
        // the keyboard hook.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            reveal(app);
        }))
        // Where the window was and how big it was, kept beside the settings so
        // closing the app — or updating it — is not a reason to lose the shape
        // it was dragged to. Visibility is left out on purpose: closing to the
        // tray hides the window, and a saved "hidden" would make the next launch
        // look like a launch that did nothing.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(saved_shape())
                .build(),
        )
        // Registered before setup, because the shortcuts are taken in setup.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // Used from Rust only, to let the user point the sign-in entry at a copy
        // of the app rather than type its path.
        .plugin(tauri_plugin_dialog::init())
        // The release endpoint, checked after launch and every six hours. The
        // state the banner reads lives in `update`.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Before anything is drawn in the window, so the first frame is
            // already the shape it was left in.
            restore_shape(app.handle());
            let (settings, notes, healed) = load(app.handle());
            let shared: SharedState = Arc::new(Mutex::new(Shared::new(settings.clone())));
            app.manage(shared.clone());
            // A shortcut that had to be replaced is written back at once: the
            // file on disk should say what the engine just took.
            if healed {
                let guard = shared.lock().unwrap();
                save(app.handle(), &guard);
            }
            {
                // A section that could not be read is worth saying out loud, so
                // "it forgot my settings" comes with the reason it did.
                let mut guard = shared.lock().unwrap();
                for note in notes {
                    guard.note(format!("settings: {note}"));
                }
            }
            let runtime: RuntimeState = Arc::new(Runtime::new());
            app.manage(runtime.clone());
            // The typer's counters live outside the runtime: a paused run is
            // still the runtime's run, but only the typer has a pause.
            app.manage(typer::Holder::new());
            hook::install(shared.clone());
            // Built hidden now, because a webview created from the replay thread
            // would hang the app. A replay only shows and hides it.
            let _ = replay::install_badge(app.handle());
            // The clicker's start shortcut and every enabled macro's shortcut.
            runtime::register_shortcuts(&runtime, app.handle(), &settings);

            let show = MenuItem::with_id(app, "show", "Open Hitboxer", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "Stop clicker and macros", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            // No pause item: each tool has its own switch in its tab, and a
            // second, global one is how the remapper ended up switched off with
            // nothing on screen saying so.
            let menu = Menu::with_items(app, &[&show, &stop, &separator, &quit])?;

            let icon = app.default_window_icon().cloned();
            let mut tray = TrayIconBuilder::with_id("main").menu(&menu).tooltip("Hitboxer");
            if let Some(icon) = icon {
                tray = tray.icon(icon);
            }
            tray.on_menu_event(|app, event| match event.id.as_ref() {
                "show" => reveal(app),
                // A run in progress is invisible until it goes wrong, so the
                // tray carries the stop switch for it as well.
                "stop" => {
                    let runtime: State<'_, RuntimeState> = app.state();
                    runtime.stop();
                }
                "quit" => {
                    save_shape(app);
                    app.exit(0);
                }
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                // Left click, and only on the way up. Matching every button —
                // which this did — swallows the right click that opens the menu,
                // and the window jumps back into focus instead of the menu
                // appearing, which looks exactly like a tray that is stuck.
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    reveal(tray.app_handle());
                }
            })
            .build(app)?;

            // Registered last, because the first check waits twenty seconds and
            // never blocks the tray coming up.
            app.manage(update::UpdateHolder::new());
            update::watch(app.handle().clone());

            Ok(())
        })
        .on_window_event(|window, event| {
            let is_picker = window.label() == picker::LABEL;
            match event {
                WindowEvent::CloseRequested { api, .. } if is_picker => {
                    // The picker is never really closed: it is hidden, and a
                    // half-finished pick is dropped rather than left waiting.
                    api.prevent_close();
                    picker::cancel(window.app_handle());
                }
                WindowEvent::CloseRequested { api, .. } => {
                    let state: State<'_, SharedState> = window.state();
                    let close_to_tray = state.lock().unwrap().settings.close_to_tray;
                    if close_to_tray {
                        api.prevent_close();
                        // From here the window cannot be moved or resized, so
                        // this is the shape it was last seen in.
                        save_shape(window.app_handle());
                        let _ = window.hide();
                    } else {
                        save_shape(window.app_handle());
                        window.app_handle().exit(0);
                    }
                }
                WindowEvent::Destroyed if is_picker => picker::closed(window.app_handle()),
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            set_tool_enabled,
            set_preferences,
            add_profile,
            remove_profile,
            set_profile_active,
            add_mapping,
            update_mapping,
            remove_mapping,
            apply_preset,
            running_processes,
            export_socd,
            import_socd,
            export_settings,
            import_settings,
            activity,
            save_clicker,
            start_clicker,
            stop_activity,
            dismiss_notices,
            save_macro,
            reorder_macros,
            delete_macro,
            run_macro,
            save_typer,
            start_typer,
            typer::typer_status,
            typer::typer_pause,
            typer::typer_resume,
            typer::typer_stop,
            typer::typer_plan,
            running_windows,
            choose_program,
            block_choices,
            replay::replay_state,
            replay::start_recording,
            replay::stop_recording,
            replay::start_replay,
            replay::stop_replay,
            replay::discard_recording,
            save_recording,
            delete_recording,
            undo_delete,
            select_recording,
            save_replay_options,
            set_autostart,
            choose_exe,
            pick_region,
            picker_context,
            save_picture,
            picture_history,
            picture_preview,
            delete_picture,
            forget_picture,
            picture_uses,
            picker_finish,
            picker_cancel,
            update::update_state,
            update::update_check,
            update::update_install
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hitboxer");
}
