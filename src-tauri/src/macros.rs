//! Macros: the AutoHotkey idea — a hotkey that runs a sequence of input — with
//! a block builder in front of it instead of a scripting language.
//!
//! A macro is a list of [`Step`]s, and a step may be a [`Step::Repeat`] holding
//! its own list, which is what lets the UI show a nestable block tree. The
//! interpreter walks it and talks to a [`Runner`], so the whole engine is
//! testable against a recording runner with no keyboard attached.
//!
//! Two guard rails matter more than they look. Nesting is depth-capped and every
//! repeat count is clamped, so a typo cannot describe a loop that runs for a
//! century; and the interpreter tracks the keys it pressed with [`PressMode::Down`]
//! and releases them on the way out, so stopping a macro halfway through a chord
//! does not leave a key stuck down.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::input::MouseButton;

/// Deepest a `Repeat` may be nested. Enforced when editing as well as running.
pub const MAX_DEPTH: usize = 6;
/// Highest repeat count a single block may carry.
pub const MAX_TIMES: u32 = 9_999;
/// Actions one run may take, counting each key press, character and click. This
/// is a runaway guard, not a limit anyone should meet: a five-minute macro of
/// typed text is a few thousand.
pub const ACTION_BUDGET: u64 = 2_000_000;

/// Longest text a note may carry. It is documentation, not a log.
pub const NOTE_CHARS: usize = 2_000;
/// Longest line a log block may write, in characters.
pub const LOG_CHARS: usize = 200;
/// How many log lines one run keeps: a loop with a log in it must not fill memory.
pub const MAX_NOTES: usize = 200;
/// Most keys one chord may hold at once.
pub const CHORD_KEYS: usize = 4;

/// A small, fast, seedable generator for the blocks that need a random number.
///
/// Seeded once per run rather than per call, so two runs of the same macro
/// differ, and so a test can pin the seed and get the same answers every time.
/// xorshift64* is plenty here: this picks a pause between two numbers, not
/// anything that needs to resist being guessed.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // A zero state is a fixed point of xorshift: it would answer zero for
        // ever, so a zero seed is replaced rather than accepted.
        Self(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    pub fn next(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.0 = state;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number between the two bounds, in either order.
    pub fn between(&mut self, min: u32, max: u32) -> u32 {
        let (low, high) = if min <= max { (min, max) } else { (max, min) };
        let span = u64::from(high - low) + 1;
        low + (self.next() % span) as u32
    }
}

/// What a press block does with what it presses: tap it, hold it, leave it
/// down, or release one that is already down.
///
/// The key block and the click block both offer tap/hold/down/up, so they share
/// the type — and the JSON — rather than each keeping its own copy of the idea.
/// `Drag` is the click block's own fifth: [`validate`] refuses it on a key,
/// which has nowhere to go.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum PressMode {
    /// Press and release straight away.
    #[default]
    Tap,
    /// Hold the key down for `hold_ms`, then release.
    Hold,
    /// Press and leave it down, for chords that span later blocks.
    Down,
    /// Release a key that an earlier block pressed.
    Up,
    /// Press, drag the cursor somewhere and let go: the whole click-and-drag in
    /// one block. Only the click block offers it.
    Drag,
}

/// What an `If` block checks.
///
/// Two conditions, because they are the two the app can already answer without
/// reaching outside itself: a picture it has been shown, and the program in
/// front, which the remapper's hook already tracks. Anything more — a window
/// title, a variable, a loop counter — is a scripting language growing back.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Condition {
    /// The picture is on screen right now.
    Picture {
        image: String,
        #[serde(default = "default_similarity")]
        similarity: u32,
    },
    /// This program is in front.
    Front { program: String },
}

/// What a Focus block is looking for.
///
/// Two ways, because a person describes a window one of two ways: by what it
/// says, or by which program it belongs to. Both are answers the app can give
/// itself — anything more specific would be an expression language arriving
/// through the side door.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WindowTarget {
    /// A window whose title contains this text — or is exactly it, with `exact`.
    Title {
        contains: String,
        #[serde(default)]
        exact: bool,
    },
    /// Any window belonging to this program, by executable name.
    Program { exe: String },
}

impl WindowTarget {
    /// Whether a window is the one this target means.
    ///
    /// Pure, so the matching can be tested without a desktop in front of it, and
    /// so the platform side stays a lookup rather than a second set of rules.
    /// Titles are compared without case because programs write them in whatever
    /// case they please.
    pub fn matches(&self, title: &str, exe: &str) -> bool {
        match self {
            WindowTarget::Title { contains, exact } => {
                let wanted = contains.trim().to_lowercase();
                if wanted.is_empty() {
                    return false;
                }
                let title = title.trim().to_lowercase();
                if *exact {
                    title == wanted
                } else {
                    title.contains(&wanted)
                }
            }
            WindowTarget::Program { exe: wanted } => {
                let wanted = wanted.trim();
                !wanted.is_empty() && exe.eq_ignore_ascii_case(wanted)
            }
        }
    }
}

/// One block.
///
/// `rename_all_fields` matters here and is not decoration: it renames the fields
/// of the variants, which plain `rename_all` does not. The window is camelCase
/// throughout, so without it a hold time it sends as `holdMs` is a field this
/// never reads — the number silently falls back to its default and the box the
/// person typed it into looks broken.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Step {
    /// Press a key.
    Key {
        scan: u16,
        #[serde(default)]
        mode: PressMode,
        /// How long a `Hold` holds, in milliseconds.
        #[serde(default = "default_hold")]
        hold_ms: u32,
        /// Hold Shift with the key.
        #[serde(default)]
        uppercase: bool,
    },
    /// Type text, one character at a time.
    Text {
        text: String,
        #[serde(default)]
        per_char_ms: u32,
    },
    /// Move the cursor.
    Move {
        x: i32,
        y: i32,
        /// Move by this much rather than to this point.
        #[serde(default)]
        relative: bool,
        /// Travel time; 0 teleports.
        #[serde(default)]
        duration_ms: u32,
    },
    /// Click the mouse.
    Click {
        #[serde(default)]
        button: MouseButton,
        /// Tap it, hold it down for a moment, leave it down for a later block,
        /// or let go of one an earlier block pressed.
        #[serde(default)]
        mode: PressMode,
        /// How many taps a `Tap` makes. Only a tap repeats.
        #[serde(default = "default_count")]
        count: u32,
        /// The button's down time: how long each tap stays down, how long a
        /// single `Hold` holds, or how long a `Drag` rests where it landed
        /// before letting go.
        #[serde(default)]
        hold_ms: u32,
        /// Where a `Drag` drags to. Unused by the other modes.
        #[serde(default)]
        x: i32,
        #[serde(default)]
        y: i32,
        /// A `Drag` moves by this much rather than to this point. Relative by
        /// default, so a fresh drag block is a press and a release where the
        /// cursor already is, not a leap to the corner of the screen.
        #[serde(default = "default_relative")]
        relative: bool,
        /// How long a `Drag` takes to travel there; 0 jumps, as in a `Move`.
        #[serde(default = "default_drag_ms")]
        duration_ms: u32,
    },
    /// Scroll the wheel; negative scrolls down.
    Scroll { amount: i32 },
    /// Wait.
    Delay { ms: u32 },
    /// Look for a picture on screen and wait until it is there.
    WaitImage {
        image: String,
        /// How long to keep looking. Zero looks once.
        #[serde(default = "default_wait_ms")]
        timeout_ms: u32,
        /// How alike the screen has to look, as a percentage.
        #[serde(default = "default_similarity")]
        similarity: u32,
    },
    /// Find a picture on screen and click it, wherever it is now.
    ClickImage {
        image: String,
        #[serde(default)]
        button: MouseButton,
        /// How long to keep looking. Zero looks once.
        #[serde(default = "default_click_wait_ms")]
        timeout_ms: u32,
        #[serde(default = "default_similarity")]
        similarity: u32,
    },
    /// Run one of two sets of blocks, depending on what is on screen.
    ///
    /// The branches nest like a `Repeat` and count against the same depth cap,
    /// so a typo cannot describe an if inside an if forever.
    If {
        condition: Condition,
        #[serde(default)]
        steps: Vec<Step>,
        /// The blocks to run when the check is false.
        #[serde(default)]
        otherwise: Vec<Step>,
    },
    /// Run the blocks inside this one again, `times` times over.
    Repeat {
        #[serde(default = "default_times")]
        times: u32,
        #[serde(default)]
        steps: Vec<Step>,
    },
    /// A note for whoever reads the macro. Runs nothing, counts nothing.
    Note {
        #[serde(default)]
        text: String,
    },
    /// Write a line into the report, so a long macro says where it got to.
    Log {
        #[serde(default)]
        text: String,
    },
    /// Wait a random time between two values.
    RandomDelay {
        #[serde(default)]
        min_ms: u32,
        #[serde(default = "default_random_max")]
        max_ms: u32,
    },
    /// Press several keys as one chord and let go of them together.
    Chord {
        #[serde(default)]
        scans: Vec<u16>,
        #[serde(default = "default_hold")]
        hold_ms: u32,
    },
    /// Move the cursor somewhere and scroll there, as one block.
    ScrollAt {
        x: i32,
        y: i32,
        amount: i32,
    },
    /// Wait until a time of day. A not-before gate, not an alarm.
    Until {
        #[serde(default)]
        at: String,
    },
    /// Run the blocks inside this one until a number of seconds has passed.
    RepeatFor {
        #[serde(default = "default_seconds")]
        seconds: u32,
        #[serde(default)]
        steps: Vec<Step>,
    },
    /// End the run here, saying why.
    Stop {
        #[serde(default)]
        reason: String,
    },
    /// End the run as a failure, saying what went wrong.
    Fail {
        #[serde(default)]
        message: String,
    },
    /// The picture has to be on screen right now, or the run stops.
    AssertImage {
        image: String,
        #[serde(default = "default_similarity")]
        similarity: u32,
    },
    /// Bring a window in front, waiting up to `timeout_ms` for one to exist.
    Focus {
        target: WindowTarget,
        #[serde(default = "default_focus_ms")]
        timeout_ms: u32,
    },
    /// Start a program, and optionally wait until it has a window.
    Launch {
        #[serde(default)]
        path: String,
        /// Passed through verbatim: there is no shell here to expand anything.
        #[serde(default)]
        args: String,
        #[serde(default = "default_true")]
        wait_for_window: bool,
        #[serde(default = "default_launch_ms")]
        timeout_ms: u32,
    },
    /// Put text on the clipboard, replacing what was there.
    Clipboard {
        #[serde(default)]
        text: String,
    },
    /// Save an area of the screen as a PNG.
    Screenshot {
        #[serde(default)]
        x: i32,
        #[serde(default)]
        y: i32,
        #[serde(default = "default_shot_width")]
        width: i32,
        #[serde(default = "default_shot_height")]
        height: i32,
        #[serde(default)]
        name: String,
    },
    /// Run another macro's blocks here, under this run.
    CallMacro {
        #[serde(default)]
        id: String,
        /// Stored for display, so the block still reads well after a rename.
        #[serde(default)]
        name: String,
    },
    /// Play a saved recording inline.
    Replay {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default = "default_passes")]
        passes: u32,
        /// 0 means "whatever the replayer's own speed is".
        #[serde(default)]
        speed: f64,
    },
    /// Flip one of the app's switches.
    Tool {
        #[serde(default)]
        tool: String,
        #[serde(default = "default_true")]
        enabled: bool,
    },
    /// Repeat the inner blocks while the check holds, up to two caps.
    While {
        condition: Condition,
        #[serde(default)]
        steps: Vec<Step>,
        #[serde(default = "default_iterations")]
        max_iterations: u32,
        #[serde(default = "default_while_ms")]
        timeout_ms: u32,
    },
}

fn default_hold() -> u32 {
    60
}

fn default_relative() -> bool {
    true
}

fn default_drag_ms() -> u32 {
    200
}

fn default_count() -> u32 {
    1
}

fn default_times() -> u32 {
    2
}

fn default_wait_ms() -> u32 {
    5_000
}

fn default_click_wait_ms() -> u32 {
    3_000
}

fn default_similarity() -> u32 {
    90
}

fn default_random_max() -> u32 {
    2_000
}

fn default_seconds() -> u32 {
    30
}

fn default_focus_ms() -> u32 {
    5_000
}

fn default_launch_ms() -> u32 {
    15_000
}

fn default_shot_width() -> i32 {
    800
}

fn default_shot_height() -> i32 {
    450
}

fn default_true() -> bool {
    true
}

fn default_passes() -> u32 {
    1
}

fn default_iterations() -> u32 {
    100
}

fn default_while_ms() -> u32 {
    60_000
}

