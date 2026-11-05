//! What is running, and the shortcuts that start it.
//!
//! One run at a time, whether it is the auto clicker or a macro: they all write
//! to the same desktop, and two at once would fight over the cursor and the
//! keyboard. The window polls [`Runtime::view`] for live counters instead of the
//! runs pushing events, which keeps both engines free of Tauri.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::clicker::{self, ClickerSettings, Outcome, Progress, WindowsMachine};
use crate::config::Settings;
use crate::hook::{self, SharedState};
use crate::macros::{self, Macro, Report, WindowsRunner};
use crate::mouse_hotkeys;

/// The name a clicker run is reported under.
pub const CLICKER: &str = "clicker";

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActivityView {
    /// `None`, `Some("clicker")`, or the name of the macro that is running.
    pub running: Option<String>,
    pub clicks: i64,
    pub elapsed_secs: f64,
    /// Clicks per second actually achieved, measured over the run so far.
    pub achieved_cps: f64,
    pub requested_cps: f64,
    /// What the clicker is pressing, for the status line.
    pub source: String,
    /// Whether Windows granted the 1 ms timer for this run.
    pub timer_raised: bool,
    pub last: Option<Outcome>,
    pub last_macro: Option<Report>,
    /// Shortcuts that could not be taken, with the reason.
    pub hotkey_errors: Vec<String>,
    /// Something that was asked for and could not happen: a replay with no
    /// recording chosen, a run refused because another one holds the desktop.
    /// Printed in the same strip as a refused shortcut, because a key that does
    /// nothing and says nothing is the failure both of them share.
    pub alerts: Vec<String>,
}

#[derive(Default)]
struct RunState {
    running: Option<String>,
    requested_cps: f64,
    source: String,
    timer_raised: bool,
    finished_clicks: i64,
    finished_elapsed: f64,
    last: Option<Outcome>,
    last_macro: Option<Report>,
    hotkey_errors: Vec<String>,
    alerts: Vec<String>,
}

pub struct Runtime {
    progress: Arc<Progress>,
    stop: Mutex<Option<Arc<AtomicBool>>>,
    state: Mutex<RunState>,
}

