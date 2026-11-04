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
mod config;
mod hook;
mod input;
mod keys;
mod macros;
mod mouse_hotkeys;
mod picker;
mod replay;
mod runtime;
mod socd;
mod typer;
mod update;

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
struct ReplayView {
    recordings: Vec<RecordingView>,
    selected: String,
    record_hotkey: String,
    replay_hotkey: String,
    /// `None` is forever.
    repeat: Option<u32>,
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
fn load(app: &AppHandle) -> (Settings, Vec<String>) {
    let Some(path) = settings_path(app) else {
        return (Settings::default(), Vec::new());
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return (Settings::default(), Vec::new());
    };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(document) => Settings::from_document(&document),
        Err(error) => (
            Settings::default(),
            vec![format!(
                "the saved settings could not be read ({error}); starting from the defaults"
            )],
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
        shared.settings.macros.retain(|entry| entry.id != id);
        save(&app, &shared);
        shared.settings.clone()
    };
    runtime::register_shortcuts(&runtime, &app, &settings);
    snapshot(state)
}

#[tauri::command]
fn run_macro(
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
    runtime::start_macro(&runtime, &definition)?;
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
        shared.settings.replay.recordings.retain(|entry| entry.id != id);
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
) -> View {
    {
        let mut shared = state.lock().unwrap();
        shared.settings.replay.record_hotkey = record_hotkey.trim().to_string();
        shared.settings.replay.replay_hotkey = replay_hotkey.trim().to_string();
        shared.settings.replay.repeat = repeat;
        save(&app, &shared);
    }
    let settings = state.lock().unwrap().settings.clone();
    runtime::register_shortcuts(&runtime, &app, &settings);
    snapshot(state)
}

/// Turn one tool on or off.
///
/// `tool` is `remapper`, `clicker` or `macros`. Switching a tool off also stops
/// what it was doing: a disabled clicker that kept clicking, or a remapper left
/// holding keys down, would be worse than the global pause this replaced.
#[tauri::command]
fn set_tool_enabled(
    app: AppHandle,
    state: State<'_, SharedState>,
    runtime: State<'_, RuntimeState>,
    tool: String,
    enabled: bool,
) -> View {
    let (settings, actions) = {
        let mut shared = state.lock().unwrap();
        let actions = match tool.as_str() {
            "remapper" => {
                shared.settings.hook_enabled = enabled;
                if enabled {
                    Vec::new()
                } else {
                    shared.engine.release_all()
                }
            }
            "clicker" => {
                shared.settings.clicker_enabled = enabled;
                Vec::new()
            }
            "macros" => {
                shared.settings.macros_enabled = enabled;
                Vec::new()
            }
            "typer" => {
                shared.settings.typer_enabled = enabled;
                Vec::new()
            }
            "replay" => {
                shared.settings.replay_enabled = enabled;
                Vec::new()
            }
            _ => Vec::new(),
        };
        save(&app, &shared);
        (shared.settings.clone(), actions)
    };
    hook::send_public(&actions);

    // Only the run belonging to the tool being switched off is stopped; a macro
    // in progress is not the clicker's business.
    if !enabled {
        let running = runtime.running();
        let stop_it = match tool.as_str() {
            "clicker" => running.as_deref() == Some(runtime::CLICKER),
            "typer" => running.as_deref() == Some(typer::TYPER),
            "replay" => running.as_deref() == Some(replay::REPLAY),
            // A macro in progress is nobody else's business: the named runs
            // above are excluded so switching the macros off leaves them alone.
            "macros" => matches!(running.as_deref(), Some(name)
                if name != runtime::CLICKER
                    && name != typer::TYPER
                    && name != replay::REPLAY),
            _ => false,
        };
        if stop_it {
            runtime.stop();
        }
    }

    // The shortcuts follow the switches: a switched-off tool must not start
    // from a key it no longer owns.
    runtime::register_shortcuts(&runtime, &app, &settings);
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
                target.remove(index);
                save(&app, &shared);
            }
        }
    }
    retarget(&state);
    snapshot(state)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Registered first, as the plugin requires. A second launch hands its
        // arguments to this callback and exits, so only one process ever owns
        // the keyboard hook.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            reveal(app);
        }))
        // Registered before setup, because the shortcuts are taken in setup.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // Used from Rust only, to let the user point the sign-in entry at a copy
        // of the app rather than type its path.
        .plugin(tauri_plugin_dialog::init())
        // The release endpoint, checked after launch and every six hours. The
        // state the banner reads lives in `update`.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let (settings, notes) = load(app.handle());
            let shared: SharedState = Arc::new(Mutex::new(Shared::new(settings.clone())));
            app.manage(shared.clone());
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
                "quit" => app.exit(0),
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
                        let _ = window.hide();
                    } else {
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
            save_macro,
            delete_macro,
            run_macro,
            save_typer,
            start_typer,
            typer::typer_status,
            typer::typer_pause,
            typer::typer_resume,
            typer::typer_stop,
            typer::typer_plan,
            replay::replay_state,
            replay::start_recording,
            replay::stop_recording,
            replay::start_replay,
            replay::stop_replay,
            replay::discard_recording,
            save_recording,
            delete_recording,
            select_recording,
            save_replay_options,
            set_autostart,
            choose_exe,
            pick_region,
            picker_context,
            picker_finish,
            picker_cancel,
            update::update_state,
            update::update_check,
            update::update_install
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hitboxer");
}