impl Step {
    /// One short line naming this block, used by the log and by tests.
    pub fn summary(&self) -> String {
        match self {
            Step::Key { scan, mode, hold_ms, uppercase } => match mode {
                PressMode::Tap => format!("{}{}", if *uppercase { "Shift+" } else { "" }, crate::keys::label(*scan)),
                PressMode::Hold => format!(
                    "{}{} held {hold_ms} ms",
                    if *uppercase { "Shift+" } else { "" },
                    crate::keys::label(*scan)
                ),
                PressMode::Down => format!("{} down", crate::keys::label(*scan)),
                PressMode::Up => format!("{} up", crate::keys::label(*scan)),
                // A key cannot be dragged; validate refuses it. Typing it is
                // the harmless reading if one arrives here anyway.
                PressMode::Drag => format!(
                    "{}{}",
                    if *uppercase { "Shift+" } else { "" },
                    crate::keys::label(*scan)
                ),
            },
            Step::Text { text, .. } => format!("type “{}”", text.chars().take(24).collect::<String>()),
            Step::Move { x, y, relative, duration_ms } => {
                if *relative {
                    format!("move by {x},{y}")
                } else if *duration_ms > 0 {
                    format!("glide to {x},{y} over {duration_ms} ms")
                } else {
                    format!("jump to {x},{y}")
                }
            }
            Step::Click { button, mode, count, hold_ms, x, y, relative, duration_ms } => {
                let button = button.label().to_lowercase();
                match mode {
                    PressMode::Tap if *hold_ms > 0 => {
                        format!("{button} ×{count}, held {hold_ms} ms")
                    }
                    PressMode::Tap => format!("{button} ×{count}"),
                    PressMode::Hold => format!("{button} held {hold_ms} ms"),
                    PressMode::Down => format!("{button} down"),
                    PressMode::Up => format!("{button} up"),
                    PressMode::Drag => {
                        let travel = if *relative {
                            format!("by {x},{y}")
                        } else {
                            format!("to {x},{y} over {duration_ms} ms")
                        };
                        match hold_ms {
                            0 => format!("drag {button} {travel}"),
                            rest => format!("drag {button} {travel}, resting {rest} ms"),
                        }
                    }
                }
            }
            Step::Scroll { amount } => format!("scroll {amount}"),
            Step::WaitImage {
                image,
                timeout_ms,
                ..
            } => match timeout_ms {
                0 => format!("look for {}", picture_name(image)),
                ms => format!("look for {} for up to {ms} ms", picture_name(image)),
            },
            Step::ClickImage {
                image,
                button,
                timeout_ms,
                ..
            } => {
                let wait = match timeout_ms {
                    0 => String::new(),
                    ms => format!(" for up to {ms} ms"),
                };
                format!(
                    "click {} with the {} button{wait}",
                    picture_name(image),
                    button.label().to_lowercase()
                )
            }
            Step::Delay { ms } => format!("wait {ms} ms"),
            Step::If {
                condition,
                steps,
                otherwise,
            } => {
                let check = match condition {
                    Condition::Picture { .. } => "if the picture is on screen".to_string(),
                    Condition::Front { program } => format!("if {program} is in front"),
                };
                match otherwise.len() {
                    0 => format!("{check}: {} blocks", steps.len()),
                    rest => format!("{check}: {} blocks, {rest} otherwise", steps.len()),
                }
            }
            Step::Repeat { times, steps } => format!("repeat {times} × {} blocks", steps.len()),
            Step::While {
                condition,
                steps,
                max_iterations,
                ..
            } => {
                let check = match condition {
                    Condition::Picture { .. } => "while the picture is on screen".to_string(),
                    Condition::Front { program } => format!("while {program} is in front"),
                };
                format!(
                    "{check}: {} blocks, at most {max_iterations} passes",
                    steps.len()
                )
            }
            Step::Note { text } => {
                let line: String = text.chars().take(40).collect();
                match line.trim() {
                    "" => "a note".to_string(),
                    _ => format!("note “{line}”"),
                }
            }
            Step::Log { text } => {
                let line: String = text.chars().take(40).collect();
                format!("log “{line}”")
            }
            Step::RandomDelay { min_ms, max_ms } => {
                format!("wait {min_ms}–{max_ms} ms, picked at random")
            }
            Step::Chord { scans, hold_ms } => {
                let keys: Vec<String> = scans.iter().map(|scan| crate::keys::label(*scan)).collect();
                format!("{} together, held {hold_ms} ms", keys.join("+"))
            }
            Step::ScrollAt { x, y, amount } => format!("scroll {amount} at {x},{y}"),
            Step::Until { at } => format!("wait until {at}"),
            Step::RepeatFor { seconds, steps } => {
                format!("run {} blocks for {seconds} s", steps.len())
            }
            Step::Stop { reason } => match reason.trim() {
                "" => "stop the macro".to_string(),
                why => format!("stop: “{why}”"),
            },
            Step::Fail { message } => format!("fail: “{}”", message.trim()),
            Step::AssertImage { .. } => "the picture has to be on screen".to_string(),
            Step::Focus { target, timeout_ms } => {
                let what = match target {
                    WindowTarget::Title { contains, exact } => {
                        if *exact {
                            format!("“{}”", contains.trim())
                        } else {
                            format!("a window titled “{}”", contains.trim())
                        }
                    }
                    WindowTarget::Program { exe } => exe.trim().to_string(),
                };
                match timeout_ms {
                    0 => format!("focus {what}"),
                    ms => format!("focus {what}, up to {ms} ms"),
                }
            }
            Step::Launch {
                path,
                args,
                wait_for_window,
                ..
            } => {
                let name = std::path::Path::new(path.trim())
                    .file_name()
                    .and_then(|file| file.to_str())
                    .unwrap_or(path.trim());
                let arguments = match args.trim() {
                    "" => String::new(),
                    given => format!(" {given}"),
                };
                if *wait_for_window {
                    format!("start {name}{arguments} and wait for its window")
                } else {
                    format!("start {name}{arguments}")
                }
            }
            Step::Clipboard { text } => format!("copy {} characters", text.chars().count()),
            Step::Screenshot {
                width,
                height,
                name,
                ..
            } => {
                let name = name.trim();
                if name.is_empty() {
                    format!("save a {width}×{height} screenshot")
                } else {
                    format!("save a {width}×{height} screenshot as “{name}”")
                }
            }
            Step::CallMacro { name, .. } => match name.trim() {
                "" => "run another macro".to_string(),
                name => format!("run “{name}”"),
            },
            Step::Replay {
                name,
                passes,
                speed,
                ..
            } => {
                let label = match name.trim() {
                    "" => "a recording".to_string(),
                    name => format!("“{name}”"),
                };
                let times = match passes {
                    1 => "once".to_string(),
                    many => format!("{many} times"),
                };
                let pace = if *speed > 0.0 {
                    format!(" at {speed}×")
                } else {
                    String::new()
                };
                format!("play {label} {times}{pace}")
            }
            Step::Tool { tool, enabled } => {
                let what = match tool.as_str() {
                    "clicker" => "the auto clicker",
                    "remapper" => "the remapper",
                    "macros" => "macros",
                    "typer" => "the typer",
                    "replay" => "the replayer",
                    other => other,
                };
                format!("turn {what} {}", if *enabled { "on" } else { "off" })
            }
        }
    }

}

/// A macro: a trigger, an optional target program, and the blocks to run.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Macro {
    pub id: String,
    pub name: String,
    /// Global shortcut that runs it, e.g. `Control+Alt+1`. Empty means no
    /// shortcut, so it can only be run from the window.
    #[serde(default)]
    pub hotkey: String,
    /// Run only while this program is in front. Empty means anywhere.
    #[serde(default)]
    pub only_when: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// Why a macro cannot be saved or run.
pub fn validate(macro_definition: &Macro) -> Result<(), String> {
    if macro_definition.name.trim().is_empty() {
        return Err("give the macro a name".to_string());
    }
    for step in &macro_definition.steps {
        check_step(step, 0)?;
    }
    Ok(())
}

fn check_step(step: &Step, depth: usize) -> Result<(), String> {
    let where_ = step.summary();
    match step {
        Step::If {
            condition,
            steps,
            otherwise,
        } => {
            if depth + 1 > MAX_DEPTH {
                return Err(format!(
                    "“{where_}” is nested deeper than {MAX_DEPTH} levels"
                ));
            }
            match condition {
                Condition::Picture {
                    image,
                    similarity,
                } => {
                    if image.trim().is_empty() {
                        return Err(format!("“{where_}” has no picture yet — pick one"));
                    }
                    if !(50..=100).contains(similarity) {
                        return Err(format!("“{where_}” needs a match between 50% and 100%"));
                    }
                }
                Condition::Front { program } => {
                    if program.trim().is_empty() {
                        return Err(format!("“{where_}” has no program chosen"));
                    }
                }
            }
            for inner in steps.iter().chain(otherwise.iter()) {
                check_step(inner, depth + 1)?;
            }
        }
        Step::While {
            condition,
            steps,
            max_iterations,
            timeout_ms,
        } => {
            if depth + 1 > MAX_DEPTH {
                return Err(format!(
                    "“{where_}” is nested deeper than {MAX_DEPTH} levels"
                ));
            }
            match condition {
                Condition::Picture {
                    image,
                    similarity,
                } => {
                    if image.trim().is_empty() {
                        return Err(format!("“{where_}” has no picture yet — pick one"));
                    }
                    if !(50..=100).contains(similarity) {
                        return Err(format!("“{where_}” needs a match between 50% and 100%"));
                    }
                }
                Condition::Front { program } => {
                    if program.trim().is_empty() {
                        return Err(format!("“{where_}” has no program chosen"));
                    }
                }
            }
            // Both caps are required. A loop whose only way out is a condition
            // that may never change is a hang waiting for a person to notice.
            if !(1..=MAX_ITERATIONS).contains(max_iterations) {
                return Err(format!(
                    "“{where_}” needs between 1 and {MAX_ITERATIONS} passes"
                ));
            }
            if !(1..=MAX_WHILE_MS).contains(timeout_ms) {
                return Err(format!(
                    "“{where_}” needs a time limit between 1 s and {} minutes",
                    MAX_WHILE_MS / 60_000
                ));
            }
            for inner in steps {
                check_step(inner, depth + 1)?;
            }
        }
        Step::Repeat { times, steps } => {
            if depth + 1 > MAX_DEPTH {
                return Err(format!(
                    "“{where_}” is nested deeper than {MAX_DEPTH} levels"
                ));
            }
            if *times == 0 {
                return Err(format!("“{where_}” needs a repeat count above zero"));
            }
            if *times > MAX_TIMES {
                return Err(format!("“{where_}” repeats more than {MAX_TIMES} times"));
            }
            for inner in steps {
                check_step(inner, depth + 1)?;
            }
        }
        Step::Key { scan, mode, hold_ms, .. } => {
            if *mode == PressMode::Drag {
                return Err(format!(
                    "“{where_}” drags, and a key has nowhere to go — drag with a click block"
                ));
            }
            if *scan == 0 {
                return Err(format!("“{where_}” has no key chosen"));
            }
            if *hold_ms > 60_000 {
                return Err(format!("“{where_}” holds for longer than a minute"));
            }
        }
        Step::Text { text, per_char_ms } => {
            if text.is_empty() {
                return Err("a text block with nothing in it does nothing".to_string());
            }
            if *per_char_ms > 5_000 {
                return Err(format!("“{where_}” types slower than one character every 5 s"));
            }
        }
        Step::Click { mode, count, hold_ms, duration_ms, .. } => {
            // Only a tap repeats, so only a tap needs a count: a `down` is one
            // press and an `up` is one release whatever the number says.
            if *mode == PressMode::Tap {
                if *count == 0 {
                    return Err(format!("“{where_}” clicks zero times"));
                }
                if *count > 100_000 {
                    return Err(format!("“{where_}” clicks more than 100 000 times"));
                }
            }
            if matches!(mode, PressMode::Tap | PressMode::Hold | PressMode::Drag)
                && *hold_ms > 60_000
            {
                return Err(format!("“{where_}” holds the button for longer than a minute"));
            }
            if *mode == PressMode::Drag && *duration_ms > 60_000 {
                return Err(format!("“{where_}” takes longer than a minute to drag"));
            }
        }
        Step::Delay { ms } => {
            if *ms > 3_600_000 {
                return Err(format!("“{where_}” waits for longer than an hour"));
            }
        }
        Step::Move { duration_ms, .. } => {
            if *duration_ms > 60_000 {
                return Err(format!("“{where_}” takes longer than a minute to travel"));
            }
        }
        Step::WaitImage {
            image,
            timeout_ms,
            similarity,
        } => {
            if image.trim().is_empty() {
                return Err(format!("“{where_}” has no picture yet — pick one"));
            }
            if *timeout_ms > 600_000 {
                return Err(format!("“{where_}” waits for longer than ten minutes"));
            }
            if !(50..=100).contains(similarity) {
                return Err(format!("“{where_}” needs a match between 50% and 100%"));
            }
        }
        Step::ClickImage {
            image,
            timeout_ms,
            similarity,
            ..
        } => {
            if image.trim().is_empty() {
                return Err(format!("“{where_}” has no picture yet — pick one"));
            }
            if *timeout_ms > 600_000 {
                return Err(format!("“{where_}” waits for longer than ten minutes"));
            }
            if !(50..=100).contains(similarity) {
                return Err(format!("“{where_}” needs a match between 50% and 100%"));
            }
        }
        Step::Scroll { amount } => {
            if *amount == 0 {
                return Err(format!("“{where_}” scrolls nowhere"));
            }
        }
        Step::Note { text } => {
            if text.chars().count() > NOTE_CHARS {
                return Err(format!("“{where_}” is longer than {NOTE_CHARS} characters"));
            }
        }
        Step::Log { text } => {
            if text.trim().is_empty() {
                return Err("a log block with nothing in it says nothing".to_string());
            }
            if text.chars().count() > LOG_CHARS {
                return Err(format!("“{where_}” is longer than {LOG_CHARS} characters"));
            }
        }
        Step::RandomDelay { min_ms, max_ms } => {
            if *min_ms > 600_000 || *max_ms > 600_000 {
                return Err(format!("“{where_}” waits for longer than ten minutes"));
            }
        }
        Step::Chord { scans, hold_ms } => {
            if scans.is_empty() {
                return Err(format!("“{where_}” has no keys in it"));
            }
            if scans.len() > CHORD_KEYS {
                return Err(format!(
                    "“{where_}” holds more than {CHORD_KEYS} keys at once"
                ));
            }
            if scans.iter().any(|scan| *scan == 0) {
                return Err(format!("“{where_}” has an empty key in it"));
            }
            if *hold_ms > 60_000 {
                return Err(format!("“{where_}” holds the chord for longer than a minute"));
            }
        }
        Step::ScrollAt { amount, .. } => {
            if *amount == 0 {
                return Err(format!("“{where_}” scrolls nowhere"));
            }
        }
        Step::Until { at } => {
            parse_clock(at)
                .map_err(|why| format!("“{where_}” needs a time of day like 09:30 — {why}"))?;
        }
        Step::RepeatFor { seconds, steps } => {
            if depth + 1 > MAX_DEPTH {
                return Err(format!(
                    "“{where_}” is nested deeper than {MAX_DEPTH} levels"
                ));
            }
            if *seconds == 0 {
                return Err(format!("“{where_}” runs for no time at all"));
            }
            if *seconds > 3_600 {
                return Err(format!("“{where_}” runs for longer than an hour"));
            }
            for inner in steps {
                check_step(inner, depth + 1)?;
            }
        }
        Step::Stop { reason } => {
            if reason.chars().count() > LOG_CHARS {
                return Err(format!("“{where_}” is longer than {LOG_CHARS} characters"));
            }
        }
        Step::Fail { message } => {
            if message.trim().is_empty() {
                return Err("a failure block has to say what went wrong".to_string());
            }
            if message.chars().count() > LOG_CHARS {
                return Err(format!("“{where_}” is longer than {LOG_CHARS} characters"));
            }
        }
        Step::AssertImage {
            image,
            similarity,
        } => {
            if image.trim().is_empty() {
                return Err(format!("“{where_}” has no picture yet — pick one"));
            }
            if !(50..=100).contains(similarity) {
                return Err(format!("“{where_}” needs a match between 50% and 100%"));
            }
        }
        Step::Focus { target, timeout_ms } => {
            match target {
                WindowTarget::Title { contains, .. } if contains.trim().is_empty() => {
                    return Err(format!("“{where_}” has no window title to look for"));
                }
                WindowTarget::Program { exe } if exe.trim().is_empty() => {
                    return Err(format!("“{where_}” has no program to look for"));
                }
                _ => {}
            }
            if *timeout_ms > 120_000 {
                return Err(format!(
                    "“{where_}” waits for a window for longer than two minutes"
                ));
            }
        }
        Step::Launch {
            path,
            args,
            timeout_ms,
            ..
        } => {
            let path = path.trim();
            if path.is_empty() {
                return Err(format!("“{where_}” has no program chosen"));
            }
            if !std::path::Path::new(path).is_absolute() {
                return Err(format!("“{where_}” needs the program's full path"));
            }
            if args.chars().count() > 4_096 {
                return Err(format!(
                    "“{where_}” has more arguments than a command line holds"
                ));
            }
            if *timeout_ms > 120_000 {
                return Err(format!(
                    "“{where_}” waits for a window for longer than two minutes"
                ));
            }
            // Checked while editing rather than when the run reaches it: a
            // program that has been moved should be caught by the person who
            // moved it, not by a macro at six in the morning.
            if !std::path::Path::new(path).exists() {
                return Err(format!(
                    "“{where_}” points at a program that is not there any more"
                ));
            }
        }
        Step::Clipboard { text } => {
            if text.chars().count() > 100_000 {
                return Err(format!("“{where_}” copies more than 100 000 characters"));
            }
        }
        Step::Screenshot {
            width,
            height,
            name,
            ..
        } => {
            if *width <= 0 || *height <= 0 {
                return Err(format!("“{where_}” has no area to capture"));
            }
            if *width > 8_000 || *height > 8_000 {
                return Err(format!("“{where_}” is bigger than any screen"));
            }
            if name.chars().count() > 60 {
                return Err(format!("“{where_}” has a name longer than 60 characters"));
            }
        }
        Step::CallMacro { id, name } => {
            if id.trim().is_empty() {
                return Err(format!("“{where_}” has no macro chosen yet"));
            }
            if name.chars().count() > 100 {
                return Err(format!("“{where_}” has a name longer than 100 characters"));
            }
        }
        Step::Replay {
            id,
            passes,
            speed,
            ..
        } => {
            if id.trim().is_empty() {
                return Err(format!("“{where_}” has no recording chosen yet"));
            }
            if !(1..=10_000).contains(passes) {
                return Err(format!("“{where_}” plays between 1 and 10 000 times"));
            }
            // Zero is the replayer's own speed; anything else has to be a pace
            // that means something, so a stray 0.01 cannot play a recording
            // faster than the desktop can keep up with.
            if *speed != 0.0 && !(0.1..=10.0).contains(speed) {
                return Err(format!(
                    "“{where_}” plays at a speed between 0.1× and 10×, or 0 to use the replayer's"
                ));
            }
        }
        Step::Tool { tool, .. } => {
            if !matches!(
                tool.as_str(),
                "clicker" | "remapper" | "macros" | "typer" | "replay"
            ) {
                return Err(format!(
                    "“{where_}” names a tool that does not exist: “{tool}”"
                ));
            }
        }
    }
    Ok(())
}