/// The one runtime, shared by the commands and by the background threads.
pub type RuntimeState = Arc<Runtime>;

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            progress: Arc::new(Progress::default()),
            stop: Mutex::new(None),
            state: Mutex::new(RunState::default()),
        }
    }

    pub fn running(&self) -> Option<String> {
        self.state.lock().unwrap().running.clone()
    }

    /// Stop whatever is going. Returns whether there was anything to stop.
    pub fn stop(&self) -> bool {
        match self.stop.lock().unwrap().clone() {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn view(&self) -> ActivityView {
        let state = self.state.lock().unwrap();
        let live = state.running.as_deref() == Some(CLICKER);
        let clicks = if live {
            self.progress.clicks()
        } else {
            state.finished_clicks
        };
        let elapsed_secs = if live {
            self.progress.elapsed_secs()
        } else {
            state.finished_elapsed
        };

        ActivityView {
            running: state.running.clone(),
            clicks,
            elapsed_secs,
            achieved_cps: if elapsed_secs > 0.0 {
                clicks as f64 / elapsed_secs
            } else {
                0.0
            },
            requested_cps: state.requested_cps,
            source: state.source.clone(),
            timer_raised: state.timer_raised,
            last: state.last.clone(),
            last_macro: state.last_macro.clone(),
            hotkey_errors: state.hotkey_errors.clone(),
            alerts: state.alerts.clone(),
        }
    }

    /// Say that something could not happen where the window will show it.
    pub fn alert(&self, message: impl Into<String>) {
        let message = message.into();
        let mut state = self.state.lock().unwrap();
        if !state.alerts.iter().any(|existing| existing == &message) {
            state.alerts.push(message);
            if state.alerts.len() > 4 {
                state.alerts.remove(0);
            }
        }
    }

    /// Clear the strip: refusals the person has read, not settings. Anything
    /// asked for again after this is a fresh answer.
    pub fn dismiss(&self) {
        let mut state = self.state.lock().unwrap();
        state.alerts.clear();
        state.hotkey_errors.clear();
    }

    /// Claim the desktop for one run, or explain who has it.
    pub(crate) fn claim(&self, label: &str) -> Result<Arc<AtomicBool>, String> {
        let mut state = self.state.lock().unwrap();
        // Starting something makes whatever was refused before it stale.
        state.alerts.clear();
        if let Some(running) = state.running.clone() {
            return Err(match running.as_str() {
                CLICKER => "the auto clicker is already running".to_string(),
                crate::typer::TYPER => "the typer is already running".to_string(),
                crate::replay::REPLAY => "a replay is already running".to_string(),
                _ => format!("the macro “{running}” is running"),
            });
        }
        let stop = Arc::new(AtomicBool::new(false));
        *self.stop.lock().unwrap() = Some(Arc::clone(&stop));
        state.running = Some(label.to_string());
        Ok(stop)
    }

    /// Hand the desktop back, with the numbers the run ended on.
    pub(crate) fn release(&self, clicks: i64, elapsed_secs: f64) {
        let mut state = self.state.lock().unwrap();
        state.running = None;
        state.finished_clicks = clicks;
        state.finished_elapsed = elapsed_secs;
        *self.stop.lock().unwrap() = None;
    }
}

/// Start a clicker run on its own thread.
pub fn start_clicker(runtime: &Arc<Runtime>, settings: ClickerSettings) -> Result<(), String> {
    clicker::validate(&settings)?;
    let stop = runtime.claim(CLICKER)?;

    {
        let mut state = runtime.state.lock().unwrap();
        state.requested_cps = clicker::clicks_per_second(&settings);
        state.source = clicker::Source::of(&settings).label();
        state.timer_raised = false;
        state.last = None;
    }
    runtime.progress.begin();

    let shared = Arc::clone(runtime);
    std::thread::Builder::new()
        .name("hitboxer-clicker".into())
        .spawn(move || {
            let mut machine = WindowsMachine::new();
            let progress = Arc::clone(&shared.progress);
            let outcome = clicker::with_timer(|raised| {
                shared.state.lock().unwrap().timer_raised = raised;
                clicker::run(&settings, &mut machine, Arc::clone(&stop), &progress)
            });
            let mut state = shared.state.lock().unwrap();
            state.requested_cps = outcome.requested_cps;
            state.last = Some(outcome.clone());
            drop(state);
            shared.release(outcome.clicks, outcome.elapsed_secs);
        })
        .map_err(|error| {
            runtime.release(0, 0.0);
            format!("could not start the clicker thread: {error}")
        })?;

    Ok(())
}

/// Run a macro on its own thread.
pub fn start_macro(
    runtime: &Arc<Runtime>,
    app: &AppHandle,
    definition: &Macro,
) -> Result<(), String> {
    macros::validate(definition)?;
    if !definition.enabled {
        return Err(format!("“{}” is switched off", definition.name));
    }
    let stop = runtime.claim(&definition.name)?;

    runtime.state.lock().unwrap().last_macro = None;

    let steps = definition.steps.clone();
    let name = definition.name.clone();
    let app = app.clone();
    let shared = Arc::clone(runtime);
    std::thread::Builder::new()
        .name("hitboxer-macro".into())
        .spawn(move || {
            // The list a called macro or a recording is looked up in, read from
            // the app rather than threaded through every caller of start_macro.
            let settings = app.state::<SharedState>().inner().clone();
            let mut runner = WindowsRunner::new(app, settings);
            let report = macros::play(&steps, &mut runner, Arc::clone(&stop));
            shared.state.lock().unwrap().last_macro = Some(Report {
                stop_reason: format!("{name}: {}", report.stop_reason),
                actions: report.actions,
                notes: report.notes,
            });
            shared.release(0, 0.0);
        })
        .map_err(|error| {
            runtime.release(0, 0.0);
            format!("could not start the macro thread: {error}")
        })?;

    Ok(())
}

/// Rewrite the registered shortcuts from the settings.
///
/// Call this after any edit to a shortcut, with the settings lock released: it
/// takes that lock itself when a shortcut fires.
pub fn register_shortcuts(runtime: &Arc<Runtime>, app: &AppHandle, settings: &Settings) {
    let shortcuts = app.global_shortcut();
    // Taking them all back first means an edited combination does not leave the
    // previous one live.
    let _ = shortcuts.unregister_all();
    let mut errors: Vec<String> = Vec::new();

    // Mouse buttons cannot be taken by the global shortcut layer, so they are
    // collected here and handed to the mouse hook instead. Everything that is
    // not a mouse button goes through the layer as before.
    let mut mouse: Vec<mouse_hotkeys::Binding> = Vec::new();

    // A tool that is switched off owns no shortcuts, so its keys stay free for
    // everything else on the machine.
    if settings.clicker_enabled && !settings.clicker.hotkey.trim().is_empty() {
        let accelerator = settings.clicker.hotkey.clone();
        let hold = settings.clicker.hold_mode;
        match mouse_hotkeys::parse(&accelerator) {
            Some((button, mods)) => {
                // A notch happens once and is over; "held" is not something it
                // can mean, and the binding would sit there doing nothing.
                if button.is_wheel() && hold {
                    errors.push(format!(
                        "the auto clicker shortcut “{accelerator}” is a wheel notch, which cannot be \
                         held down — switch the activation to Toggle, or pick a mouse button"
                    ));
                } else {
                    mouse.push(mouse_hotkeys::Binding {
                        button,
                        mods,
                        action: mouse_hotkeys::Action::Clicker { hold },
                    });
                }
            }
            None => match Shortcut::from_str(&accelerator) {
                Ok(shortcut) => {
                    let runtime = Arc::clone(runtime);
                    let result = shortcuts.on_shortcut(shortcut, move |app, _, event| {
                        clicker_shortcut(app, &runtime, hold, event.state == ShortcutState::Pressed)
                    });
                    if let Err(error) = result {
                        errors.push(format!("the auto clicker shortcut “{accelerator}”: {error}"));
                    }
                }
                Err(error) => errors.push(format!(
                    "the auto clicker shortcut “{accelerator}” is not one I understand ({error})"
                )),
            },
        }
    }

    // The typer's start/stop shortcut. It shares the one run slot, so pressing
    // it while the clicker is going is refused with a message rather than
    // quietly doing nothing.
    if settings.typer_enabled && !settings.typer.hotkey.trim().is_empty() {
        let accelerator = settings.typer.hotkey.clone();
        if mouse_hotkeys::parse(&accelerator).is_some() {
            errors.push(format!(
                "the typer shortcut “{accelerator}” must be a key combination: a mouse button \
                 cannot start typing"
            ));
        } else {
            match Shortcut::from_str(&accelerator) {
                Ok(shortcut) => {
                    let runtime = Arc::clone(runtime);
                    let result = shortcuts.on_shortcut(shortcut, move |app, _, event| {
                        if event.state != ShortcutState::Pressed {
                            return;
                        }
                        typer_shortcut(app, &runtime);
                    });
                    if let Err(error) = result {
                        errors.push(format!("the typer shortcut “{accelerator}”: {error}"));
                    }
                }
                Err(error) => errors.push(format!(
                    "the typer shortcut “{accelerator}” is not one I understand ({error})"
                )),
            }
        }
    }

    // The replayer's two shortcuts. Recording and replaying are separate
    // toggles, and each is one key: press it to begin, press it again to end.
    if settings.replay_enabled {
        let pairs = [
            ("record", settings.replay.record_hotkey.clone()),
            ("replay", settings.replay.replay_hotkey.clone()),
        ];
        for (which, accelerator) in pairs {
            if accelerator.trim().is_empty() {
                continue;
            }
            match mouse_hotkeys::parse(&accelerator) {
                Some((button, mods)) => mouse.push(mouse_hotkeys::Binding {
                    button,
                    mods,
                    action: if which == "record" {
                        mouse_hotkeys::Action::Record
                    } else {
                        mouse_hotkeys::Action::Replay
                    },
                }),
                None => match Shortcut::from_str(&accelerator) {
                    Ok(shortcut) => {
                        let runtime = Arc::clone(runtime);
                        let result = shortcuts.on_shortcut(shortcut, move |app, _, event| {
                            if event.state != ShortcutState::Pressed {
                                return;
                            }
                            if which == "record" {
                                crate::replay::record_shortcut(app, &runtime);
                            } else {
                                crate::replay::replay_shortcut(app, &runtime);
                            }
                        });
                        if let Err(error) = result {
                            errors.push(format!("the {which} shortcut “{accelerator}”: {error}"));
                        }
                    }
                    Err(error) => errors.push(format!(
                        "the {which} shortcut “{accelerator}” is not one I understand ({error})"
                    )),
                },
            }
        }
    }

    for definition in settings
        .macros
        .iter()
        .filter(|entry| settings.macros_enabled && entry.enabled && !entry.hotkey.trim().is_empty())
    {
        let accelerator = definition.hotkey.clone();
        let id = definition.id.clone();
        let name = definition.name.clone();
        match mouse_hotkeys::parse(&accelerator) {
            Some((button, mods)) => mouse.push(mouse_hotkeys::Binding {
                button,
                mods,
                action: mouse_hotkeys::Action::Macro { id, name },
            }),
            None => match Shortcut::from_str(&accelerator) {
                Ok(shortcut) => {
                    let runtime = Arc::clone(runtime);
                    // The closure owns its own copy of the name, so the message
                    // below can still use the original.
                    let fired_name = name.clone();
                    let result = shortcuts.on_shortcut(shortcut, move |app, _, event| {
                        if event.state != ShortcutState::Pressed {
                            return;
                        }
                        macro_shortcut(app, &runtime, &id, &fired_name);
                    });
                    if let Err(error) = result {
                        errors.push(format!("the “{name}” shortcut “{accelerator}”: {error}"));
                    }
                }
                Err(error) => errors.push(format!(
                    "the “{name}” shortcut “{accelerator}” is not one I understand ({error})"
                )),
            },
        }
    }

    mouse_hotkeys::set(app, mouse);
    runtime.state.lock().unwrap().hotkey_errors = errors;
}

/// The clicker's shortcut fired, from the keyboard or from a mouse button.
///
/// Shared by both paths so the two can never drift: a toggle that starts when
/// the keyboard is used and stops when the mouse is used would be a bug nobody
/// could reproduce.
pub fn clicker_shortcut(app: &AppHandle, runtime: &Arc<Runtime>, hold: bool, down: bool) {
    match (hold, down) {
        (false, true) => {
            if runtime.running().as_deref() == Some(CLICKER) {
                runtime.stop();
            } else {
                clicker_from_settings(app, runtime);
            }
        }
        (true, true) => clicker_from_settings(app, runtime),
        (true, false) => {
            runtime.stop();
        }
        (false, false) => {}
    }
}

/// The typer's shortcut fired: stop a run that is going, or start the draft.
///
/// The text is read as it is now, so an edit takes effect without a restart.
pub fn typer_shortcut(app: &AppHandle, runtime: &Arc<Runtime>) {
    if runtime.running().as_deref() == Some(crate::typer::TYPER) {
        runtime.stop();
        return;
    }
    let settings = {
        let state = app.state::<SharedState>();
        let shared = state.lock().unwrap();
        if !shared.settings.typer_enabled {
            return;
        }
        shared.settings.typer.clone()
    };
    let holder = app.state::<crate::typer::Holder>();
    if let Err(error) = crate::typer::start(runtime, &holder, &settings) {
        holder.current().stopped();
        runtime.state.lock().unwrap().last_macro = Some(Report {
            stop_reason: format!("typer: {error}"),
            actions: 0,
            notes: Vec::new(),
        });
    }
}

/// A macro's shortcut fired. Read as it is now, so an edit takes effect without
/// a restart, and honour its target program.
pub fn macro_shortcut(app: &AppHandle, runtime: &Arc<Runtime>, id: &str, name: &str) {
    let Some(current) = current_macro(app, id) else {
        return;
    };
    if !macros::should_run(&current, &hook::foreground_program()) {
        return;
    }
    if let Err(error) = start_macro(runtime, app, &current) {
        runtime.state.lock().unwrap().last_macro = Some(Report {
            stop_reason: format!("{name}: {error}"),
            actions: 0,
            notes: Vec::new(),
        });
    }
}

/// Flip one of the app's switches, whether the window asked or a block did.
///
/// One implementation for both, because the parts that matter are the parts
/// that are easy to forget: saving the file, letting go of keys the remapper is
/// holding, stopping the run that belongs to the tool being switched off, and
/// re-registering shortcuts so a switched-off tool cannot start from a key it
/// no longer owns.
///
/// `exempt` is the run that asked, if a run did. Switching a tool off does not
/// stop the run that is doing the switching: a macro that turns macros off has
/// said something about the next run, not about itself.
pub fn apply_tool(
    app: &AppHandle,
    state: &SharedState,
    runtime: &Arc<Runtime>,
    tool: &str,
    enabled: bool,
    exempt: Option<&str>,
) -> Result<(), String> {
    if !matches!(tool, "clicker" | "remapper" | "macros" | "typer" | "replay") {
        return Err(format!("there is no tool called “{tool}”"));
    }
    let (settings, actions) = {
        let mut shared = state.lock().unwrap();
        let actions = match tool {
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
            _ => {
                shared.settings.replay_enabled = enabled;
                Vec::new()
            }
        };
        crate::save(app, &shared);
        (shared.settings.clone(), actions)
    };
    hook::send_public(&actions);

    if !enabled {
        let running = runtime.running();
        let stop_it = match tool {
            "clicker" => running.as_deref() == Some(CLICKER),
            "typer" => running.as_deref() == Some(crate::typer::TYPER),
            "replay" => running.as_deref() == Some(crate::replay::REPLAY),
            // A macro in progress is nobody else's business: the named runs
            // above are excluded so switching the macros off leaves them alone.
            "macros" => matches!(running.as_deref(), Some(name)
                if name != CLICKER
                    && name != crate::typer::TYPER
                    && name != crate::replay::REPLAY),
            _ => false,
        };
        if stop_it && running.as_deref() != exempt {
            runtime.stop();
        }
    }

    register_shortcuts(runtime, app, &settings);
    Ok(())
}

/// Start the clicker from whatever the settings say now.
fn clicker_from_settings(app: &AppHandle, runtime: &Arc<Runtime>) {
    let settings = {
        let state = app.state::<SharedState>();
        let shared = state.lock().unwrap();
        shared.settings.clicker.clone()
    };
    if let Err(error) = start_clicker(runtime, settings) {
        runtime.state.lock().unwrap().last = Some(Outcome {
            stop_reason: error,
            ..Outcome::default()
        });
    }
}

/// Read a macro as it is stored now.
fn current_macro(app: &AppHandle, id: &str) -> Option<Macro> {
    let state = app.state::<SharedState>();
    let shared = state.lock().unwrap();
    shared
        .settings
        .macros
        .iter()
        .find(|entry| entry.id == id)
        .cloned()
}