/// Should this macro run, given what is in front?
pub fn should_run(macro_definition: &Macro, foreground: &str) -> bool {
    if !macro_definition.enabled {
        return false;
    }
    if macro_definition.only_when.trim().is_empty() {
        return true;
    }
    macro_definition
        .only_when
        .trim()
        .eq_ignore_ascii_case(foreground.trim())
}

/// How a picture is named in a line of text. Its file name is a timestamp,
/// which means nothing to a person, so the line says "the picture" and the
/// block itself shows the thumbnail.
fn picture_name(_image: &str) -> &'static str {
    "the picture"
}

/// Keep looking until the picture is on screen, the time is up, or the run is
/// stopped.
fn wait_for_picture(
    runner: &mut dyn Runner,
    image: &str,
    similarity: u32,
    timeout_ms: u32,
    stop: &AtomicBool,
) -> Result<(i32, i32), String> {
    let started = Instant::now();
    let limit = Duration::from_millis(u64::from(timeout_ms.min(600_000)));
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err("Stopped".to_string());
        }
        if let Some(at) = runner.find_image(image, similarity) {
            return Ok(at);
        }
        if started.elapsed() >= limit {
            return Err(match timeout_ms {
                0 => "the picture is not on screen".to_string(),
                ms => format!(
                    "the picture did not appear within {:.1} s",
                    f64::from(ms) / 1000.0
                ),
            });
        }
        // Looked for again a beat later rather than in a tight loop: the
        // capture costs a few milliseconds and the screen does not change
        // between two reads of it.
        if !runner.wait(120, stop) {
            return Err("Stopped".to_string());
        }
    }
}

/// Everything the interpreter needs from the operating system.
pub trait Runner {
    fn tap_key(&mut self, scan: u16, uppercase: bool);
    fn key(&mut self, scan: u16, down: bool, uppercase: bool);
    /// Press or release a mouse button and leave it that way. Used by the `down`
    /// and `up` click blocks, and to let go of a held button when a run ends.
    fn button(&mut self, button: MouseButton, down: bool);
    fn text(&mut self, text: &str);
    fn move_to(&mut self, x: i32, y: i32, duration_ms: u32);
    fn move_by(&mut self, dx: i32, dy: i32);
    fn click(&mut self, button: MouseButton, count: u32, hold_ms: u32);
    fn scroll(&mut self, amount: i32);
    /// Where the saved picture is on screen now, as the middle of the match.
    /// `None` means it is not there, which is a normal answer while waiting.
    fn find_image(&mut self, image: &str, similarity: u32) -> Option<(i32, i32)>;
    /// The program in front, as its executable name.
    fn foreground(&mut self) -> String;
    /// Wait, unless the run is stopped. Returns false when it is.
    fn wait(&mut self, ms: u32, stop: &AtomicBool) -> bool;
    /// Minutes since midnight on this machine's clock, for the time-of-day gate.
    fn local_minutes(&mut self) -> u32;
    /// Bring a matching window in front, waiting up to `timeout_ms` for one to
    /// exist. Failing means the run stops, with the reason in the report.
    fn focus(
        &mut self,
        target: &WindowTarget,
        timeout_ms: u32,
        stop: &AtomicBool,
    ) -> Result<(), String>;
    /// Start a program, and optionally wait until it has a window of its own.
    fn launch(
        &mut self,
        path: &str,
        args: &str,
        wait_for_window: bool,
        timeout_ms: u32,
        stop: &AtomicBool,
    ) -> Result<(), String>;
    /// Put text on the clipboard, replacing what was there.
    fn clipboard(&mut self, text: &str) -> Result<(), String>;
    /// Save an area of the screen as a PNG named for the run.
    fn screenshot(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        name: &str,
    ) -> Result<(), String>;
    /// Run another macro's blocks under this run.
    ///
    /// `chain` is what is already running, outermost first; the runner is what
    /// owns the rule about how deep that may go and what counts as a cycle.
    /// The notes come back so the caller's report can carry them.
    fn call_macro(
        &mut self,
        id: &str,
        chain: &[String],
        stop: &Arc<AtomicBool>,
    ) -> Result<Vec<String>, String>;
    /// Play a recording inline, under this run.
    fn replay(
        &mut self,
        id: &str,
        passes: u32,
        speed: f64,
        stop: &Arc<AtomicBool>,
    ) -> Result<(), String>;
    /// Flip one of the app's switches.
    fn set_tool(&mut self, tool: &str, enabled: bool) -> Result<(), String>;
}

/// Why a run ended.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub stop_reason: String,
    pub actions: u64,
    /// What the run's log blocks wrote, in order.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Walk the blocks. Stops early — cleanly, releasing what it pressed — when the
/// stop flag goes up or the action budget runs out.
pub fn play(steps: &[Step], runner: &mut dyn Runner, stop: Arc<AtomicBool>) -> Report {
    play_seeded(steps, runner, stop, clock_seed())
}

/// A run started by another run.
///
/// The chain of macro ids already running is carried in, so a macro that ends
/// up calling itself is refused at the block that closes the loop rather than
/// after eight levels of the same macro.
pub fn play_nested(
    steps: &[Step],
    runner: &mut dyn Runner,
    stop: Arc<AtomicBool>,
    chain: Vec<String>,
) -> Report {
    play_with(steps, runner, stop, chain, clock_seed())
}

/// The same run, with the random numbers pinned.
///
/// Tests use this so a random block is still a reproducible one; nothing else
/// has any business choosing the seed.
pub fn play_seeded(
    steps: &[Step],
    runner: &mut dyn Runner,
    stop: Arc<AtomicBool>,
    seed: u64,
) -> Report {
    play_with(steps, runner, stop, Vec::new(), seed)
}

/// The one body every entry point runs through.
fn play_with(
    steps: &[Step],
    runner: &mut dyn Runner,
    stop: Arc<AtomicBool>,
    chain: Vec<String>,
    seed: u64,
) -> Report {
    let mut held = Presses::default();
    let mut state = State {
        actions: 0,
        reason: String::from("Ran to the end"),
        rng: Rng::new(seed),
        notes: Vec::new(),
        chain,
    };

    match walk(steps, runner, &stop, &mut state, &mut held, 0) {
        Flow::Done => {}
        Flow::Stopped(reason) => state.reason = reason,
    }

    // Anything pressed with a `down` block and never released would stay down
    // after the macro finished, which is worse than not running it at all: a
    // key held down types on its own, and a mouse button held down keeps
    // clicking whatever the cursor happens to be over.
    held.release_all(runner);

    Report {
        stop_reason: state.reason,
        actions: state.actions,
        notes: state.notes,
    }
}

/// A seed nobody has to think about, different for every run.
fn clock_seed() -> u64 {
    use std::sync::atomic::AtomicU64;
    static RUNS: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0);
    // Two runs started inside the same nanosecond should still differ.
    nanos ^ RUNS
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

struct State {
    actions: u64,
    reason: String,
    rng: Rng,
    notes: Vec<String>,
    /// The macros already running above this one, outermost first.
    chain: Vec<String>,
}

/// What a run has pressed with a `down` block and not released yet.
#[derive(Default)]
struct Presses {
    keys: Vec<(u16, bool)>,
    buttons: Vec<MouseButton>,
}

impl Presses {
    fn press_key(&mut self, scan: u16, uppercase: bool) {
        self.keys.push((scan, uppercase));
    }

    /// Forget a key that a block released on purpose, so the run does not
    /// release it a second time.
    fn release_key(&mut self, scan: u16) {
        if let Some(index) = self.keys.iter().position(|(pressed, _)| *pressed == scan) {
            self.keys.remove(index);
        }
    }

    fn press_button(&mut self, button: MouseButton) {
        if !self.buttons.contains(&button) {
            self.buttons.push(button);
        }
    }

    fn release_button(&mut self, button: MouseButton) {
        self.buttons.retain(|pressed| *pressed != button);
    }

    /// Let go of everything still held, newest first.
    fn release_all(&mut self, runner: &mut dyn Runner) {
        for button in std::mem::take(&mut self.buttons).into_iter().rev() {
            runner.button(button, false);
        }
        for (scan, uppercase) in std::mem::take(&mut self.keys).into_iter().rev() {
            runner.key(scan, false, uppercase);
        }
    }
}

enum Flow {
    Done,
    Stopped(String),
}

/// How many passes a While may make before it stops loudly.
const MAX_ITERATIONS: u32 = 10_000;
/// The longest a While may run before it stops loudly: one hour.
const MAX_WHILE_MS: u32 = 3_600_000;

/// Whether a block's question is true right now.
///
/// One look, not a wait: a block that waits is what `Wait for picture` is for.
/// Both the If and the While ask through here, so they cannot drift apart.
fn holds(condition: &Condition, runner: &mut dyn Runner) -> bool {
    match condition {
        Condition::Picture {
            image,
            similarity,
        } => runner.find_image(image, *similarity).is_some(),
        Condition::Front { program } => runner
            .foreground()
            .trim()
            .eq_ignore_ascii_case(program.trim()),
    }
}

fn walk(
    steps: &[Step],
    runner: &mut dyn Runner,
    stop: &Arc<AtomicBool>,
    state: &mut State,
    held: &mut Presses,
    depth: usize,
) -> Flow {
    if depth > MAX_DEPTH {
        return Flow::Stopped(format!("nesting deeper than {MAX_DEPTH} levels"));
    }

    for step in steps {
        if stop.load(Ordering::Relaxed) {
            return Flow::Stopped("Stopped".to_string());
        }
        if state.actions >= ACTION_BUDGET {
            return Flow::Stopped(format!(
                "stopped after {} actions",
                state.actions
            ));
        }

        match step {
            Step::Key {
                scan,
                mode,
                hold_ms,
                uppercase,
            } => {
                state.actions += 1;
                // A key block can carry a mouse button instead of a key. The
                // modes mean the same thing — tap it, hold it, leave it down —
                // and a chord often wants a right click in among the keys.
                if let Some(button) = MouseButton::from_scan(*scan) {
                    match mode {
                        // A tap is the click block's click, hold time and all,
                        // so it is not a second implementation of the same
                        // gesture.
                        PressMode::Tap | PressMode::Drag => {
                            runner.click(button, 1, *hold_ms);
                        }
                        PressMode::Hold => {
                            runner.button(button, true);
                            held.press_button(button);
                            if !runner.wait(*hold_ms, stop) {
                                return Flow::Stopped("Stopped".to_string());
                            }
                            runner.button(button, false);
                            held.release_button(button);
                        }
                        PressMode::Down => {
                            runner.button(button, true);
                            held.press_button(button);
                        }
                        PressMode::Up => {
                            runner.button(button, false);
                            held.release_button(button);
                        }
                    }
                } else {
                    match mode {
                        PressMode::Tap | PressMode::Drag => runner.tap_key(*scan, *uppercase),
                        PressMode::Hold => {
                            runner.key(*scan, true, *uppercase);
                            if !runner.wait(*hold_ms, stop) {
                                runner.key(*scan, false, *uppercase);
                                return Flow::Stopped("Stopped".to_string());
                            }
                            runner.key(*scan, false, *uppercase);
                        }
                        PressMode::Down => {
                            runner.key(*scan, true, *uppercase);
                            held.press_key(*scan, *uppercase);
                        }
                        PressMode::Up => {
                            runner.key(*scan, false, *uppercase);
                            // Released on purpose, so it is not ours to clean up.
                            held.release_key(*scan);
                        }
                    }
                }
            }
            Step::Text { text, per_char_ms } => {
                if *per_char_ms == 0 {
                    state.actions += text.chars().count() as u64;
                    runner.text(text);
                } else {
                    // One character per block keeps a stopped run from dumping
                    // the rest of the string into whatever is focused.
                    for character in text.chars() {
                        state.actions += 1;
                        runner.text(&character.to_string());
                        if !runner.wait(*per_char_ms, stop) {
                            return Flow::Stopped("Stopped".to_string());
                        }
                    }
                }
            }
            Step::Move {
                x,
                y,
                relative,
                duration_ms,
            } => {
                state.actions += 1;
                if *relative {
                    runner.move_by(*x, *y);
                } else {
                    runner.move_to(*x, *y, *duration_ms);
                }
            }
            Step::Click {
                button,
                mode,
                count,
                hold_ms,
                x,
                y,
                relative,
                duration_ms,
            } => match mode {
                // A tap is the click the block is named after: one press and
                // release per count, each down for `hold_ms`.
                PressMode::Tap => {
                    state.actions += u64::from(*count);
                    runner.click(*button, *count, *hold_ms);
                }
                // One deliberate press, held for the time given and released.
                // Waited here rather than inside the runner so a stop is
                // noticed partway through a long hold, and so the button is
                // let go of even when the run is cut short.
                PressMode::Hold => {
                    state.actions += 1;
                    runner.button(*button, true);
                    held.press_button(*button);
                    if !runner.wait(*hold_ms, stop) {
                        return Flow::Stopped("Stopped".to_string());
                    }
                    runner.button(*button, false);
                    held.release_button(*button);
                }
                PressMode::Down => {
                    state.actions += 1;
                    runner.button(*button, true);
                    held.press_button(*button);
                }
                PressMode::Up => {
                    state.actions += 1;
                    runner.button(*button, false);
                    held.release_button(*button);
                }
                // The whole gesture in one block: press where the cursor is,
                // travel, rest there, let go. The press is registered as held
                // for the same reason a `Down` is — a run cut short in the
                // middle of a drag must not leave the button down.
                PressMode::Drag => {
                    state.actions += 2;
                    runner.button(*button, true);
                    held.press_button(*button);
                    if *relative {
                        runner.move_by(*x, *y);
                    } else {
                        runner.move_to(*x, *y, *duration_ms);
                    }
                    if !runner.wait(*hold_ms, stop) {
                        runner.button(*button, false);
                        held.release_button(*button);
                        return Flow::Stopped("Stopped".to_string());
                    }
                    runner.button(*button, false);
                    held.release_button(*button);
                }
            },
            Step::Scroll { amount } => {
                state.actions += 1;
                runner.scroll(*amount);
            }
            // Both picture blocks wait the same way: look, wait a moment, look
            // again, and give up saying how long it looked for. A run that
            // cannot find its picture stops rather than clicking the place the
            // picture used to be.
            Step::WaitImage {
                image,
                timeout_ms,
                similarity,
            } => {
                state.actions += 1;
                if let Err(reason) = wait_for_picture(runner, image, *similarity, *timeout_ms, stop)
                {
                    return Flow::Stopped(reason);
                }
            }
            Step::ClickImage {
                image,
                button,
                timeout_ms,
                similarity,
            } => {
                state.actions += 2;
                match wait_for_picture(runner, image, *similarity, *timeout_ms, stop) {
                    Ok((x, y)) => {
                        runner.move_to(x, y, 0);
                        runner.click(*button, 1, 0);
                    }
                    Err(reason) => return Flow::Stopped(reason),
                }
            }
            Step::Delay { ms } => {
                state.actions += 1;
                if !runner.wait(*ms, stop) {
                    return Flow::Stopped("Stopped".to_string());
                }
            }
            Step::If {
                condition,
                steps,
                otherwise,
            } => {
                state.actions += 1;
                let taken = holds(condition, runner);
                let branch = if taken { steps } else { otherwise };
                match walk(branch, runner, stop, state, held, depth + 1) {
                    Flow::Done => {}
                    Flow::Stopped(reason) => return Flow::Stopped(reason),
                }
            }
            Step::Repeat { times, steps } => {
                // Clamped rather than rejected: a run is not the place to
                // discover a bad number, and [`validate`] refuses them anyway.
                for _ in 0..(*times).clamp(1, MAX_TIMES) {
                    match walk(steps, runner, stop, state, held, depth + 1) {
                        Flow::Done => {}
                        Flow::Stopped(reason) => return Flow::Stopped(reason),
                    }
                }
            }
            // A note is for the person reading the macro, never for the run.
            Step::Note { .. } => {}
            Step::Log { text } => {
                if state.notes.len() < MAX_NOTES {
                    state
                        .notes
                        .push(text.chars().take(LOG_CHARS).collect::<String>());
                }
            }
            Step::RandomDelay { min_ms, max_ms } => {
                let ms = state.rng.between(*min_ms, *max_ms);
                if !runner.wait(ms, stop) {
                    return Flow::Stopped("Stopped".to_string());
                }
            }
            Step::Chord { scans, hold_ms } => {
                state.actions += scans.len() as u64;
                for scan in scans {
                    if *scan == 0 {
                        continue;
                    }
                    runner.key(*scan, true, false);
                    held.press_key(*scan, false);
                }
                if !runner.wait(*hold_ms, stop) {
                    // Let go before answering: a chord cut short must not leave
                    // three keys down for the rest of the session.
                    release_chord(held, runner, scans);
                    return Flow::Stopped("Stopped".to_string());
                }
                release_chord(held, runner, scans);
            }
            Step::ScrollAt { x, y, amount } => {
                state.actions += 2;
                runner.move_to(*x, *y, 0);
                runner.scroll(*amount);
            }
            Step::Until { at } => {
                if let Err(reason) = wait_until_clock(runner, at, stop) {
                    return Flow::Stopped(reason);
                }
            }
            Step::RepeatFor { seconds, steps } => {
                let until = Instant::now() + Duration::from_secs(u64::from(*seconds));
                while Instant::now() < until {
                    if steps.is_empty() {
                        // Validation refuses this, but a run must not spin for
                        // however long the block was set to.
                        if !runner.wait(100, stop) {
                            return Flow::Stopped("Stopped".to_string());
                        }
                        continue;
                    }
                    match walk(steps, runner, stop, state, held, depth + 1) {
                        Flow::Done => {}
                        Flow::Stopped(reason) => return Flow::Stopped(reason),
                    }
                }
            }
            Step::Stop { reason } => {
                return Flow::Stopped(match reason.trim() {
                    "" => "the macro stopped itself".to_string(),
                    why => why.to_string(),
                });
            }
            Step::Fail { message } => {
                return Flow::Stopped(match message.trim() {
                    "" => "the macro failed".to_string(),
                    why => format!("Failed: {why}"),
                });
            }
            Step::AssertImage { image, similarity } => {
                state.actions += 1;
                if runner.find_image(image, *similarity).is_none() {
                    return Flow::Stopped("the picture is not on screen".to_string());
                }
            }
            Step::Focus {
                target,
                timeout_ms,
            } => {
                state.actions += 1;
                if let Err(why) = runner.focus(target, *timeout_ms, stop) {
                    return Flow::Stopped(why);
                }
            }
            Step::Launch {
                path,
                args,
                wait_for_window,
                timeout_ms,
            } => {
                state.actions += 1;
                if let Err(why) = runner.launch(path, args, *wait_for_window, *timeout_ms, stop) {
                    return Flow::Stopped(why);
                }
            }
            Step::Clipboard { text } => {
                state.actions += 1;
                if let Err(why) = runner.clipboard(text) {
                    return Flow::Stopped(why);
                }
            }
            Step::Screenshot {
                x,
                y,
                width,
                height,
                name,
            } => {
                state.actions += 1;
                if let Err(why) = runner.screenshot(*x, *y, *width, *height, name) {
                    return Flow::Stopped(why);
                }
            }
            Step::CallMacro { id, .. } => {
                state.actions += 1;
                match runner.call_macro(id, &state.chain, stop) {
                    Ok(notes) => {
                        for note in notes {
                            if state.notes.len() < MAX_NOTES {
                                state.notes.push(note);
                            }
                        }
                    }
                    Err(why) => return Flow::Stopped(why),
                }
            }
            Step::Replay {
                id,
                passes,
                speed,
                ..
            } => {
                state.actions += 1;
                if let Err(why) = runner.replay(id, *passes, *speed, stop) {
                    return Flow::Stopped(why);
                }
            }
            Step::Tool { tool, enabled } => {
                state.actions += 1;
                if let Err(why) = runner.set_tool(tool, *enabled) {
                    return Flow::Stopped(why);
                }
            }
            Step::While {
                condition,
                steps,
                max_iterations,
                timeout_ms,
            } => {
                state.actions += 1;
                let limit = (*max_iterations).clamp(1, MAX_ITERATIONS);
                let deadline = std::time::Duration::from_millis(u64::from(*timeout_ms));
                let started = std::time::Instant::now();
                let mut passes = 0u32;
                while holds(condition, runner) {
                    if stop.load(Ordering::Relaxed) {
                        return Flow::Stopped("Stopped".to_string());
                    }
                    // Checked before the pass that would exceed them, so a
                    // runaway loop stops loudly rather than quietly going on.
                    if passes >= limit {
                        return Flow::Stopped(format!(
                            "while: stopped after {limit} passes — the check never went false"
                        ));
                    }
                    if started.elapsed() >= deadline {
                        return Flow::Stopped(format!(
                            "while: stopped after {} s — the check never went false",
                            deadline.as_secs_f32()
                        ));
                    }
                    match walk(steps, runner, stop, state, held, depth + 1) {
                        Flow::Done => {}
                        Flow::Stopped(reason) => return Flow::Stopped(reason),
                    }
                    passes += 1;
                }
            }
        }
    }

    Flow::Done
}

/// Let go of a chord in the reverse order it was pressed.
fn release_chord(held: &mut Presses, runner: &mut dyn Runner, scans: &[u16]) {
    for scan in scans.iter().rev() {
        if *scan == 0 {
            continue;
        }
        runner.key(*scan, false, false);
        held.release_key(*scan);
    }
}

/// Minutes since midnight, from a `HH:MM` written by a person.
fn parse_clock(at: &str) -> Result<u32, String> {
    let text = at.trim();
    if text.is_empty() {
        return Err("no time was given".to_string());
    }
    let (hours, minutes) = text
        .split_once(':')
        .ok_or_else(|| "a time needs hours and minutes, like 09:30".to_string())?;
    let hours: u32 = hours
        .trim()
        .parse()
        .map_err(|_| "the hours are not a number".to_string())?;
    let minutes: u32 = minutes
        .trim()
        .parse()
        .map_err(|_| "the minutes are not a number".to_string())?;
    if hours > 23 || minutes > 59 {
        return Err("a time has to be between 00:00 and 23:59".to_string());
    }
    Ok(hours * 60 + minutes)
}

/// Wait until the clock reaches `at`, or return at once if it already has.
///
/// "Not before", not "at": a macro started at ten past nine for a nine-o'clock
/// gate should run, not wait a day for tomorrow's nine. The catch that follows
/// from having no date is that asking for 00:05 at a quarter to midnight does
/// not wait — the block is a gate, and the day is not its business.
fn wait_until_clock(runner: &mut dyn Runner, at: &str, stop: &AtomicBool) -> Result<(), String> {
    let wanted = parse_clock(at)?;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err("Stopped".to_string());
        }
        if runner.local_minutes() >= wanted {
            return Ok(());
        }
        // A second at a time, with the clock read again each time: a machine
        // whose clock is corrected mid-wait still stops at the right moment.
        if !runner.wait(1_000, stop) {
            return Err("Stopped".to_string());
        }
    }
}

/// Shortest time a tap keeps the button down.
///
/// A synthetic down and an up in the same instant is a real click to Windows
/// and usually to a window, but not always to a game: one that samples its
/// input once a frame can see a press and a release that arrive together as
/// "not pressed". Measured here, a tap with no hold time put its down and its
/// up inside the same sampling tick, well under a millisecond apart. One frame
/// at 60 fps is the floor; a block's own time is used whenever it is longer.
pub const MIN_TAP_MS: u32 = 15;

/// How long a tap actually keeps the button down.
fn tap_hold(hold_ms: u32) -> u32 {
    hold_ms.max(MIN_TAP_MS)
}

/// The real runner: input through [`crate::input`], waits on the clock.
use crate::hook::SharedState;

/// How many macros may be running inside each other before the run is refused.
///
/// Nested macros are not recursion — there is no base case to reach — so the
/// cap is what stops a chain that would otherwise never end, and it is small
/// enough that the failure reads as a mistake rather than as a hang.
const MAX_CALL_DEPTH: usize = 8;

pub struct WindowsRunner {
    app: AppHandle,
    /// The list a called macro or a recording is looked up in, read at the
    /// moment it is needed rather than copied when the run started: a macro
    /// edited mid-run should be seen, not run from a stale copy.
    settings: SharedState,
}

impl WindowsRunner {
    pub fn new(app: AppHandle, settings: SharedState) -> Self {
        Self { app, settings }
    }
}

impl Runner for WindowsRunner {
    fn tap_key(&mut self, scan: u16, uppercase: bool) {
        if uppercase {
            crate::input::key(0x2A, true);
        }
        crate::input::key(scan, true);
        crate::input::key(scan, false);
        if uppercase {
            crate::input::key(0x2A, false);
        }
    }

    fn key(&mut self, scan: u16, down: bool, uppercase: bool) {
        if down && uppercase {
            crate::input::key(0x2A, true);
        }
        crate::input::key(scan, down);
        if !down && uppercase {
            crate::input::key(0x2A, false);
        }
    }

    fn text(&mut self, text: &str) {
        crate::input::text(text);
    }

    fn move_to(&mut self, x: i32, y: i32, duration_ms: u32) {
        if duration_ms == 0 {
            crate::input::move_to(x, y);
            return;
        }
        let (start_x, start_y) = crate::input::cursor_position();
        let steps = (duration_ms / 8).clamp(4, 60);
        let step_ms = f64::from(duration_ms) / f64::from(steps);
        for step in 1..=steps {
            let t = f64::from(step) / f64::from(steps);
            let eased = if t < 0.5 {
                2.0 * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
            };
            let next_x = f64::from(start_x) + f64::from(x - start_x) * eased;
            let next_y = f64::from(start_y) + f64::from(y - start_y) * eased;
            crate::input::move_to(next_x.round() as i32, next_y.round() as i32);
            std::thread::sleep(std::time::Duration::from_secs_f64(step_ms / 1_000.0));
        }
        crate::input::move_to(x, y);
    }

    fn move_by(&mut self, dx: i32, dy: i32) {
        crate::input::move_by(dx, dy);
    }

    fn button(&mut self, button: MouseButton, down: bool) {
        crate::input::button(button, down);
    }

    fn click(&mut self, button: MouseButton, count: u32, hold_ms: u32) {
        let down_for = tap_hold(hold_ms);
        for _ in 0..count {
            crate::input::button(button, true);
            std::thread::sleep(std::time::Duration::from_millis(u64::from(down_for)));
            crate::input::button(button, false);
        }
    }

    fn scroll(&mut self, amount: i32) {
        crate::input::scroll(amount);
    }

    fn find_image(&mut self, image: &str, similarity: u32) -> Option<(i32, i32)> {
        crate::pictures::locate(&self.app, image, similarity)
    }

    fn foreground(&mut self) -> String {
        crate::hook::foreground_program()
    }

    fn wait(&mut self, ms: u32, stop: &AtomicBool) -> bool {
        // Sliced so a ten-minute wait still notices the stop button.
        let mut left = ms;
        while left > 0 {
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            let slice = left.min(20);
            std::thread::sleep(std::time::Duration::from_millis(u64::from(slice)));
            left -= slice;
        }
        !stop.load(Ordering::Relaxed)
    }

    fn local_minutes(&mut self) -> u32 {
        let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        u32::from(now.wHour) * 60 + u32::from(now.wMinute)
    }

    fn focus(
        &mut self,
        target: &WindowTarget,
        timeout_ms: u32,
        stop: &AtomicBool,
    ) -> Result<(), String> {
        let window = crate::window::find(
            |window| target.matches(&window.title, &window.exe),
            timeout_ms,
            stop,
        )?;
        crate::window::activate(&window)
    }

    fn launch(
        &mut self,
        path: &str,
        args: &str,
        wait_for_window: bool,
        timeout_ms: u32,
        stop: &AtomicBool,
    ) -> Result<(), String> {
        let started = crate::spawn::launch(path, args)?;
        if !wait_for_window {
            return Ok(());
        }
        // Only this program's own window counts. A launcher that starts the real
        // game as a child would not be found by pid, which is why Focus is a
        // separate block rather than something this one guesses at.
        crate::window::find(|window| window.pid == started.pid, timeout_ms, stop)
            .map(|_| ())
            .map_err(|why| format!("“{path}” was started, but {why}"))
    }

    fn clipboard(&mut self, text: &str) -> Result<(), String> {
        crate::clipboard::set_text(text)
    }

    fn screenshot(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        name: &str,
    ) -> Result<(), String> {
        crate::screenshots::save(&self.app, x, y, width, height, name).map(|_| ())
    }

    fn call_macro(
        &mut self,
        id: &str,
        chain: &[String],
        stop: &Arc<AtomicBool>,
    ) -> Result<Vec<String>, String> {
        if chain.len() >= MAX_CALL_DEPTH {
            return Err(format!("macros are nested more than {MAX_CALL_DEPTH} deep"));
        }
        if chain.iter().any(|seen| seen == id) {
            return Err("that macro ends up calling itself".to_string());
        }
        let steps = {
            let shared = self.settings.lock().unwrap();
            let found = shared
                .settings
                .macros
                .iter()
                .find(|entry| entry.id == id)
                .ok_or_else(|| "that macro is no longer in the list".to_string())?;
            if !found.enabled {
                return Err(format!("“{}” is switched off", found.name));
            }
            found.steps.clone()
        };
        let mut next = chain.to_vec();
        next.push(id.to_string());
        let report = play_nested(&steps, self, Arc::clone(stop), next);
        if report.stop_reason != "Ran to the end" {
            return Err(report.stop_reason);
        }
        Ok(report.notes)
    }

    fn replay(
        &mut self,
        id: &str,
        passes: u32,
        speed: f64,
        stop: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        let (events, saved_speed) = {
            let shared = self.settings.lock().unwrap();
            let recording = shared
                .settings
                .replay
                .recordings
                .iter()
                .find(|entry| entry.id == id)
                .ok_or_else(|| "that recording is no longer in the list".to_string())?;
            (recording.events.clone(), shared.settings.replay.speed)
        };
        let speed = if speed > 0.0 { speed } else { saved_speed };
        crate::replay::play_inside(&events, Some(passes.max(1)), speed, stop)
    }

    fn set_tool(&mut self, tool: &str, enabled: bool) -> Result<(), String> {
        let runtime = self
            .app
            .state::<crate::runtime::RuntimeState>()
            .inner()
            .clone();
        // The run doing the switching is the one that is running, and it is not
        // stopped by its own block.
        let current = runtime.running();
        crate::runtime::apply_tool(
            &self.app,
            &self.settings,
            &runtime,
            tool,
            enabled,
            current.as_deref(),
        )
    }
}

#[cfg(test)]
mod group_a_tests {
    use super::*;

    /// A runner that writes down what it was asked to do, and can be told what
    /// the clock says, whether the picture is there, and whether a wait should
    /// come back early.
    #[derive(Default)]
    struct Rec {
        seen: Vec<String>,
        notes: Vec<String>,
        waits: Vec<u32>,
        minute: u32,
        found: bool,
        refuse_wait: bool,
    }

    impl Runner for Rec {
        fn tap_key(&mut self, scan: u16, _uppercase: bool) {
            self.seen.push(format!("tap {}", crate::keys::label(scan)));
        }
        fn key(&mut self, scan: u16, down: bool, _uppercase: bool) {
            self.seen.push(format!(
                "{} {}",
                if down { "down" } else { "up" },
                crate::keys::label(scan)
            ));
        }
        fn button(&mut self, _button: MouseButton, _down: bool) {}
        fn text(&mut self, _text: &str) {}
        fn move_to(&mut self, x: i32, y: i32, _duration_ms: u32) {
            self.seen.push(format!("to {x},{y}"));
        }
        fn move_by(&mut self, _dx: i32, _dy: i32) {}
        fn click(&mut self, _button: MouseButton, _count: u32, _hold_ms: u32) {}
        fn scroll(&mut self, amount: i32) {
            self.seen.push(format!("scroll {amount}"));
        }
        fn find_image(&mut self, _image: &str, _similarity: u32) -> Option<(i32, i32)> {
            self.found.then_some((1, 1))
        }
        fn foreground(&mut self) -> String {
            "explorer.exe".to_string()
        }
        fn wait(&mut self, ms: u32, stop: &AtomicBool) -> bool {
            self.waits.push(ms);
            !self.refuse_wait && !stop.load(Ordering::Relaxed)
        }
        fn local_minutes(&mut self) -> u32 {
            self.minute
        }
        fn focus(
            &mut self,
            _target: &WindowTarget,
            _timeout_ms: u32,
            _stop: &AtomicBool,
        ) -> Result<(), String> {
            self.seen.push("focus".to_string());
            Ok(())
        }
        fn launch(
            &mut self,
            _path: &str,
            _args: &str,
            _wait_for_window: bool,
            _timeout_ms: u32,
            _stop: &AtomicBool,
        ) -> Result<(), String> {
            self.seen.push("launch".to_string());
            Ok(())
        }
        fn clipboard(&mut self, _text: &str) -> Result<(), String> {
            self.seen.push("copy".to_string());
            Ok(())
        }
        fn screenshot(
            &mut self,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            _name: &str,
        ) -> Result<(), String> {
            self.seen.push(format!("shot {x},{y} {width}×{height}"));
            Ok(())
        }
        fn call_macro(
            &mut self,
            id: &str,
            chain: &[String],
            _stop: &Arc<AtomicBool>,
        ) -> Result<Vec<String>, String> {
            if chain.len() >= MAX_CALL_DEPTH {
                return Err(format!("macros are nested more than {MAX_CALL_DEPTH} deep"));
            }
            if chain.iter().any(|seen| seen == id) {
                return Err("that macro ends up calling itself".to_string());
            }
            self.seen.push(format!("call {id}"));
            self.notes.push(format!("from {id}"));
            Ok(self.notes.clone())
        }
        fn replay(
            &mut self,
            id: &str,
            passes: u32,
            speed: f64,
            _stop: &Arc<AtomicBool>,
        ) -> Result<(), String> {
            self.seen.push(format!("replay {id} ×{passes} at {speed}"));
            Ok(())
        }
        fn set_tool(&mut self, tool: &str, enabled: bool) -> Result<(), String> {
            self.seen.push(format!(
                "{tool} {}",
                if enabled { "on" } else { "off" }
            ));
            Ok(())
        }
    }

    fn run(steps: Vec<Step>, rec: &mut Rec) -> Report {
        play(&steps, rec, Arc::new(AtomicBool::new(false)))
    }

    fn problem(step: Step) -> String {
        let definition = Macro {
            id: "m".to_string(),
            name: "Test".to_string(),
            hotkey: String::new(),
            only_when: String::new(),
            enabled: true,
            steps: vec![step],
        };
        validate(&definition).unwrap_err()
    }

    #[test]
    fn a_note_runs_nothing_and_counts_nothing() {
        let mut rec = Rec::default();
        let report = run(
            vec![Step::Note {
                text: "the game must be in front".to_string(),
            }],
            &mut rec,
        );
        assert_eq!(report.stop_reason, "Ran to the end");
        assert_eq!(report.actions, 0);
        assert!(rec.seen.is_empty());
        assert!(report.notes.is_empty());
    }

    #[test]
    fn a_log_line_lands_in_the_report_in_order() {
        let mut rec = Rec::default();
        let report = run(
            vec![
                Step::Log {
                    text: "opened the menu".to_string(),
                },
                Step::Log {
                    text: "clicked the button".to_string(),
                },
            ],
            &mut rec,
        );
        assert_eq!(report.notes, vec!["opened the menu", "clicked the button"]);
        // A log line is not an action: it touches nothing on the desktop.
        assert_eq!(report.actions, 0);
    }

    #[test]
    fn a_random_wait_lands_between_its_bounds() {
        let mut rec = Rec::default();
        run(
            vec![Step::RandomDelay {
                min_ms: 100,
                max_ms: 200,
            }],
            &mut rec,
        );
        assert_eq!(rec.waits.len(), 1);
        assert!((100..=200).contains(&rec.waits[0]), "{:?}", rec.waits);
    }

    #[test]
    fn a_chord_presses_in_order_and_lets_go_backwards() {
        let mut rec = Rec::default();
        let report = run(
            vec![Step::Chord {
                scans: vec![0x1d, 0x2e],
                hold_ms: 0,
            }],
            &mut rec,
        );
        assert_eq!(report.stop_reason, "Ran to the end");
        let first = crate::keys::label(0x1d);
        let second = crate::keys::label(0x2e);
        assert_eq!(
            rec.seen,
            vec![
                format!("down {first}"),
                format!("down {second}"),
                format!("up {second}"),
                format!("up {first}"),
            ]
        );
    }

    /// A chord cut short mid-hold must still let go of every key it pressed.
    #[test]
    fn a_chord_lets_go_when_the_run_is_stopped() {
        let mut rec = Rec {
            refuse_wait: true,
            ..Rec::default()
        };
        let report = run(
            vec![Step::Chord {
                scans: vec![0x1d],
                hold_ms: 50,
            }],
            &mut rec,
        );
        assert_eq!(report.stop_reason, "Stopped");
        let key = crate::keys::label(0x1d);
        assert_eq!(
            rec.seen,
            vec![format!("down {key}"), format!("up {key}")]
        );
    }

    #[test]
    fn scrolling_at_a_point_moves_there_first() {
        let mut rec = Rec::default();
        let report = run(
            vec![Step::ScrollAt {
                x: 5,
                y: 6,
                amount: -3,
            }],
            &mut rec,
        );
        assert_eq!(report.stop_reason, "Ran to the end");
        assert_eq!(rec.seen, vec!["to 5,6", "scroll -3"]);
        assert_eq!(report.actions, 2);
    }

    #[test]
    fn a_time_that_has_already_passed_does_not_wait() {
        // Ten past ten, waiting for half past nine: a gate that is already open.
        let mut rec = Rec {
            minute: 10 * 60 + 10,
            ..Rec::default()
        };
        let report = run(
            vec![Step::Until {
                at: "09:30".to_string(),
            }],
            &mut rec,
        );
        assert_eq!(report.stop_reason, "Ran to the end");
        assert!(rec.waits.is_empty(), "{:?}", rec.waits);
    }

    #[test]
    fn a_time_still_to_come_follows_a_stop() {
        let mut rec = Rec::default(); // midnight
        let report = play(
            &[Step::Until {
                at: "09:30".to_string(),
            }],
            &mut rec,
            Arc::new(AtomicBool::new(true)),
        );
        assert_eq!(report.stop_reason, "Stopped");
    }

    #[test]
    fn stop_ends_the_run_saying_why() {
        let mut rec = Rec::default();
        let report = run(
            vec![
                Step::Note {
                    text: "about to finish".to_string(),
                },
                Step::Stop {
                    reason: "the run is done".to_string(),
                },
            ],
            &mut rec,
        );
        assert_eq!(report.stop_reason, "the run is done");
    }

    #[test]
    fn fail_ends_the_run_saying_what_went_wrong() {
        let mut rec = Rec::default();
        let report = run(
            vec![Step::Fail {
                message: "there was no health bar".to_string(),
            }],
            &mut rec,
        );
        assert_eq!(report.stop_reason, "Failed: there was no health bar");
    }

    #[test]
    fn an_assertion_that_is_not_met_stops_the_run() {
        let mut rec = Rec::default();
        let report = run(
            vec![Step::AssertImage {
                image: "pic-1".to_string(),
                similarity: 90,
            }],
            &mut rec,
        );
        assert_eq!(report.stop_reason, "the picture is not on screen");
    }

    #[test]
    fn an_assertion_that_is_met_carries_on() {
        let mut rec = Rec {
            found: true,
            ..Rec::default()
        };
        let report = run(
            vec![
                Step::AssertImage {
                    image: "pic-1".to_string(),
                    similarity: 90,
                },
                Step::Log {
                    text: "the menu is open".to_string(),
                },
            ],
            &mut rec,
        );
        assert_eq!(report.stop_reason, "Ran to the end");
        assert_eq!(report.notes, vec!["the menu is open"]);
    }

    #[test]
    fn an_empty_note_is_allowed_but_a_paragraph_is_not() {
        let definition = Macro {
            id: "m".to_string(),
            name: "Test".to_string(),
            hotkey: String::new(),
            only_when: String::new(),
            enabled: true,
            steps: vec![Step::Note {
                text: String::new(),
            }],
        };
        // An empty note is a note being written: refusing it would make the
        // block unusable until it is finished.
        assert!(validate(&definition).is_ok());
        let long = "x".repeat(NOTE_CHARS + 1);
        assert!(problem(Step::Note { text: long }).contains("longer than"));
    }

    #[test]
    fn a_log_with_nothing_in_it_is_refused() {
        assert!(problem(Step::Log {
            text: "   ".to_string()
        })
        .contains("says nothing"));
    }

    #[test]
    fn a_chord_is_limited_to_the_keys_a_hand_can_hold() {
        let too_many = Step::Chord {
            scans: vec![0x1d, 0x2a, 0x38, 0x11, 0x12],
            hold_ms: 50,
        };
        assert!(problem(too_many).contains("more than"));
        let empty = Step::Chord {
            scans: Vec::new(),
            hold_ms: 50,
        };
        assert!(problem(empty).contains("no keys"));
    }

    #[test]
    fn a_time_of_day_has_to_be_a_time_of_day() {
        assert!(problem(Step::Until {
            at: "half past nine".to_string()
        })
        .contains("09:30"));
        assert!(problem(Step::Until {
            at: "24:00".to_string()
        })
        .contains("09:30"));
    }

    #[test]
    fn a_failure_with_nothing_to_say_is_refused() {
        assert!(problem(Step::Fail {
            message: "  ".to_string()
        })
        .contains("what went wrong"));
    }

    #[test]
    fn a_repeat_for_needs_some_time_to_run_for() {
        assert!(problem(Step::RepeatFor {
            seconds: 0,
            steps: Vec::new()
        })
        .contains("no time at all"));
    }

    /// The guard against a block that works but cannot be saved: every new
    /// kind has to survive the trip through `settings.json` and come back
    /// identical, with the camelCase field names the window sends.
    #[test]
    fn every_block_survives_a_settings_round_trip() {
        let steps = vec![
            Step::Note {
                text: "open the menu first".to_string(),
            },
            Step::Log {
                text: "menu open".to_string(),
            },
            Step::RandomDelay {
                min_ms: 200,
                max_ms: 700,
            },
            Step::Chord {
                scans: vec![0x1d, 0x2e],
                hold_ms: 90,
            },
            Step::ScrollAt {
                x: 500,
                y: 300,
                amount: -4,
            },
            Step::Until {
                at: "09:30".to_string(),
            },
            Step::RepeatFor {
                seconds: 5,
                steps: vec![Step::Delay { ms: 100 }],
            },
            Step::Stop {
                reason: "done".to_string(),
            },
            Step::Fail {
                message: "no health bar".to_string(),
            },
            Step::AssertImage {
                image: "pic-1".to_string(),
                similarity: 85,
            },
            Step::Focus {
                target: WindowTarget::Title {
                    contains: "Notepad".to_string(),
                    exact: false,
                },
                timeout_ms: 2_500,
            },
            Step::Launch {
                path: "C:\\game\\game.exe".to_string(),
                args: "--windowed".to_string(),
                wait_for_window: true,
                timeout_ms: 9_000,
            },
            Step::Clipboard {
                text: "copied".to_string(),
            },
            Step::Screenshot {
                x: 10,
                y: 20,
                width: 640,
                height: 480,
                name: "after".to_string(),
            },
            Step::CallMacro {
                id: "macro-2".to_string(),
                name: "Open the menu".to_string(),
            },
            Step::Replay {
                id: "replay-1".to_string(),
                name: "Opening".to_string(),
                passes: 3,
                speed: 1.5,
            },
            Step::Tool {
                tool: "remapper".to_string(),
                enabled: true,
            },
            Step::While {
                condition: Condition::Front {
                    program: "notepad.exe".to_string(),
                },
                steps: vec![Step::Log {
                    text: "still there".to_string(),
                }],
                max_iterations: 42,
                timeout_ms: 30_000,
            },
        ];
        let json = serde_json::to_string(&steps).expect("serialise");
        // The window is camelCase throughout; a field renamed here is a number
        // that silently falls back to its default.
        assert!(json.contains("\"minMs\":200"), "{json}");
        assert!(json.contains("\"holdMs\":90"), "{json}");
        assert!(json.contains("\"similarity\":85"), "{json}");
        assert!(json.contains("\"timeoutMs\":2500"), "{json}");
        assert!(json.contains("\"waitForWindow\":true"), "{json}");
        assert!(json.contains("\"maxIterations\":42"), "{json}");
        assert!(json.contains("\"passes\":3"), "{json}");
        let back: Vec<Step> = serde_json::from_str(&json).expect("read back");
        assert_eq!(back, steps);
    }

    /// A random wait is only testable if the same seed makes the same choice,
    /// which is the whole reason `play_seeded` exists.
    #[test]
    fn the_same_seed_picks_the_same_random_wait() {
        let steps = vec![Step::RandomDelay {
            min_ms: 100,
            max_ms: 900,
        }];
        let mut first = Rec::default();
        let mut second = Rec::default();
        play_seeded(
            &steps,
            &mut first,
            Arc::new(AtomicBool::new(false)),
            0x5eed,
        );
        play_seeded(
            &steps,
            &mut second,
            Arc::new(AtomicBool::new(false)),
            0x5eed,
        );

        assert_eq!(first.waits, second.waits);
        assert!((100..=900).contains(&first.waits[0]), "{:?}", first.waits);
    }

    #[test]
    fn a_while_stops_loudly_when_its_check_never_goes_false() {
        let mut rec = Rec {
            found: true,
            ..Rec::default()
        };
        let report = run(
            vec![Step::While {
                condition: Condition::Picture {
                    image: "pic-1".to_string(),
                    similarity: 90,
                },
                steps: vec![Step::Delay { ms: 0 }],
                max_iterations: 3,
                timeout_ms: 60_000,
            }],
            &mut rec,
        );

        assert!(
            report.stop_reason.contains("3 passes"),
            "{}",
            report.stop_reason
        );
        assert_eq!(rec.waits.len(), 3);
    }

    #[test]
    fn a_while_runs_nothing_when_the_check_is_false() {
        let mut rec = Rec::default();
        let report = run(
            vec![Step::While {
                condition: Condition::Front {
                    program: "notpad.exe".to_string(),
                },
                steps: vec![Step::Delay { ms: 40 }],
                max_iterations: 5,
                timeout_ms: 60_000,
            }],
            &mut rec,
        );

        assert_eq!(report.stop_reason, "Ran to the end");
        assert!(rec.waits.is_empty());
    }

    #[test]
    fn a_while_needs_both_of_its_caps() {
        let no_cap = problem(Step::While {
            condition: Condition::Front {
                program: "notepad.exe".to_string(),
            },
            steps: Vec::new(),
            max_iterations: 0,
            timeout_ms: 60_000,
        });
        assert!(no_cap.contains("between 1 and"), "{no_cap}");
    }

    #[test]
    fn a_tool_block_flips_one_switch_and_nothing_else() {
        let mut rec = Rec::default();
        let report = run(
            vec![
                Step::Tool {
                    tool: "clicker".to_string(),
                    enabled: false,
                },
                Step::Tool {
                    tool: "remapper".to_string(),
                    enabled: true,
                },
            ],
            &mut rec,
        );

        assert_eq!(report.actions, 2);
        assert_eq!(
            rec.seen,
            vec!["clicker off".to_string(), "remapper on".to_string()]
        );
    }

    #[test]
    fn a_tool_block_has_to_name_a_tool_that_exists() {
        let wrong = problem(Step::Tool {
            tool: "paint".to_string(),
            enabled: true,
        });
        assert!(wrong.contains("does not exist"), "{wrong}");
    }

    #[test]
    fn a_called_macro_runs_inside_this_run() {
        let mut rec = Rec::default();
        let report = run(
            vec![
                Step::CallMacro {
                    id: "other".to_string(),
                    name: "Other".to_string(),
                },
                Step::Replay {
                    id: "rec-1".to_string(),
                    name: "Opening".to_string(),
                    passes: 2,
                    speed: 0.0,
                },
            ],
            &mut rec,
        );

        assert_eq!(report.actions, 2);
        assert_eq!(
            rec.seen,
            vec!["call other".to_string(), "replay rec-1 ×2 at 0".to_string()]
        );
        // A called macro's log lines come back into this run's report, or a
        // nested macro could never say where it got to.
        assert_eq!(report.notes, vec!["from other".to_string()]);
    }

    #[test]
    fn a_macro_that_calls_itself_is_refused_by_name() {
        let mut rec = Rec::default();
        let report = play_nested(
            &[Step::CallMacro {
                id: "a".to_string(),
                name: "A".to_string(),
            }],
            &mut rec,
            Arc::new(AtomicBool::new(false)),
            vec!["a".to_string()],
        );
        assert!(
            report.stop_reason.contains("calling itself"),
            "{}",
            report.stop_reason
        );
    }

    #[test]
    fn a_call_needs_a_macro_and_a_replay_needs_a_sane_speed() {
        let empty_call = problem(Step::CallMacro {
            id: String::new(),
            name: String::new(),
        });
        assert!(empty_call.contains("no macro chosen"), "{empty_call}");

        let bad_speed = problem(Step::Replay {
            id: "r".to_string(),
            name: String::new(),
            passes: 1,
            speed: 0.05,
        });
        assert!(bad_speed.contains("speed between"), "{bad_speed}");
    }

    #[test]
    fn focus_launch_clipboard_and_a_screenshot_reach_the_runner_in_order() {
        let mut rec = Rec::default();
        let report = run(
            vec![
                Step::Focus {
                    target: WindowTarget::Program {
                        exe: "game.exe".to_string(),
                    },
                    timeout_ms: 0,
                },
                Step::Launch {
                    path: "C:\\game\\game.exe".to_string(),
                    args: "--fast".to_string(),
                    wait_for_window: false,
                    timeout_ms: 0,
                },
                Step::Clipboard {
                    text: "hello".to_string(),
                },
                Step::Screenshot {
                    x: 1,
                    y: 2,
                    width: 30,
                    height: 40,
                    name: "after the fight".to_string(),
                },
            ],
            &mut rec,
        );

        assert_eq!(report.actions, 4);
        assert_eq!(
            rec.seen,
            vec![
                "focus".to_string(),
                "launch".to_string(),
                "copy".to_string(),
                "shot 1,2 30×40".to_string(),
            ]
        );
    }

    #[test]
    fn a_launch_needs_a_program_that_is_there() {
        let relative = problem(Step::Launch {
            path: "game.exe".to_string(),
            args: String::new(),
            wait_for_window: true,
            timeout_ms: 5_000,
        });
        assert!(relative.contains("full path"), "{relative}");

        let gone = problem(Step::Launch {
            path: "C:\\no\\such\\program.exe".to_string(),
            args: String::new(),
            wait_for_window: true,
            timeout_ms: 5_000,
        });
        assert!(gone.contains("not there any more"), "{gone}");
    }

    #[test]
    fn a_focus_needs_something_to_look_for() {
        let empty = problem(Step::Focus {
            target: WindowTarget::Title {
                contains: "  ".to_string(),
                exact: false,
            },
            timeout_ms: 5_000,
        });
        assert!(empty.contains("no window title"), "{empty}");
    }

    #[test]
    fn a_screenshot_needs_an_area() {
        let empty = problem(Step::Screenshot {
            x: 0,
            y: 0,
            width: 0,
            height: 400,
            name: String::new(),
        });
        assert!(empty.contains("no area"), "{empty}");
    }

    #[test]
    fn an_assertion_needs_a_picture_and_a_similarity() {
        assert!(problem(Step::AssertImage {
            image: String::new(),
            similarity: 90
        })
        .contains("pick one"));
        assert!(problem(Step::AssertImage {
            image: "pic-1".to_string(),
            similarity: 10
        })
        .contains("between 50% and 100%"));
    }
}

#[cfg(test)]
mod foundation_tests {
    use super::*;

    fn title(contains: &str, exact: bool) -> WindowTarget {
        WindowTarget::Title {
            contains: contains.to_string(),
            exact,
        }
    }

    #[test]
    fn a_title_target_matches_a_title_that_contains_it() {
        assert!(title("sales", false).matches("Weekly Sales Report", "excel.exe"));
        assert!(!title("sales", false).matches("Weekly report", "excel.exe"));
    }

    #[test]
    fn an_exact_title_has_to_be_the_whole_title() {
        let target = title("  Weekly Sales Report ", true);
        assert!(target.matches("weekly sales report", "excel.exe"));
        assert!(!target.matches("Weekly Sales Report - 2026", "excel.exe"));
    }

    #[test]
    fn a_target_that_says_nothing_matches_nothing() {
        // Otherwise an empty field would focus the first window on the desktop,
        // which is the worst possible answer to "which window".
        assert!(!title("   ", false).matches("Anything", "x.exe"));
        assert!(!WindowTarget::Program {
            exe: "  ".to_string()
        }
        .matches("Anything", "x.exe"));
    }

    #[test]
    fn a_program_target_matches_the_executable_name() {
        let target = WindowTarget::Program {
            exe: "Overwatch.exe".to_string(),
        };
        assert!(target.matches("Overwatch", "overwatch.exe"));
        assert!(!target.matches("Overwatch", "battle.net.exe"));
    }

    #[test]
    fn random_numbers_stay_inside_their_bounds() {
        let mut rng = Rng::new(7);
        let (mut saw_low, mut saw_high) = (false, false);
        for _ in 0..1_000 {
            let number = rng.between(10, 20);
            assert!((10..=20).contains(&number), "{number}");
            saw_low |= number == 10;
            saw_high |= number == 20;
        }
        // Both ends have to be reachable: a range that never returns its own
        // maximum is a range whose upper box does nothing.
        assert!(saw_low && saw_high);
    }

    #[test]
    fn a_range_written_backwards_still_works() {
        let mut rng = Rng::new(1);
        assert!((3..=9).contains(&rng.between(9, 3)));
    }

    fn answers(seed: u64) -> Vec<u32> {
        let mut rng = Rng::new(seed);
        (0..8).map(|_| rng.between(0, 1_000)).collect()
    }

    #[test]
    fn the_same_seed_answers_the_same_way() {
        assert_eq!(answers(42), answers(42));
        assert_ne!(answers(42), answers(43));
    }

    #[test]
    fn a_zero_seed_still_produces_numbers() {
        // Zero is a fixed point of xorshift and would answer zero for ever.
        let mut rng = Rng::new(0);
        assert_ne!(rng.next(), 0);
    }
}

#[cfg(test)]
mod picture_tests {
    use super::*;

    /// A runner that finds whatever the test told it to find, and writes down
    /// what it was asked to do.
    #[derive(Default)]
    struct Fake {
        seen: Vec<String>,
        found: Option<(i32, i32)>,
        front: String,
    }

    impl Runner for Fake {
        fn tap_key(&mut self, _scan: u16, _uppercase: bool) {}
        fn key(&mut self, _scan: u16, _down: bool, _uppercase: bool) {}
        fn button(&mut self, _button: MouseButton, _down: bool) {}
        fn text(&mut self, _text: &str) {}
        fn move_to(&mut self, x: i32, y: i32, _duration_ms: u32) {
            self.seen.push(format!("to {x},{y}"));
        }
        fn move_by(&mut self, _dx: i32, _dy: i32) {}
        fn click(&mut self, button: MouseButton, count: u32, _hold_ms: u32) {
            self.seen
                .push(format!("click {} ×{count}", button.label().to_lowercase()));
        }
        fn scroll(&mut self, _amount: i32) {}
        fn find_image(&mut self, image: &str, _similarity: u32) -> Option<(i32, i32)> {
            self.seen.push(format!("look for {image}"));
            self.found
        }
        fn foreground(&mut self) -> String {
            self.front.clone()
        }
        fn wait(&mut self, _ms: u32, stop: &AtomicBool) -> bool {
            !stop.load(Ordering::Relaxed)
        }
        fn focus(
            &mut self,
            _target: &WindowTarget,
            _timeout_ms: u32,
            _stop: &AtomicBool,
        ) -> Result<(), String> {
            Ok(())
        }
        fn launch(
            &mut self,
            _path: &str,
            _args: &str,
            _wait_for_window: bool,
            _timeout_ms: u32,
            _stop: &AtomicBool,
        ) -> Result<(), String> {
            Ok(())
        }
        fn clipboard(&mut self, _text: &str) -> Result<(), String> {
            Ok(())
        }
        fn screenshot(
            &mut self,
            _x: i32,
            _y: i32,
            _width: i32,
            _height: i32,
            _name: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        fn call_macro(
            &mut self,
            _id: &str,
            _chain: &[String],
            _stop: &Arc<AtomicBool>,
        ) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }
        fn replay(
            &mut self,
            _id: &str,
            _passes: u32,
            _speed: f64,
            _stop: &Arc<AtomicBool>,
        ) -> Result<(), String> {
            Ok(())
        }
        fn set_tool(&mut self, _tool: &str, _enabled: bool) -> Result<(), String> {
            Ok(())
        }
        fn local_minutes(&mut self) -> u32 {
            0
        }
    }

    fn wait_for(timeout_ms: u32) -> Step {
        Step::WaitImage {
            image: "pic-1".to_string(),
            timeout_ms,
            similarity: 90,
        }
    }

    #[test]
    fn a_wait_block_goes_on_once_the_picture_is_there() {
        let mut fake = Fake {
            found: Some((10, 10)),
            ..Fake::default()
        };
        let report = play(&[wait_for(0)], &mut fake, Arc::new(AtomicBool::new(false)));
        assert_eq!(report.stop_reason, "Ran to the end");
        assert_eq!(fake.seen, vec!["look for pic-1"]);
    }

    /// A picture that never appears stops the run and says why, rather than
    /// carrying on to click somewhere the picture used to be.
    #[test]
    fn a_picture_that_is_not_there_stops_the_run() {
        let mut fake = Fake::default();
        let report = play(&[wait_for(0)], &mut fake, Arc::new(AtomicBool::new(false)));
        assert!(
            report.stop_reason.contains("not on screen"),
            "{}",
            report.stop_reason
        );
    }

    #[test]
    fn a_click_picture_block_clicks_what_it_found() {
        let mut fake = Fake {
            found: Some((300, 200)),
            ..Fake::default()
        };
        let step = Step::ClickImage {
            image: "pic-1".to_string(),
            button: MouseButton::Left,
            timeout_ms: 0,
            similarity: 90,
        };
        let report = play(&[step], &mut fake, Arc::new(AtomicBool::new(false)));
        assert_eq!(report.stop_reason, "Ran to the end");
        assert_eq!(fake.seen, vec!["look for pic-1", "to 300,200", "click left ×1"]);
    }

    fn tap_w() -> Step {
        Step::Key {
            scan: 0x11,
            mode: PressMode::Tap,
            hold_ms: 0,
            uppercase: false,
        }
    }

    fn branch(picture: bool, then_steps: Vec<Step>, else_steps: Vec<Step>) -> Step {
        Step::If {
            condition: Condition::Picture {
                image: "pic-1".to_string(),
                similarity: 90,
            },
            steps: if picture { then_steps } else { Vec::new() },
            otherwise: if picture { Vec::new() } else { else_steps },
        }
    }

    /// The picture is there, so the blocks under it run and the ones under
    /// Otherwise do not.
    #[test]
    fn an_if_runs_its_blocks_when_the_picture_is_there() {
        let mut fake = Fake {
            found: Some((1, 1)),
            ..Fake::default()
        };
        let step = branch(true, vec![tap_w()], vec![Step::Delay { ms: 5 }]);
        let report = play(&[step], &mut fake, Arc::new(AtomicBool::new(false)));
        assert_eq!(report.stop_reason, "Ran to the end");
        assert_eq!(fake.seen, vec!["look for pic-1"]);
    }

    /// The picture is not there, so the other set runs instead — and the run
    /// reaches the end rather than stopping, because a missing picture is the
    /// question being asked, not a failure.
    #[test]
    fn an_if_runs_the_otherwise_blocks_when_it_is_not() {
        let mut fake = Fake::default();
        let step = branch(false, vec![tap_w()], vec![Step::Delay { ms: 0 }]);
        let report = play(&[step], &mut fake, Arc::new(AtomicBool::new(false)));
        assert_eq!(report.stop_reason, "Ran to the end");
        assert_eq!(fake.seen, vec!["look for pic-1"]);
    }

    /// With nothing to do either way, an if is a look and nothing else.
    #[test]
    fn an_if_with_no_blocks_still_looks_once_and_carries_on() {
        let mut fake = Fake::default();
        let report = play(
            &[branch(false, Vec::new(), Vec::new())],
            &mut fake,
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(report.stop_reason, "Ran to the end");
        assert_eq!(fake.seen, vec!["look for pic-1"]);
    }

    #[test]
    fn an_if_on_the_program_in_front_compares_names_without_minding_case() {
        let mut fake = Fake {
            front: "Notepad.EXE".to_string(),
            ..Fake::default()
        };
        let step = Step::If {
            condition: Condition::Front {
                program: "notepad.exe".to_string(),
            },
            steps: vec![tap_w()],
            otherwise: Vec::new(),
        };
        let report = play(&[step], &mut fake, Arc::new(AtomicBool::new(false)));
        assert_eq!(report.stop_reason, "Ran to the end");
    }

    #[test]
    fn an_if_with_nothing_to_check_is_refused() {
        let macro_definition = Macro {
            id: "m".to_string(),
            name: "If test".to_string(),
            hotkey: String::new(),
            only_when: String::new(),
            enabled: true,
            steps: vec![Step::If {
                condition: Condition::Front {
                    program: "  ".to_string(),
                },
                steps: Vec::new(),
                otherwise: Vec::new(),
            }],
        };
        let problem = validate(&macro_definition).unwrap_err();
        assert!(problem.contains("no program"), "{problem}");
    }

    #[test]
    fn a_picture_block_with_no_picture_is_refused_before_it_runs() {
        let macro_definition = Macro {
            id: "m".to_string(),
            name: "Picture test".to_string(),
            hotkey: String::new(),
            only_when: String::new(),
            enabled: true,
            steps: vec![Step::WaitImage {
                image: String::new(),
                timeout_ms: 0,
                similarity: 90,
            }],
        };
        let problem = validate(&macro_definition).unwrap_err();
        assert!(problem.contains("no picture"), "{problem}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Records every action, and runs on a virtual clock so a `wait` costs
    /// nothing but a number.
    #[derive(Default)]
    struct Recorder {
        actions: Rc<RefCell<Vec<String>>>,
        clock: Rc<RefCell<u64>>,
        /// What a picture block should find, if anything.
        found: Option<(i32, i32)>,
    }

    impl Recorder {
        fn note(&self, line: String) {
            self.actions.borrow_mut().push(line);
        }

        fn lines(&self) -> Vec<String> {
            self.actions.borrow().clone()
        }
    }

    impl Runner for Recorder {
        fn tap_key(&mut self, scan: u16, uppercase: bool) {
            self.note(format!(
                "tap {}{}",
                if uppercase { "Shift+" } else { "" },
                crate::keys::label(scan)
            ));
        }

        fn key(&mut self, scan: u16, down: bool, uppercase: bool) {
            self.note(format!(
                "{} {}{}",
                if down { "down" } else { "up" },
                if uppercase { "Shift+" } else { "" },
                crate::keys::label(scan)
            ));
        }

        fn text(&mut self, text: &str) {
            self.note(format!("type {text:?}"));
        }

        fn move_to(&mut self, x: i32, y: i32, duration_ms: u32) {
            self.note(format!("to {x},{y} in {duration_ms} ms"));
        }

        fn move_by(&mut self, dx: i32, dy: i32) {
            self.note(format!("by {dx},{dy}"));
        }

        fn button(&mut self, button: MouseButton, down: bool) {
            self.note(format!(
                "{} {}",
                if down { "down" } else { "up" },
                button.label()
            ));
        }

        fn click(&mut self, button: MouseButton, count: u32, hold_ms: u32) {
            self.note(format!("{} ×{count} hold {hold_ms}", button.label()));
        }

        fn scroll(&mut self, amount: i32) {
            self.note(format!("scroll {amount}"));
        }

        fn find_image(&mut self, image: &str, _similarity: u32) -> Option<(i32, i32)> {
            self.note(format!("look for {image}"));
            self.found
        }

        fn foreground(&mut self) -> String {
            "explorer.exe".to_string()
        }

        /// Waits in real slices, the way the Windows runner does, so a stop in
        /// the middle of a long wait is noticed instead of optimistically
        /// skipped. The virtual clock still advances by the whole duration.
        fn wait(&mut self, ms: u32, stop: &AtomicBool) -> bool {
            let mut left = ms;
            while left > 0 {
                if stop.load(Ordering::Relaxed) {
                    return false;
                }
                let slice = left.min(20);
                std::thread::sleep(std::time::Duration::from_millis(u64::from(slice)));
                *self.clock.borrow_mut() += u64::from(slice);
                left -= slice;
            }
            !stop.load(Ordering::Relaxed)
        }

        fn focus(
            &mut self,
            target: &WindowTarget,
            _timeout_ms: u32,
            _stop: &AtomicBool,
        ) -> Result<(), String> {
            // The engine hands over a target; describing it in the transcript
            // is what makes a focus test readable afterwards.
            self.note(format!("focus {:?}", target));
            Ok(())
        }

        fn launch(
            &mut self,
            path: &str,
            args: &str,
            wait_for_window: bool,
            _timeout_ms: u32,
            _stop: &AtomicBool,
        ) -> Result<(), String> {
            self.note(format!(
                "launch {path} {args:?}{}",
                if wait_for_window { " and wait" } else { "" }
            ));
            Ok(())
        }

        fn clipboard(&mut self, text: &str) -> Result<(), String> {
            self.note(format!("copy {text:?}"));
            Ok(())
        }

        fn screenshot(
            &mut self,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            name: &str,
        ) -> Result<(), String> {
            self.note(format!("shot {x},{y} {width}×{height} as {name:?}"));
            Ok(())
        }

        fn call_macro(
            &mut self,
            id: &str,
            _chain: &[String],
            _stop: &Arc<AtomicBool>,
        ) -> Result<Vec<String>, String> {
            self.note(format!("call {id}"));
            Ok(Vec::new())
        }

        fn replay(
            &mut self,
            id: &str,
            passes: u32,
            speed: f64,
            _stop: &Arc<AtomicBool>,
        ) -> Result<(), String> {
            self.note(format!("replay {id} ×{passes} at {speed}"));
            Ok(())
        }

        fn set_tool(&mut self, tool: &str, enabled: bool) -> Result<(), String> {
            self.note(format!(
                "{tool} {}",
                if enabled { "on" } else { "off" }
            ));
            Ok(())
        }

        /// The virtual clock, read as a time of day: the recorder starts at
        /// midnight, so a test can move time by moving the clock.
        fn local_minutes(&mut self) -> u32 {
            ((*self.clock.borrow() / 60_000) % 1_440) as u32
        }
    }

    /// The block every other test uses: press W and let go.
    fn tap_w() -> Step {
        Step::Key {
            scan: 0x11,
            mode: PressMode::Tap,
            hold_ms: 0,
            uppercase: false,
        }
    }

    /// A macro with nothing in it yet.
    fn sample(id: &str, name: &str) -> Macro {
        Macro {
            id: id.to_string(),
            name: name.to_string(),
            hotkey: String::new(),
            only_when: String::new(),
            enabled: true,
            steps: Vec::new(),
        }
    }

    /// Run one macro and hand back what it did.
    fn run(steps: &[Step]) -> Vec<String> {
        let mut recorder = Recorder::default();
        let actions = Rc::clone(&recorder.actions);
        let stop = Arc::new(AtomicBool::new(false));
        let report = play(steps, &mut recorder, stop);
        assert_eq!(report.stop_reason, "Ran to the end");
        let lines = actions.borrow().clone();
        lines
    }

    #[test]
    fn a_tap_presses_and_a_hold_releases_after_its_time() {
        let lines = run(&[
            Step::Key {
                scan: 0x11,
                mode: PressMode::Tap,
                hold_ms: 60,
                uppercase: false,
            },
            Step::Key {
                scan: 0x1E,
                mode: PressMode::Hold,
                hold_ms: 250,
                uppercase: false,
            },
            Step::Key {
                scan: 0x11,
                mode: PressMode::Tap,
                hold_ms: 60,
                uppercase: true,
            },
        ]);

        assert_eq!(lines, vec!["tap W", "down A", "up A", "tap Shift+W"]);
    }

    #[test]
    fn a_key_block_can_hold_a_mouse_button() {
        // The same block, pointed at the right button: tap, then leave it down,
        // then let go — the chord a game menu wants, in three blocks. 0x202 is
        // the right button, above every keyboard scan.
        let lines = run(&[
            Step::Key {
                scan: 0x202,
                mode: PressMode::Tap,
                hold_ms: 40,
                uppercase: false,
            },
            Step::Key {
                scan: 0x202,
                mode: PressMode::Down,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Delay { ms: 50 },
            Step::Key {
                scan: 0x202,
                mode: PressMode::Up,
                hold_ms: 0,
                uppercase: false,
            },
        ]);

        assert_eq!(lines, vec!["Right ×1 hold 40", "down Right", "up Right"]);
        assert_eq!(crate::keys::label(0x202), "Right click");
    }

    #[test]
    fn a_down_block_spans_later_blocks_until_it_is_released() {
        let lines = run(&[
            Step::Key {
                scan: 0x2A,
                mode: PressMode::Down,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Delay { ms: 50 },
            Step::Click {
                button: MouseButton::Left,
                mode: PressMode::Tap,
                count: 1,
                hold_ms: 0,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            },
            Step::Key {
                scan: 0x2A,
                mode: PressMode::Up,
                hold_ms: 0,
                uppercase: false,
            },
        ]);

        assert_eq!(
            lines,
            vec!["down Left Shift", "Left ×1 hold 0", "up Left Shift"]
        );
    }

    /// A held click is one press, whatever the count says: the count belongs to
    /// a tap, and this is the block for "keep the button down for a moment".
    #[test]
    fn a_held_click_presses_once_waits_and_lets_go() {
        let recorder = Recorder::default();
        let actions = Rc::clone(&recorder.actions);
        let clock = Rc::clone(&recorder.clock);
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));
        let report = play(
            &[Step::Click {
                button: MouseButton::Right,
                mode: PressMode::Hold,
                count: 7,
                hold_ms: 300,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            }],
            &mut runner,
            stop,
        );

        assert_eq!(actions.borrow().clone(), vec!["down Right", "up Right"]);
        assert_eq!(*clock.borrow(), 300, "the hold should take its time");
        assert_eq!(report.actions, 1);
    }

    /// Down and up span blocks, which is what they are for: press, do something
    /// else with the button still down, then release.
    #[test]
    fn a_click_held_down_spans_blocks_until_its_own_up_block() {
        let lines = run(&[
            Step::Click {
                button: MouseButton::Left,
                mode: PressMode::Down,
                count: 1,
                hold_ms: 0,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            },
            Step::Delay { ms: 20 },
            Step::Click {
                button: MouseButton::Left,
                mode: PressMode::Up,
                count: 1,
                hold_ms: 0,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            },
        ]);

        // Nothing extra on the way out: the `up` block released it already.
        assert_eq!(lines, vec!["down Left", "up Left"]);
    }

    /// A stuck mouse button is worse than a stuck key, so the same net covers
    /// it: a run that is cut short must not leave one held down.
    #[test]
    fn a_stopped_run_lets_go_of_a_held_mouse_button() {
        let recorder = Recorder::default();
        let actions = Rc::clone(&recorder.actions);
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));
        {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(30));
                stop.store(true, Ordering::Relaxed);
            });
        }
        let report = play(
            &[
                Step::Click {
                    button: MouseButton::Left,
                    mode: PressMode::Down,
                    count: 1,
                    hold_ms: 0,
                    x: 0,
                    y: 0,
                    relative: true,
                    duration_ms: 0,
                },
                Step::Delay { ms: 10_000 },
            ],
            &mut runner,
            stop,
        );

        assert_eq!(report.stop_reason, "Stopped");
        assert_eq!(actions.borrow().clone(), vec!["down Left", "up Left"]);
    }

    #[test]
    fn a_press_or_release_needs_no_count_but_a_tap_does() {
        let click = |mode| Step::Click {
            button: MouseButton::Left,
            mode,
            count: 0,
            hold_ms: 0,
            x: 0,
            y: 0,
            relative: true,
            duration_ms: 0,
        };

        for mode in [PressMode::Down, PressMode::Up, PressMode::Drag] {
            let mut macro_definition = sample("m", "Test");
            macro_definition.steps = vec![click(mode)];
            assert!(
                validate(&macro_definition).is_ok(),
                "a {mode:?} is one action, so a zero count is not a mistake"
            );
        }

        let mut macro_definition = sample("m", "Test");
        macro_definition.steps = vec![click(PressMode::Tap)];
        assert!(
            validate(&macro_definition).is_err(),
            "a tap of nothing clicks nothing"
        );
    }

    #[test]
    fn every_click_mode_reads_as_what_it_does() {
        let describe = |mode, count, hold_ms| {
            Step::Click {
                button: MouseButton::Left,
                mode,
                count,
                hold_ms,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            }
            .summary()
        };

        assert_eq!(describe(PressMode::Tap, 3, 0), "left ×3");
        assert_eq!(describe(PressMode::Tap, 3, 40), "left ×3, held 40 ms");
        assert_eq!(describe(PressMode::Hold, 1, 250), "left held 250 ms");
        assert_eq!(describe(PressMode::Down, 1, 0), "left down");
        assert_eq!(describe(PressMode::Up, 1, 0), "left up");

        let drag = |relative, hold_ms| {
            Step::Click {
                button: MouseButton::Right,
                mode: PressMode::Drag,
                count: 1,
                hold_ms,
                x: 900,
                y: 500,
                relative,
                duration_ms: 250,
            }
            .summary()
        };

        assert_eq!(drag(true, 0), "drag right by 900,500");
        assert_eq!(drag(true, 40), "drag right by 900,500, resting 40 ms");
        assert_eq!(
            drag(false, 0),
            "drag right to 900,500 over 250 ms",
            "a drag to a point says how long it takes to get there"
        );
    }

    /// The safety net: a stop in the middle of a chord must not leave Shift
    /// held down for the rest of the session.
    #[test]
    fn a_stopped_run_releases_the_keys_it_pressed() {
        let recorder = Recorder::default();
        let actions = Rc::clone(&recorder.actions);
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));

        let steps = vec![
            Step::Key {
                scan: 0x11,
                mode: PressMode::Down,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Key {
                scan: 0x39,
                mode: PressMode::Down,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Delay { ms: 10_000 },
        ];

        // Stop it while it is waiting.
        {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(30));
                stop.store(true, Ordering::Relaxed);
            });
        }
        let report = play(&steps, &mut runner, stop);

        assert_eq!(report.stop_reason, "Stopped");
        assert_eq!(
            actions.borrow().clone(),
            vec!["down W", "down Spacebar", "up Spacebar", "up W"],
            "held keys should be released in reverse order"
        );
    }

    #[test]
    fn text_is_typed_one_character_at_a_time_when_a_gap_is_set() {
        let lines = run(&[Step::Text {
            text: "hi".to_string(),
            per_char_ms: 30,
        }]);
        assert_eq!(lines, vec!["type \"h\"", "type \"i\""]);

        let at_once = run(&[Step::Text {
            text: "hello".to_string(),
            per_char_ms: 0,
        }]);
        assert_eq!(at_once, vec!["type \"hello\""]);
    }

    #[test]
    fn nested_repeats_multiply_out() {
        let recorder = Recorder::default();
        let clock = Rc::clone(&recorder.clock);
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));

        let inner = Step::Repeat {
            times: 2,
            steps: vec![tap_w(), Step::Delay { ms: 5 }],
        };
        let report = play(
            &[Step::Repeat {
                times: 3,
                steps: vec![inner],
            }],
            &mut runner,
            stop,
        );

        // Three outer passes of two inner passes, each a tap and a wait.
        assert_eq!(report.actions, 3 * 2 * 2);
        assert_eq!(*clock.borrow(), 3 * 2 * 5, "the waits should add up");
    }

    #[test]
    fn a_repeat_of_one_still_runs_once() {
        // A zero count is refused when editing, but the interpreter must not
        // turn it into a skip either.
        let once = run(&[Step::Repeat {
            times: 0,
            steps: vec![tap_w()],
        }]);
        let twice = run(&[Step::Repeat {
            times: 2,
            steps: vec![tap_w()],
        }]);
        assert_eq!(once, vec!["tap W"]);
        assert_eq!(twice, vec!["tap W", "tap W"]);
    }

    #[test]
    fn the_report_counts_what_the_run_did() {
        let recorder = Recorder::default();
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));
        let report = play(
            &[
                Step::Click {
                    button: MouseButton::Right,
                    mode: PressMode::Tap,
                    count: 3,
                    hold_ms: 1,
                    x: 0,
                    y: 0,
                    relative: true,
                    duration_ms: 0,
                },
                Step::Text {
                    text: "abc".to_string(),
                    per_char_ms: 1,
                },
                Step::Scroll { amount: -2 },
            ],
            &mut runner,
            stop,
        );

        // Three clicks, three characters, one scroll.
        assert_eq!(report.actions, 7);
        assert_eq!(report.stop_reason, "Ran to the end");
    }

    #[test]
    fn every_kind_of_block_is_described_and_written_out() {
        let steps = vec![
            Step::Key {
                scan: 0x11,
                mode: PressMode::Hold,
                hold_ms: 120,
                uppercase: true,
            },
            Step::Move {
                x: 40,
                y: 60,
                relative: true,
                duration_ms: 0,
            },
            Step::Move {
                x: 400,
                y: 600,
                relative: false,
                duration_ms: 200,
            },
            Step::Scroll { amount: 3 },
            Step::Delay { ms: 250 },
            Step::Click {
                button: MouseButton::Left,
                mode: PressMode::Drag,
                count: 1,
                hold_ms: 0,
                x: 900,
                y: 500,
                relative: false,
                duration_ms: 250,
            },
        ];
        let lines = run(&steps);

        assert_eq!(lines[0], "down Shift+W");
        assert_eq!(lines[1], "up Shift+W");
        assert_eq!(lines[2], "by 40,60");
        assert_eq!(lines[3], "to 400,600 in 200 ms");
        assert_eq!(lines[4], "scroll 3");
        assert_eq!(lines[5], "down Left");
        assert_eq!(lines[6], "to 900,500 in 250 ms");
        assert_eq!(lines[7], "up Left");

        assert_eq!(steps[0].summary(), "Shift+W held 120 ms");
        assert_eq!(steps[1].summary(), "move by 40,60");
        assert_eq!(steps[2].summary(), "glide to 400,600 over 200 ms");
        assert_eq!(steps[3].summary(), "scroll 3");
        assert_eq!(steps[4].summary(), "wait 250 ms");
        assert_eq!(steps[5].summary(), "drag left to 900,500 over 250 ms");
    }

    #[test]
    fn a_macro_only_runs_where_it_says_it_does() {
        let mut macro_definition = sample("m1", "Burst fire");
        assert!(should_run(&macro_definition, "Anything.exe"));

        macro_definition.only_when = "Game.exe".into();
        assert!(should_run(&macro_definition, "game.exe"));
        assert!(!should_run(&macro_definition, "Discord.exe"));

        macro_definition.enabled = false;
        assert!(!should_run(&macro_definition, "Game.exe"));
    }

    #[test]
    fn nesting_is_capped_when_the_macro_is_checked() {
        let mut step = Step::Repeat {
            times: 2,
            steps: vec![Step::Delay { ms: 1 }],
        };
        for _ in 0..MAX_DEPTH + 1 {
            step = Step::Repeat {
                times: 2,
                steps: vec![step],
            };
        }

        let mut macro_definition = sample("m1", "Deep");
        macro_definition.steps = vec![step];
        let error = validate(&macro_definition).expect_err("too deep should be refused");
        assert!(error.contains("nested deeper"), "{error}");
    }

    #[test]
    fn silly_blocks_are_refused_with_the_reason() {
        let cases = [
            Step::Key {
                scan: 0,
                mode: PressMode::Tap,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Text {
                text: String::new(),
                per_char_ms: 0,
            },
            Step::Click {
                button: MouseButton::Left,
                mode: PressMode::Tap,
                count: 0,
                hold_ms: 0,
                x: 0,
                y: 0,
                relative: true,
                duration_ms: 0,
            },
            // A key has nowhere to go, so a drag on one is a mistake.
            Step::Key {
                scan: 0x11,
                mode: PressMode::Drag,
                hold_ms: 0,
                uppercase: false,
            },
            Step::Scroll { amount: 0 },
            Step::Repeat {
                times: 0,
                steps: vec![],
            },
        ];

        for step in cases {
            let mut macro_definition = sample("m", "Test");
            macro_definition.steps = vec![step.clone()];
            assert!(
                validate(&macro_definition).is_err(),
                "{} should be refused",
                step.summary()
            );
        }

        assert!(validate(&sample("m", "  ")).is_err());
    }

    #[test]
    fn the_action_budget_stops_a_runaway_nest() {
        let recorder = Recorder::default();
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));
        // 2000 × 2000 × 10 = 40 million actions, far past the budget.
        let report = play(
            &[Step::Repeat {
                times: MAX_TIMES,
                steps: vec![Step::Repeat {
                    times: MAX_TIMES,
                    steps: vec![Step::Delay { ms: 0 }],
                }],
            }],
            &mut runner,
            stop,
        );

        assert!(
            report.stop_reason.contains("actions"),
            "the budget should end it, said “{}”",
            report.stop_reason
        );
    }

    /// The click-and-drag in one block: press, travel, rest where it landed for
    /// the block's own time, and let go.
    #[test]
    fn a_drag_presses_travels_rests_and_releases() {
        let recorder = Recorder::default();
        let actions = Rc::clone(&recorder.actions);
        let clock = Rc::clone(&recorder.clock);
        let mut runner = recorder;
        let stop = Arc::new(AtomicBool::new(false));
        let report = play(
            &[
                Step::Click {
                    button: MouseButton::Left,
                    mode: PressMode::Drag,
                    count: 1,
                    hold_ms: 40,
                    x: 300,
                    y: 0,
                    relative: true,
                    duration_ms: 0,
                },
                Step::Click {
                    button: MouseButton::Right,
                    mode: PressMode::Drag,
                    count: 1,
                    hold_ms: 0,
                    x: 900,
                    y: 500,
                    relative: false,
                    duration_ms: 250,
                },
            ],
            &mut runner,
            stop,
        );

        assert_eq!(
            actions.borrow().clone(),
            vec![
                "down Left",
                "by 300,0",
                "up Left",
                "down Right",
                "to 900,500 in 250 ms",
                "up Right",
            ]
        );
        assert_eq!(*clock.borrow(), 40, "the drag rests before it lets go");
        assert_eq!(report.actions, 4, "two presses and two travels");
    }

    /// A tap is allowed to ask for no time at all, and that is exactly the click
    /// a frame-based program can miss: the release must never come in the same
    /// instant as the press.
    #[test]
    fn a_tap_never_lets_go_in_the_same_instant() {
        assert_eq!(tap_hold(0), MIN_TAP_MS);
        assert_eq!(tap_hold(1), MIN_TAP_MS);
        assert_eq!(tap_hold(MIN_TAP_MS), MIN_TAP_MS);
        assert_eq!(tap_hold(500), 500, "a longer press is left alone");
    }

    /// A block has to read back in the names the window writes it with. This is
    /// the bug that made the block fields look broken: the window is camelCase,
    /// the JSON was snake_case, and every number the person typed was dropped on
    /// the way in.
    #[test]
    fn a_block_reads_the_names_the_window_writes() {
        let step: Step = serde_json::from_str(
            r#"{"kind":"click","button":"right","mode":"drag","count":1,"holdMs":40,"x":900,"y":500,"relative":false,"durationMs":250}"#,
        )
        .expect("the window's own field names");

        assert_eq!(
            step.summary(),
            "drag right to 900,500 over 250 ms, resting 40 ms",
            "the hold and the travel time have to come back, not their defaults"
        );
    }

    /// The same in the other direction, so a block saved today is read back
    /// unchanged tomorrow.
    #[test]
    fn a_block_survives_a_round_trip_through_json() {
        let step = Step::Click {
            button: MouseButton::Right,
            mode: PressMode::Drag,
            count: 1,
            hold_ms: 40,
            x: 900,
            y: 500,
            relative: false,
            duration_ms: 250,
        };
        let json = serde_json::to_string(&step).unwrap();

        for field in ["\"holdMs\":40", "\"durationMs\":250", "\"x\":900", "\"relative\":false"] {
            assert!(json.contains(field), "{json} is missing {field}");
        }
        let back: Step = serde_json::from_str(&json).unwrap();
        assert_eq!(back, step);
    }

    /// A key has nowhere to go, so a drag on a key block is refused rather than
    /// quietly typed.
    #[test]
    fn a_key_block_cannot_drag() {
        let mut macro_definition = sample("m", "Test");
        macro_definition.steps = vec![Step::Key {
            scan: 0x11,
            mode: PressMode::Drag,
            hold_ms: 0,
            uppercase: false,
        }];
        let error = validate(&macro_definition).expect_err("a key cannot be dragged");
        assert!(error.contains("drag"), "said “{error}”");
    }
}
